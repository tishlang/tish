//! Promise helpers for the bytecode VM and native codegen (`Promise.resolve`, etc.).
//!
//! The global `Promise` value is an **object** with a `__call` entry so the VM can
//! invoke `Promise(executor)` like `new Promise(executor)` in JS. Static methods live
//! on the same object (`resolve`, `reject`, `all`, `race`, `any`, `allSettled`, `spawn`).
//!
//! ## Concurrency model for race / any / allSettled / spawn
//!
//! `TishPromise::block_until_settled` is a *blocking* call. To wait on "whichever of N
//! settles first" without serializing them, we spawn one OS thread per promise — each
//! calls `block_until_settled` and forwards the result (with its index) to a shared
//! `mpsc::channel`. The main thread reads from that channel:
//!   - `race`  → first message wins (fulfilled or rejected).
//!   - `any`   → first *fulfilled* message wins; collect rejections; if all reject →
//!     `AggregateError` (array of reasons).
//!   - `allSettled` → drain all N messages, sort by index, build `{status,value|reason}`.
//!
//! This requires `Value: Send`, which holds under the `send-values` feature (all handles
//! become `Arc<Mutex<…>>`). The `send-values` feature is enabled in every build that has
//! `http` (i.e. the shipped `full` binary). Without it (wasm / wasi) we fall back to a
//! sequential path — correct but not concurrent.
//!
//! `Promise.spawn(fn)` runs `fn()` on a fresh OS thread and returns a Promise. This is
//! the primitive for CPU-bound and GPU-bound work (e.g. `Promise.spawn(() => matmul(…))`
//! from `tish:mlx` or `tish:metal`). The thread is an ordinary OS thread, not a tokio
//! task, so it does not contend with the I/O runtime.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tishlang_core::{ObjectMap, TishPromise, Value, VmRef};

/// Fulfilled or rejected before anyone awaits — `block_until_settled` consumes the result once.
pub struct ImmediateSettledPromise {
    slot: Mutex<Option<Result<Value, Value>>>,
}

impl ImmediateSettledPromise {
    fn new(result: Result<Value, Value>) -> Self {
        Self {
            slot: Mutex::new(Some(result)),
        }
    }
}

impl TishPromise for ImmediateSettledPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        self.slot
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(Value::String(
                "Promise already settled or consumed".into(),
            )))
    }
    /// Always already settled — return the result immediately without blocking.
    fn try_settle(&self) -> Option<std::result::Result<Value, Value>> {
        Some(self.slot.lock().unwrap().take().unwrap_or(
            Err(Value::String("Promise already consumed".into())),
        ))
    }
}

fn fulfilled(v: Value) -> Value {
    Value::Promise(Arc::new(ImmediateSettledPromise::new(Ok(v))))
}

fn rejected(v: Value) -> Value {
    Value::Promise(Arc::new(ImmediateSettledPromise::new(Err(v))))
}

fn flatten_chain_out(v: Value) -> std::result::Result<Value, Value> {
    match v {
        Value::Promise(p) => p.block_until_settled(),
        other => Ok(other),
    }
}

/// `Promise(executor)` — executor runs synchronously; `resolve` / `reject` unblock `recv`.
struct DeferredChannelPromise {
    rx: Mutex<Option<mpsc::Receiver<Result<Value, Value>>>>,
}

impl TishPromise for DeferredChannelPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        match rx {
            Some(r) => r.recv().unwrap_or(Err(Value::String(
                "Promise executor did not call resolve or reject".into(),
            ))),
            None => Err(Value::String(
                "Promise already consumed or settled".into(),
            )),
        }
    }

    /// Non-blocking settle: if the executor has already called resolve/reject (the channel
    /// has a message waiting), return it immediately. Returns `None` if the work is still
    /// pending (channel empty). This lets `race`/`any`/`allSettled` handle already-settled
    /// `new Promise(executor)` promises in input-order without spawning threads.
    fn try_settle(&self) -> Option<std::result::Result<Value, Value>> {
        let mut lock = self.rx.lock().unwrap();
        match lock.as_ref() {
            None => Some(Err(Value::String("Promise already consumed".into()))),
            Some(rx) => match rx.try_recv() {
                Ok(r) => {
                    *lock = None; // consumed — block_until_settled would error now (correct)
                    Some(r)
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    *lock = None;
                    Some(Err(Value::String(
                        "Promise executor did not call resolve or reject".into(),
                    )))
                }
                Err(mpsc::TryRecvError::Empty) => None, // still pending
            },
        }
    }
}

