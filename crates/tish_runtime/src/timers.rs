//! setTimeout, clearTimeout for compiled Tish and VM.
//! Callbacks run when blocking ops (e.g. ws.receiveTimeout) yield in their poll loop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tishlang_core::Value;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

struct TimerEntry {
    due: Instant,
    callback: Value,
    args: Vec<Value>,
    interval_ms: u64,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<u64, TimerEntry>> = RefCell::new(HashMap::new());
}

fn extract_num(v: Option<&Value>) -> u64 {
    v.and_then(|x| match x {
        Value::Number(n) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    })
    .unwrap_or(0)
}

/// Sleep for ms, running due timers before sleeping. Use this instead of thread::sleep
/// in blocking loops so setTimeout callbacks can fire.
#[allow(dead_code)] // Used by embedders with blocking poll loops; AppKit uses [`drain_timers`] instead.
pub fn sleep_with_drain(ms: u64) {
    run_due_timers();
    std::thread::sleep(Duration::from_millis(ms));
}

/// Run all due timer callbacks (e.g. from an AppKit / GUI event pump).
#[inline]
pub fn drain_timers() {
    run_due_timers();
}

/// Run all due timer callbacks.
fn run_due_timers() {
    let due = take_due_timers();
    for (id, callback, args, interval_ms) in due {
        if let Value::Function(f) = &callback {
            let _ = f(&args);
        }
        if interval_ms > 0 {
            re_register_interval(id, callback, args, interval_ms);
        }
    }
}

fn take_due_timers() -> Vec<(u64, Value, Vec<Value>, u64)> {
    let now = Instant::now();
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let due: Vec<_> = reg
            .iter()
            .filter(|(_, e)| e.due <= now)
            .map(|(id, e)| (*id, e.callback.clone(), e.args.clone(), e.interval_ms))
            .collect();
        for (id, _, _, _) in &due {
            reg.remove(id);
        }
        due
    })
}

fn re_register_interval(id: u64, callback: Value, args: Vec<Value>, interval_ms: u64) {
    let due = Instant::now() + Duration::from_millis(interval_ms);
    REGISTRY.with(|r| {
        r.borrow_mut().insert(
            id,
            TimerEntry {
                due,
                callback,
                args,
                interval_ms,
            },
        );
    });
}

/// setTimeout(callback, delayMs, ...args) - returns timer id.
/// Callbacks run when run_due_timers() is invoked (e.g. from ws.receiveTimeout poll loop).
pub fn set_timeout(args: &[Value]) -> Value {
    let callback = args.first().cloned().unwrap_or(Value::Null);
    let delay_ms = extract_num(args.get(1)).min(3600_000);
    let extra_args: Vec<Value> = args.iter().skip(2).cloned().collect();
    if matches!(callback, Value::Null) {
        return Value::Number(next_id() as f64);
    }
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(delay_ms);
    REGISTRY.with(|r| {
        r.borrow_mut().insert(
            id,
            TimerEntry {
                due,
                callback,
                args: extra_args,
                interval_ms: 0,
            },
        );
    });
    Value::Number(id as f64)
}

/// clearTimeout(id) - removes timer.
pub fn clear_timeout(args: &[Value]) -> Value {
    let id = args
        .first()
        .and_then(|v| match v {
            Value::Number(n) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
            _ => None,
        })
        .unwrap_or(0);
    REGISTRY.with(|r| {
        r.borrow_mut().remove(&id);
    });
    Value::Null
}
