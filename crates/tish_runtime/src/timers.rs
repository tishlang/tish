//! setTimeout, clearTimeout for compiled Tish.
//! Native compile: setTimeout returns an id, clearTimeout is a no-op.
//! Timer callbacks do not run in native (Value is !Send); use interpreter for full timer support.

use std::sync::atomic::{AtomicU64, Ordering};
use tish_core::Value;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

fn extract_num(v: Option<&Value>) -> u64 {
    v.and_then(|x| match x {
        Value::Number(n) => Some(*n as u64),
        _ => None,
    })
    .unwrap_or(0)
}

/// setTimeout(callback, delayMs, ...args) - returns timer id.
/// Note: In native compile, callbacks do not run (Value is !Send).
/// Use interpreter for non-blocking timer callbacks.
pub fn set_timeout(args: &[Value]) -> Value {
    let _delay_ms = extract_num(args.get(1));
    let _ = args; // suppress unused
    Value::Number(next_id() as f64)
}

/// clearTimeout(id) - no-op in native (timers do not run).
pub fn clear_timeout(args: &[Value]) -> Value {
    let _ = args;
    Value::Null
}
