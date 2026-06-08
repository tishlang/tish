//! Regression: concurrent HTTP handlers that mutate shared module-level state must not deadlock.
//!
//! ## What this guards
//!
//! Under `send-values` (forced on by the `http` feature), `serve(port, handler)` runs the handler
//! closure — a `NativeFn` (`Arc<dyn Callable>`, `Send + Sync`) — **directly on each accept thread**
//! (`tish_runtime::http::worker_loop_direct`). So N concurrent requests execute the SAME handler in
//! parallel, all sharing the captured module scope through `Arc<Mutex>` (`VmRef`). A handler that
//! mutates a module-level `let` (a request counter / cache / rate-limiter) therefore has many threads
//! reading and writing the same scope cell at once.
//!
//! This test drives that exact path without a network: it pulls the handler `Value::Function` out of
//! a freshly-run program and invokes it from many OS threads while they all read-modify-write a shared
//! module-level `let`. It exists to catch a regression where the VM's variable-write path holds a
//! scope guard across a re-acquisition of the same lock (which would deadlock concurrent writers and
//! hang `serve`). The watchdog turns such a hang into a fast, explicit test failure instead of a
//! stuck CI job.
//!
//! Note on coverage: macOS `SO_REUSEPORT` funnels HTTP accepts to a single worker thread, so a
//! network-level test can't actually run two handlers concurrently on macOS. Calling the handler
//! closure directly does — and OS thread scheduling + mutex semantics are platform-independent, so
//! this reproduces the Linux multi-worker dispatch contention the bug was reported against.
#![cfg(feature = "send-values")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tishlang_bytecode::compile;
use tishlang_core::{NativeFn, Value};
use tishlang_vm::Vm;

// `Value::Function` holds a `NativeFn` (= `Arc<dyn Callable>`, `Callable: Send + Sync` under
// send-values); invoke a handler via the trait-object method `.call(args)`.
type Handler = NativeFn;

/// Compile + run `src`, then pull a function it stored in a global back out.
fn export(vm: &Vm, name: &str) -> Handler {
    match vm.get_global(name).unwrap_or_else(|| panic!("global `{name}` not found")) {
        Value::Function(f) => f,
        other => panic!("global `{name}` is not a function: {other:?}"),
    }
}

fn read_num(obj: &Value, field: &str) -> f64 {
    match obj {
        Value::Object(o) => match o.borrow().strings.get(field) {
            Some(Value::Number(n)) => *n,
            other => panic!("stats.{field} is not a number: {other:?}"),
        },
        other => panic!("stats is not an object: {other:?}"),
    }
}

#[test]
fn concurrent_handlers_mutating_shared_module_state_do_not_deadlock() {
    // `handler`/`stats` are bare top-level assignments (undeclared names) -> stored in globals, so
    // the test can pull them out. Both functions close over the same module-level `let`s, exactly as
    // a real `serve` handler closes over module state. `served` is monotonic (only incremented), so
    // it gives a deterministic plausibility bound even though the read-modify-write is racy.
    let src = r#"
let active = 0
let maxActive = 0
let served = 0
fn handleRequest(req) {
    active = active + 1
    served = served + 1
    if (active > maxActive) { maxActive = active }
    let i = 0
    while (i < 2000) { i = i + 1 }   // brief CPU hold so handlers overlap
    active = active - 1
    return { status: 200, body: "ok" }
}
fn getStats() {
    return { active: active, maxActive: maxActive, served: served }
}
handler = handleRequest
stats = getStats
"#;
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    let mut vm = Vm::new();
    vm.run(&chunk).expect("run top-level");
    let handler = export(&vm, "handler");
    let stats = export(&vm, "stats");

    const THREADS: usize = 12;
    const ITERS: usize = 100;
    let total = THREADS * ITERS;

    let done = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();
    let mut handles = Vec::with_capacity(THREADS);
    for t in 0..THREADS {
        let h = handler.clone();
        let done = Arc::clone(&done);
        handles.push(std::thread::spawn(move || {
            for i in 0..ITERS {
                let resp = h.call(&[Value::Number((t * ITERS + i) as f64)]);
                assert!(matches!(resp, Value::Object(_)), "handler must return a response object");
                done.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // Watchdog: if concurrent writers deadlocked on the scope mutex, `done` stops advancing.
    // Fail fast (and loudly) rather than hang the test runner.
    let mut last = 0usize;
    let mut last_change = Instant::now();
    while done.load(Ordering::Relaxed) < total {
        let cur = done.load(Ordering::Relaxed);
        if cur != last {
            last = cur;
            last_change = Instant::now();
        }
        assert!(
            last_change.elapsed() < Duration::from_secs(15),
            "DEADLOCK regression: concurrent handlers stalled at {cur}/{total} (no progress for 15s)"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for h in handles {
        h.join().expect("a handler thread panicked");
    }

    // All calls returned without hanging. Read the shared counters back.
    let s = stats.call(&[]);
    let served = read_num(&s, "served");
    let max_active = read_num(&s, "maxActive");
    let active = read_num(&s, "active");
    eprintln!(
        "completed {total} concurrent calls / {THREADS} threads in {:?}; served={served}, maxActive={max_active}, active(final)={active}",
        start.elapsed()
    );

    // `served` is monotonic, so it is deterministically in (0, total] regardless of lost updates.
    assert!(served > 0.0 && served <= total as f64, "served={served} out of plausible range (0, {total}]");
    // `maxActive` >= 2 proves at least two handlers were genuinely in-flight simultaneously, i.e. we
    // actually exercised concurrent shared-state mutation (not an accidentally-serialized run).
    assert!(max_active >= 2.0, "handlers never overlapped (maxActive={max_active}); test did not exercise concurrency");
}