/// `.then` / `.catch` chain: when awaited, settle the predecessor then optionally invoke a handler.
pub struct ThenPromise {
    pred: Arc<dyn TishPromise>,
    on_fulfilled: Option<Value>,
    on_rejected: Option<Value>,
}

impl TishPromise for ThenPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        match self.pred.block_until_settled() {
            Ok(v) => {
                if let Some(Value::Function(f)) = &self.on_fulfilled {
                    flatten_chain_out(f.call(&[v]))
                } else {
                    Ok(v)
                }
            }
            Err(e) => {
                if let Some(Value::Function(f)) = &self.on_rejected {
                    flatten_chain_out(f.call(&[e]))
                } else {
                    Err(e)
                }
            }
        }
    }
}

/// `Promise.resolve(value)` — adopt promises, otherwise wrap in a fulfilled promise.
pub fn promise_resolve(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Promise(p)) => Value::Promise(Arc::clone(p)),
        Some(v) => fulfilled(v.clone()),
        None => fulfilled(Value::Null),
    }
}

/// `Promise.reject(reason)` — always a rejected promise.
pub fn promise_reject(args: &[Value]) -> Value {
    rejected(
        args.first()
            .cloned()
            .unwrap_or(Value::Null),
    )
}

/// `Promise.all(iterable)` — block on each promise in order; non-promises pass through.
pub fn promise_all(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Array(arr)) => {
            let mut out: Vec<Value> = Vec::new();
            for v in arr.borrow().iter() {
                let item = if let Value::Promise(p) = v {
                    match p.block_until_settled() {
                        Ok(x) => x,
                        Err(rej) => return rejected(rej),
                    }
                } else {
                    v.clone()
                };
                out.push(item);
            }
            fulfilled(Value::Array(VmRef::new(out)))
        }
        Some(v) => fulfilled(v.clone()),
        None => fulfilled(Value::Null),
    }
}

// ---------------------------------------------------------------------------
// Concurrent combinators (race / any / allSettled) + Promise.spawn
//
// All three combinators need to wait on multiple promises concurrently. We
// spawn one OS thread per promise; each thread calls block_until_settled and
// sends (index, Result) to a shared mpsc channel on the calling thread.
// ---------------------------------------------------------------------------

/// Extract the array of items from `Promise.all/race/any/allSettled(array)`.
fn combinator_items(args: &[Value]) -> Option<Vec<Value>> {
    match args.first() {
        Some(Value::Array(arr)) => Some(arr.borrow().clone()),
        _ => None,
    }
}

/// Concurrent settlement channel for `race`/`any`/`allSettled`.
///
/// **Two-phase:** already-settled promises (`try_settle` returns `Some`) are handled
/// inline in input-order before any threads are spawned. This gives deterministic
/// JS-compatible ordering for already-settled inputs (e.g. `Promise.any([rej, ok, ok])`
/// reliably returns the first fulfilled, not a random thread-schedule winner). Only
/// genuinely-pending promises (e.g. from `Promise.spawn`) go to background threads,
/// which is where concurrency matters.
///
/// Returns the receiving end of the channel plus the count of items it will send.
#[cfg(feature = "send-values")]
#[allow(clippy::type_complexity)]
fn race_channel(
    items: Vec<Value>,
) -> (mpsc::Receiver<(usize, std::result::Result<Value, Value>)>, usize) {
    let (tx, rx) = mpsc::channel::<(usize, std::result::Result<Value, Value>)>();
    let mut count = 0usize;
    for (i, v) in items.into_iter().enumerate() {
        count += 1;
        match v {
            Value::Promise(ref p) => {
                // Phase 1: try non-blocking settle (ImmediateSettledPromise, ThenPromise
                // over immediate, etc.). These never need a thread; handle in order.
                if let Some(r) = p.try_settle() {
                    let _ = tx.send((i, r));
                } else {
                    // Phase 2: genuinely pending — spawn a thread.
                    let p = Arc::clone(p);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let r = p.block_until_settled();
                        let _ = tx.send((i, r));
                    });
                }
            }
            other => {
                let _ = tx.send((i, Ok(other)));
            }
        }
    }
    drop(tx); // closes the channel when all senders finish
    (rx, count)
}

