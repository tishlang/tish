//! Several mutually untrusted programs in one process, each in its own `Vm` (an embedder hosting
//! plugins). Two process-wide mechanisms leak between such VMs, and these tests pin the controls:
//!
//! - The numeric JIT's caches and callee registry are process-global, and every JIT-on `run` resets
//!   the registry, which races with VMs running on other threads. `Vm::set_jit_enabled(false)` keeps
//!   a VM out of the JIT entirely (`jit_off_leaves_jit_state.rs` checks it never touches the caches).
//! - `set_execution_deadline` is process-wide. `set_thread_execution_deadline` budgets only the
//!   calling thread. The interpreter polls on loop back-edges and on calls; JIT-compiled loops do
//!   not poll, so a budgeted VM runs JIT-off.
//!
//! Programs hand values out through bare top-level assignments (undeclared names are globals).
#![cfg(feature = "send-values")]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use tishlang_bytecode::compile;
use tishlang_core::{set_thread_execution_deadline, Value, DEADLINE_ERROR};
use tishlang_vm::Vm;

fn run(src: &str, jit: bool) -> (Vm, Result<Value, String>) {
    let chunk = compile(&tishlang_parser::parse(src).expect("parse")).expect("compile");
    let mut vm = Vm::new();
    vm.set_jit_enabled(jit);
    let r = vm.run(&chunk);
    (vm, r)
}

fn global_num(vm: &Vm, name: &str) -> f64 {
    match vm.get_global(name) {
        Some(Value::Number(n)) => n,
        other => panic!("global `{name}` is not a number: {other:?}"),
    }
}

fn global_fn(vm: &Vm, name: &str) -> tishlang_core::NativeFn {
    match vm.get_global(name) {
        Some(Value::Function(f)) => f,
        other => panic!("global `{name}` is not a function: {other:?}"),
    }
}

/// Run `f` on a fresh thread and fail (instead of hanging the suite) if it takes longer than `limit`.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).expect("did not finish in time")
}

const SPIN: &str = "fn spin(n) { let s = 0\nlet i = 0\nwhile (i < n) { s = s + i\ni = i + 1 }\nreturn s }\n";

#[test]
fn jit_off_gives_the_same_results() {
    let src = "fn fib(n) { if (n < 2) { return n }\nreturn fib(n - 1) + fib(n - 2) }\n\
               fn sum(n) { let s = 0\nfor (let i = 0; i < n; i = i + 1) { s = s + i * 0.5 }\nreturn s }\n\
               out = fib(20) + sum(50000)";
    let (on, r) = run(src, true);
    r.expect("jit on");
    let (off, r) = run(src, false);
    r.expect("jit off");
    assert_eq!(global_num(&on, "out"), 6765.0 + 624987500.0);
    assert_eq!(global_num(&on, "out"), global_num(&off, "out"));
}

/// A hot numeric loop inside a function: with the JIT on it would be compiled and never poll.
#[test]
fn thread_deadline_stops_a_jit_off_hot_loop() {
    let (r, elapsed) = within(Duration::from_secs(10), || {
        let t0 = Instant::now();
        set_thread_execution_deadline(Some(100));
        let r = run(&format!("{SPIN}out = spin(1000000000000)"), false).1;
        set_thread_execution_deadline(None);
        (r, t0.elapsed())
    });
    let err = r.expect_err("the deadline must abort the loop");
    assert!(err.contains(DEADLINE_ERROR), "{err}");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

/// No loop at all: naive recursion stays far below the call-depth limit but never hits a back-edge.
#[test]
fn thread_deadline_stops_loop_free_recursion() {
    let (r, elapsed) = within(Duration::from_secs(10), || {
        let t0 = Instant::now();
        set_thread_execution_deadline(Some(100));
        let r = run("fn fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) }\nout = fib(45)", false).1;
        set_thread_execution_deadline(None);
        (r, t0.elapsed())
    });
    let err = r.expect_err("the deadline must abort the recursion");
    assert!(err.contains(DEADLINE_ERROR), "{err}");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

#[test]
fn thread_deadline_is_per_thread() {
    set_thread_execution_deadline(Some(1));
    std::thread::sleep(Duration::from_millis(5));
    let n = within(Duration::from_secs(20), || {
        let (vm, r) = run("let s = 0\nfor (let i = 0; i < 3000000; i = i + 1) { s = s + 1 }\nout = s", false);
        r.expect("another thread's expired deadline must not apply");
        global_num(&vm, "out")
    });
    set_thread_execution_deadline(None);
    assert_eq!(n, 3000000.0);
}

#[test]
fn jit_setting_is_inherited_by_closures_called_from_the_host() {
    let (vm, r) = run(&format!("{SPIN}spinFn = spin"), false);
    r.expect("runs");
    let spin = global_fn(&vm, "spinFn");
    let thrown = within(Duration::from_secs(10), move || {
        set_thread_execution_deadline(Some(100));
        let _ = spin.call(&[Value::Number(1e12)]);
        set_thread_execution_deadline(None);
        tishlang_core::take_pending_throw().map(|v| v.to_display_string())
    });
    let thrown = thrown.expect("a host call into a JIT-off closure must hit the deadline and throw");
    assert!(thrown.contains(DEADLINE_ERROR), "{thrown}");
}
