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