/// `Promise.race(iterable)` — first to settle (fulfilled or rejected) wins.
/// Fixed: genuinely concurrent — the old impl only ever blocked on element 0.
pub fn promise_race(args: &[Value]) -> Value {
    let items = match combinator_items(args) {
        Some(v) => v,
        None => return fulfilled(args.first().cloned().unwrap_or(Value::Null)),
    };
    if items.is_empty() {
        return rejected(Value::String("Promise.race: empty iterable".into()));
    }
    #[cfg(feature = "send-values")]
    {
        let (rx, _) = race_channel(items);
        match rx.recv() {
            Ok((_, Ok(v)))  => fulfilled(v),
            Ok((_, Err(e))) => rejected(e),
            Err(_)          => rejected(Value::String("Promise.race: all promises dropped".into())),
        }
    }
    #[cfg(not(feature = "send-values"))]
    {
        // Sequential fallback (no threads): first item wins, whether promise or value.
        for item in items {
            return match item {
                Value::Promise(p) => match p.block_until_settled() {
                    Ok(v)  => fulfilled(v),
                    Err(e) => rejected(e),
                },
                other => fulfilled(other),
            };
        }
        rejected(Value::String("Promise.race: empty iterable".into()))
    }
}

/// `Promise.any(iterable)` — resolves with the **first fulfilled** value.
/// Rejects with an array of all rejection reasons only if every promise rejects
/// (matching the JS `AggregateError.errors` convention — we return the array
/// directly, not wrapped, to keep things simple without a full AggregateError class).
pub fn promise_any(args: &[Value]) -> Value {
    let items = match combinator_items(args) {
        Some(v) => v,
        None => return fulfilled(args.first().cloned().unwrap_or(Value::Null)),
    };
    if items.is_empty() {
        return rejected(Value::Array(VmRef::new(vec![])));
    }
    let n = items.len();
    #[cfg(feature = "send-values")]
    {
        let (rx, sent) = race_channel(items);
        let mut errors = vec![Value::Null; n];
        let mut reject_count = 0usize;
        // Drain the channel: the first fulfilled result wins immediately; collect
        // all rejections in case every promise rejects.
        let mut received = 0usize;
        while received < sent {
            match rx.recv() {
                Ok((_, Ok(v))) => return fulfilled(v), // first fulfillment wins
                Ok((i, Err(e))) => {
                    errors[i] = e;
                    reject_count += 1;
                    received += 1;
                    if reject_count == sent {
                        return rejected(Value::Array(VmRef::new(errors)));
                    }
                }
                Err(_) => break,
            }
        }
        rejected(Value::Array(VmRef::new(errors)))
    }
    #[cfg(not(feature = "send-values"))]
    {
        // Sequential: return first fulfilled, or array of all rejections.
        let mut errors = Vec::with_capacity(n);
        for item in items {
            match item {
                Value::Promise(p) => match p.block_until_settled() {
                    Ok(v)  => return fulfilled(v),
                    Err(e) => errors.push(e),
                },
                other => return fulfilled(other),
            }
        }
        rejected(Value::Array(VmRef::new(errors)))
    }
}

/// `Promise.allSettled(iterable)` — always fulfills with an array of outcome objects.
/// Each entry is `{status:"fulfilled",value:v}` or `{status:"rejected",reason:e}`.
pub fn promise_all_settled(args: &[Value]) -> Value {
    let items = match combinator_items(args) {
        Some(v) => v,
        None => return fulfilled(Value::Array(VmRef::new(vec![]))),
    };
    let n = items.len();
    if n == 0 {
        return fulfilled(Value::Array(VmRef::new(vec![])));
    }

    fn make_settled(r: std::result::Result<Value, Value>) -> Value {
        let mut obj = ObjectMap::default();
        match r {
            Ok(v) => {
                obj.insert(Arc::from("status"), Value::String("fulfilled".into()));
                obj.insert(Arc::from("value"), v);
            }
            Err(e) => {
                obj.insert(Arc::from("status"), Value::String("rejected".into()));
                obj.insert(Arc::from("reason"), e);
            }
        }
        Value::object(obj)
    }

    #[cfg(feature = "send-values")]
    {
        let (rx, _) = race_channel(items);
        let mut results = vec![None::<std::result::Result<Value, Value>>; n];
        while let Ok((i, r)) = rx.recv() {
            results[i] = Some(r);
        }
        let out: Vec<Value> = results
            .into_iter()
            .map(|r| make_settled(r.unwrap_or(Err(Value::String("Promise dropped".into())))))
            .collect();
        fulfilled(Value::Array(VmRef::new(out)))
    }
    #[cfg(not(feature = "send-values"))]
    {
        let out: Vec<Value> = items.into_iter().map(|item| {
            let r = match item {
                Value::Promise(p) => p.block_until_settled(),
                other => Ok(other),
            };
            make_settled(r)
        }).collect();
        fulfilled(Value::Array(VmRef::new(out)))
    }
}

