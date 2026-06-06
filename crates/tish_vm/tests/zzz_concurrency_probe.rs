//! TEMPORARY probe: faithful handler (Date.now busy-loop + object return), true parallel.
#![cfg(feature = "send-values")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tishlang_bytecode::compile;
use tishlang_core::Value;
use tishlang_vm::Vm;

type Handler = Arc<dyn Fn(&[Value]) -> Value + Send + Sync>;

fn build(src: &str) -> Handler {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    let mut vm = Vm::new();
    vm.run(&chunk).expect("run");
    match vm.get_global("exported").expect("exported global") {
        Value::Function(f) => f,
        other => panic!("expected fn, got {:?}", other),
    }
}

fn run_parallel(label: &str, handler: &Handler, threads: usize, iters: usize) {
    let done = Arc::new(AtomicUsize::new(0));
    let total = threads * iters;
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..threads {
        let h = handler.clone();
        let done = Arc::clone(&done);
        handles.push(std::thread::spawn(move || {
            for i in 0..iters {
                let _ = h(&[Value::Number(i as f64)]);
                done.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    let mut last = 0;
    let mut last_change = Instant::now();
    while done.load(Ordering::Relaxed) < total {
        let cur = done.load(Ordering::Relaxed);
        if cur != last {
            last = cur;
            last_change = Instant::now();
        }
        if last_change.elapsed() > Duration::from_secs(8) {
            panic!(
                "[{}] DEADLOCK: stalled at {}/{} after {:?}",
                label,
                cur,
                total,
                start.elapsed()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for h in handles {
        h.join().unwrap();
    }
    eprintln!("[{}] OK: {} calls, {} threads, {:?}", label, total, threads, start.elapsed());
}

#[test]
fn faithful_counter_handler_parallel() {
    let counter = build(
        r#"
let active = 0
let maxActive = 0
fn handleRequest(req) {
    active = active + 1
    if (active > maxActive) { maxActive = active }
    let start = Date.now()
    while (Date.now() - start < 80) { }
    active = active - 1
    return { status: 200, body: "ok" }
}
exported = handleRequest
"#,
    );
    run_parallel("counter", &counter, 8, 6);
}

#[test]
fn faithful_nocounter_handler_parallel() {
    let nocounter = build(
        r#"
fn handleRequest(req) {
    let start = Date.now()
    while (Date.now() - start < 80) { }
    return { status: 200, body: "ok" }
}
exported = handleRequest
"#,
    );
    run_parallel("nocounter", &nocounter, 8, 6);
}
