//! setTimeout, setInterval, clearTimeout, clearInterval for compiled Tish and VM.
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

/// Maximum number of LIVE (registered, not-yet-fired) timers per worker thread. A program that keeps
/// scheduling `setTimeout`/`setInterval` without them draining would otherwise grow the registry (and
/// its retained callbacks + closed-over data) without bound (#384). Past the cap a new timer is
/// dropped rather than registered. Override with `TISH_MAX_TIMERS`; default 100k — far above any real
/// timer workload, low enough to bound memory.
fn max_live_timers() -> usize {
    // Read per call (timer registration is not a hot path, and the cap exists precisely to bound the
    // pathological caller): keeps the limit overridable per test without a process-global cache.
    std::env::var("TISH_MAX_TIMERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(100_000)
}

#[cfg(test)]
fn registry_len() -> usize {
    REGISTRY.with(|r| r.borrow().len())
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

/// Run all due timer callbacks (including timers scheduled by other timers).
fn run_due_timers() {
    for _ in 0..64 {
        let due = take_due_timers();
        if due.is_empty() {
            break;
        }
        for (id, callback, args, interval_ms) in due {
            if let Value::Function(f) = &callback {
                let _ = f.call(&args);
            }
            if interval_ms > 0 {
                re_register_interval(id, callback, args, interval_ms);
            }
        }
    }
}

fn take_due_timers() -> Vec<(u64, Value, Vec<Value>, u64)> {
    let now = Instant::now();
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let mut due: Vec<_> = reg
            .iter()
            .filter(|(_, e)| e.due <= now)
            .map(|(id, e)| (e.due, *id, e.callback.clone(), e.args.clone(), e.interval_ms))
            .collect();
        // Deterministic JS timer order: earliest `due` first, ties broken by registration order
        // (the monotonic id). REGISTRY is a HashMap whose iteration order is otherwise arbitrary,
        // which scrambled same-delay timers (e.g. three `setTimeout(_, 0)` firing out of order).
        due.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, id, _, _, _) in &due {
            reg.remove(id);
        }
        due.into_iter()
            .map(|(_, id, cb, args, iv)| (id, cb, args, iv))
            .collect()
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
    let delay_ms = extract_num(args.get(1)).min(3_600_000);
    let extra_args: Vec<Value> = args.iter().skip(2).cloned().collect();
    if matches!(callback, Value::Null) {
        return Value::Number(next_id() as f64);
    }
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(delay_ms);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        // #384: bound the live-timer count so a runaway scheduler can't grow the registry unbounded.
        if reg.len() >= max_live_timers() {
            return;
        }
        reg.insert(
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

/// setInterval(callback, intervalMs, ...args) — first run after `intervalMs`, then repeats.
pub fn set_interval(args: &[Value]) -> Value {
    let callback = args.first().cloned().unwrap_or(Value::Null);
    let interval_ms = extract_num(args.get(1)).min(3_600_000);
    let extra_args: Vec<Value> = args.iter().skip(2).cloned().collect();
    if matches!(callback, Value::Null) {
        return Value::Number(next_id() as f64);
    }
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(interval_ms);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        // #384: bound the live-timer count (see set_timeout).
        if reg.len() >= max_live_timers() {
            return;
        }
        reg.insert(
            id,
            TimerEntry {
                due,
                callback,
                args: extra_args,
                interval_ms,
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

/// clearInterval(id) — same registry as clearTimeout.
pub fn clear_interval(args: &[Value]) -> Value {
    clear_timeout(args)
}

#[cfg(test)]
mod timer_cap_tests_384 {
    use super::*;

    #[test]
    fn set_timeout_is_bounded_by_max_timers() {
        // Each `#[test]` runs on its own thread, so REGISTRY (thread_local) starts empty here.
        std::env::set_var("TISH_MAX_TIMERS", "5");
        let cb = tishlang_core::native_fn(|_| Value::Null);
        for _ in 0..20 {
            let _ = set_timeout(&[Value::Function(cb.clone()), Value::Number(10_000.0)]);
        }
        assert_eq!(registry_len(), 5, "live timers must be capped at TISH_MAX_TIMERS");
        std::env::remove_var("TISH_MAX_TIMERS");
    }
}