/// `Promise.spawn(fn)` — run `fn()` on a background OS thread and return a Promise
/// that resolves with the function's return value. This is the key primitive for
/// CPU-bound and GPU-bound work:
///
/// ```tish
/// import { matmul } from 'tish:mlx'
/// let result = await Promise.any([
///     Promise.spawn(() => matmul(a, b, N)),   // MLX GPU path
///     Promise.spawn(() => fallback(a, b, N)), // CPU fallback
/// ])
/// ```
///
/// Under `send-values` (the shipped `full` build), the function runs on a real OS
/// thread; other threads can proceed concurrently. Without `send-values` (wasm/wasi),
/// the function runs synchronously and the result is wrapped in an immediate promise.
pub fn promise_spawn(args: &[Value]) -> Value {
    let f = match args.first() {
        Some(Value::Function(f)) => Arc::clone(f),
        _ => return rejected(Value::String("Promise.spawn: expected a function argument".into())),
    };
    #[cfg(feature = "send-values")]
    {
        let (tx, rx) = mpsc::channel::<std::result::Result<Value, Value>>();
        std::thread::spawn(move || {
            // Wrap in catch_unwind so a panicking GPU/CPU kernel rejects the promise
            // rather than aborting the whole process.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.call(&[])));
            let _ = tx.send(match result {
                Ok(v)  => Ok(v),
                Err(_) => Err(Value::String("Promise.spawn: task panicked".into())),
            });
        });
        Value::Promise(Arc::new(DeferredChannelPromise {
            rx: Mutex::new(Some(rx)),
        }))
    }
    #[cfg(not(feature = "send-values"))]
    {
        // No threads available (wasm/wasi): run synchronously, wrap result.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.call(&[])));
        match result {
            Ok(v)  => fulfilled(v),
            Err(_) => rejected(Value::String("Promise.spawn: task panicked".into())),
        }
    }
}

/// Build the global `Promise` object: `__call` (constructor) + static methods.
pub fn promise_object() -> Value {
    let mut map: ObjectMap = ObjectMap::default();

    let ctor = Value::native(|args: &[Value]| match args.first() {
        Some(Value::Function(f)) => {
            let (tx, rx) = mpsc::channel();
            let tx_cell = Arc::new(Mutex::new(Some(tx)));
            let resolve = Value::native({
                let tx_cell = Arc::clone(&tx_cell);
                move |a: &[Value]| {
                    if let Some(t) = tx_cell.lock().unwrap().take() {
                        let _ = t.send(Ok(
                            a.first().cloned().unwrap_or(Value::Null),
                        ));
                    }
                    Value::Null
                }
            });
            let reject = Value::native({
                let tx_cell = Arc::clone(&tx_cell);
                move |a: &[Value]| {
                    if let Some(t) = tx_cell.lock().unwrap().take() {
                        let _ = t.send(Err(
                            a.first().cloned().unwrap_or(Value::Null),
                        ));
                    }
                    Value::Null
                }
            });
            let _ = f.call(&[resolve, reject]);
            Value::Promise(Arc::new(DeferredChannelPromise {
                rx: Mutex::new(Some(rx)),
            }))
        }
        _ => Value::Null,
    });

    map.insert(Arc::from("__call"), ctor);
    map.insert(
        Arc::from("resolve"),
        Value::native(|args: &[Value]| promise_resolve(args)),
    );
    map.insert(
        Arc::from("reject"),
        Value::native(|args: &[Value]| promise_reject(args)),
    );
    map.insert(
        Arc::from("all"),
        Value::native(|args: &[Value]| promise_all(args)),
    );
    map.insert(
        Arc::from("race"),
        Value::native(|args: &[Value]| promise_race(args)),
    );
    map.insert(
        Arc::from("any"),
        Value::native(|args: &[Value]| promise_any(args)),
    );
    map.insert(
        Arc::from("allSettled"),
        Value::native(|args: &[Value]| promise_all_settled(args)),
    );
    map.insert(
        Arc::from("spawn"),
        Value::native(|args: &[Value]| promise_spawn(args)),
    );
    Value::object(map)
}

/// `.then(onFulfilled, onRejected)` for a `Value::Promise` instance (VM `GetMember`).
pub fn promise_instance_then(p: &Arc<dyn TishPromise>, args: &[Value]) -> Value {
    Value::Promise(Arc::new(ThenPromise {
        pred: Arc::clone(p),
        on_fulfilled: args.first().cloned(),
        on_rejected: args.get(1).cloned(),
    }))
}

/// `.catch(onRejected)` for a `Value::Promise` instance.
pub fn promise_instance_catch(p: &Arc<dyn TishPromise>, args: &[Value]) -> Value {
    Value::Promise(Arc::new(ThenPromise {
        pred: Arc::clone(p),
        on_fulfilled: None,
        on_rejected: args.first().cloned(),
    }))
}

/// Unwrap a settled [`Value::Promise`], or pass non-promise values through (VM `AwaitPromise` /
/// `tish:http.await`). Fetch promises still require the `http` feature.
pub fn await_promise(v: Value) -> Value {
    if let Value::Promise(p) = v {
        match p.block_until_settled() {
            Ok(val) => val,
            Err(rejection) => rejection,
        }
    } else {
        v
    }
}

/// Like [`await_promise`], but a REJECTED promise surfaces as a catchable throw rather than
/// silently yielding the rejection value. `await Promise.reject(x)` must throw `x` (so a
/// surrounding `try/catch` fires) — matching interp/vm/cranelift/wasi. The codegen emits this
/// variant (with `?`) wherever an error channel exists (inside a `try` body, or top-level `run()`),
/// and falls back to [`await_promise`] only where there is no channel (a nested value-fn with no
/// enclosing try), mirroring how `throw` is lowered.
pub fn await_promise_throw(v: Value) -> Result<Value, Box<dyn std::error::Error>> {
    if let Value::Promise(p) = v {
        match p.block_until_settled() {
            Ok(val) => Ok(val),
            Err(rejection) => {
                Err(Box::new(crate::TishError::Throw(rejection)) as Box<dyn std::error::Error>)
            }
        }
    } else {
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Tests — issue #702: Promise.race / Promise.any must not park one OS thread
// per pending input, and combinator inputs must remain awaitable afterwards.
// Everything goes through the public surface (`promise_object`, `promise_race`,
// `promise_any`, `promise_all_settled`) so the tests are independent of the
// promise internals they guard.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_core::object_get;

    /// Build `new Promise(executor)` through the public `Promise.__call` ctor, with an
    /// executor that stashes `resolve` for later (or never) — the genuinely-pending
    /// shape from issue #702 (event-registry / never-settling loser).
    /// Returns `(promise, resolve)`.
    fn deferred() -> (Value, Value) {
        let stash: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let executor = Value::native({
            let stash = Arc::clone(&stash);
            move |args: &[Value]| {
                stash
                    .lock()
                    .unwrap()
                    .push(args.first().cloned().unwrap_or(Value::Null));
                Value::Null
            }
        });
        let promise_global = promise_object();
        let ctor = object_get(&promise_global, &Value::String("__call".into()))
            .expect("Promise global has __call");
        let p = match &ctor {
            Value::Function(f) => f.call(std::slice::from_ref(&executor)),
            _ => panic!("Promise.__call is not callable"),
        };
        let resolve = stash
            .lock()
            .unwrap()
            .first()
            .cloned()
            .expect("executor ran synchronously");
        (p, resolve)
    }

    fn call_value(f: &Value, args: &[Value]) {
        match f {
            Value::Function(f) => {
                f.call(args);
            }
            _ => panic!("expected a callable Value"),
        }
    }

    /// Await a `Value::Promise` (panics on non-promise).
    fn settle(v: &Value) -> std::result::Result<Value, Value> {
        match v {
            Value::Promise(p) => p.block_until_settled(),
            other => panic!("expected a promise, got {}", other.type_name()),
        }
    }

    fn expect_num(r: std::result::Result<Value, Value>) -> f64 {
        match r {
            Ok(Value::Number(n)) => n,
            other => panic!("expected fulfilled number, got {other:?}"),
        }
    }

    /// Live OS threads in this process (macOS via `ps -M`, Linux via /proc).
    fn os_thread_count() -> usize {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_dir("/proc/self/task").unwrap().count()
        }
        #[cfg(target_os = "macos")]
        {
            let pid = std::process::id().to_string();
            let out = std::process::Command::new("ps")
                .args(["-M", &pid])
                .output()
                .expect("run ps -M");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .count()
                .saturating_sub(1) // header line
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            0
        }
    }

    /// Issue #702 regression: N races against never-settling losers must NOT grow the
    /// process thread count by N. Before the subscription redesign each pending input
    /// parked one uncancellable OS thread in `block_until_settled` forever.
    #[test]
    fn race_with_never_settling_loser_does_not_park_threads() {
        if os_thread_count() == 0 {
            return; // platform without a thread-count probe
        }
        let n = 64usize;
        // Keep the losers (and their stashed resolvers) alive: the leak is conditional
        // on the loser never settling, which requires its resolve to stay reachable.
        let mut keep: Vec<(Value, Value)> = Vec::new();

        // Warm up lazy machinery so it doesn't count against the delta.
        let (wp, wr) = deferred();
        let out = promise_race(&[Value::array(vec![promise_resolve(&[Value::Number(-1.0)]), wp.clone()])]);
        assert_eq!(expect_num(settle(&out)), -1.0);
        keep.push((wp, wr));
        std::thread::sleep(std::time::Duration::from_millis(100));

        let baseline = os_thread_count();
        for i in 0..n {
            let (p, r) = deferred();
            let out = promise_race(&[Value::array(vec![
                promise_resolve(&[Value::Number(i as f64)]),
                p.clone(),
            ])]);
            assert_eq!(expect_num(settle(&out)), i as f64);
            keep.push((p, r));
        }
        // Give any transient threads a moment to exit before measuring.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after = os_thread_count();
        let grown = after.saturating_sub(baseline);
        assert!(
            grown < n / 4,
            "Promise.race parked ~1 thread per never-settling loser: baseline {} -> {} (+{}) after {} races",
            baseline,
            after,
            grown,
            n
        );
    }

    /// Same regression for `Promise.any`.
    #[test]
    fn any_with_never_settling_loser_does_not_park_threads() {
        if os_thread_count() == 0 {
            return;
        }
        let n = 64usize;
        let mut keep: Vec<(Value, Value)> = Vec::new();

        let (wp, wr) = deferred();
        let out = promise_any(&[Value::array(vec![wp.clone(), promise_resolve(&[Value::Number(-1.0)])])]);
        assert_eq!(expect_num(settle(&out)), -1.0);
        keep.push((wp, wr));
        std::thread::sleep(std::time::Duration::from_millis(100));

        let baseline = os_thread_count();
        for i in 0..n {
            let (p, r) = deferred();
            let out = promise_any(&[Value::array(vec![
                p.clone(),
                promise_resolve(&[Value::Number(i as f64)]),
            ])]);
            assert_eq!(expect_num(settle(&out)), i as f64);
            keep.push((p, r));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after = os_thread_count();
        let grown = after.saturating_sub(baseline);
        assert!(
            grown < n / 4,
            "Promise.any parked ~1 thread per never-settling loser: baseline {} -> {} (+{}) after {} calls",
            baseline,
            after,
            grown,
            n
        );
    }

    /// Semantics guard: race settles with the first settled input.
    #[test]
    fn race_first_settled_input_wins() {
        let (pending, _r) = deferred();
        let out = promise_race(&[Value::array(vec![
            promise_resolve(&[Value::Number(1.0)]),
            pending,
        ])]);
        assert_eq!(expect_num(settle(&out)), 1.0);
    }

    /// Semantics guard: a genuinely-pending winner settled from another thread wins the race.
    #[test]
    fn race_pending_winner_settled_from_another_thread() {
        let (winner, resolve) = deferred();
        let (never, _keep) = deferred();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            call_value(&resolve, &[Value::Number(7.0)]);
        });
        let out = promise_race(&[Value::array(vec![winner, never])]);
        assert_eq!(expect_num(settle(&out)), 7.0);
        t.join().unwrap();
    }

    /// Semantics guard: any returns the first FULFILLMENT even with a pending loser present.
    #[test]
    fn any_first_fulfillment_wins_with_pending_loser() {
        let (never, _keep) = deferred();
        let out = promise_any(&[Value::array(vec![
            never,
            promise_reject(&[Value::String("nope".into())]),
            promise_resolve(&[Value::Number(3.0)]),
        ])]);
        assert_eq!(expect_num(settle(&out)), 3.0);
    }

    /// Semantics guard: any rejects with all reasons, in input order, when every input rejects.
    #[test]
    fn any_all_rejected_yields_reasons_in_order() {
        let out = promise_any(&[Value::array(vec![
            promise_reject(&[Value::String("a".into())]),
            promise_reject(&[Value::String("b".into())]),
        ])]);
        match settle(&out) {
            Err(Value::Array(arr)) => {
                let arr = arr.borrow();
                assert_eq!(arr.len(), 2);
                assert!(matches!(&arr[0], Value::String(s) if s.as_str() == "a"));
                assert!(matches!(&arr[1], Value::String(s) if s.as_str() == "b"));
            }
            other => panic!("expected rejection with reasons array, got {other:?}"),
        }
    }

    /// Semantics guard: allSettled drains pending inputs settled from another thread.
    #[test]
    fn all_settled_waits_for_pending_inputs() {
        let (p1, r1) = deferred();
        let (p2, r2) = deferred();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            call_value(&r1, &[Value::Number(1.0)]);
            std::thread::sleep(std::time::Duration::from_millis(30));
            call_value(&r2, &[Value::Number(2.0)]);
        });
        let out = promise_all_settled(&[Value::array(vec![p1, p2])]);
        let settled = settle(&out).expect("allSettled fulfills");
        let Value::Array(arr) = settled else {
            panic!("allSettled must fulfill with an array")
        };
        let arr = arr.borrow();
        assert_eq!(arr.len(), 2);
        for (i, expected) in [(0usize, 1.0f64), (1, 2.0)] {
            let v = object_get(&arr[i], &Value::String("value".into())).expect("value key");
            assert!(matches!(v, Value::Number(n) if n == expected), "slot {i}");
        }
        t.join().unwrap();
    }

    /// Issue #702 adjacent defect: `race_channel` must not CONSUME its inputs. A promise
    /// that loses a race stays awaitable — settling it later and awaiting must yield the
    /// value, not "Promise already settled or consumed".
    #[test]
    fn race_loser_remains_awaitable() {
        let (loser, resolve) = deferred();
        let out = promise_race(&[Value::array(vec![
            promise_resolve(&[Value::Number(1.0)]),
            loser.clone(),
        ])]);
        assert_eq!(expect_num(settle(&out)), 1.0);

        call_value(&resolve, &[Value::Number(9.0)]);
        // Give the (pre-fix) parked waiter thread time to consume the settlement, so the
        // pre-fix failure is deterministic rather than a race with the test body.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            expect_num(settle(&loser)),
            9.0,
            "a promise that lost a race must remain awaitable"
        );
    }

    /// Issue #702 adjacent defect (peek-not-take): an already-settled promise can be
    /// awaited more than once, like in JS — including after losing a race.
    #[test]
    fn settled_promise_is_reawaitable() {
        let p = promise_resolve(&[Value::Number(5.0)]);
        assert_eq!(expect_num(settle(&p)), 5.0);
        assert_eq!(
            expect_num(settle(&p)),
            5.0,
            "second await of a settled promise must yield the value again"
        );

        // And after being consumed by a race in phase 1 (try_settle path):
        let q = promise_resolve(&[Value::Number(6.0)]);
        let out = promise_race(&[Value::array(vec![q.clone()])]);
        assert_eq!(expect_num(settle(&out)), 6.0);
        assert_eq!(
            expect_num(settle(&q)),
            6.0,
            "a settled promise that entered a race must remain awaitable"
        );
    }
}
