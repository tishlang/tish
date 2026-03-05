//! setTimeout, setInterval, clearTimeout, clearInterval.
//! Non-blocking: setTimeout returns immediately; callbacks run in a drain phase
//! after the script yields (when run() finishes the synchronous program).

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::value::Value;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TimerEntry {
    due: Instant,
    callback: Value,
    args: Vec<Value>,
    interval_ms: u64,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<u64, TimerEntry>> = RefCell::new(HashMap::new());
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

/// Register a one-shot timer. Returns immediately with timer id.
#[allow(non_snake_case)]
pub fn setTimeout(callback: Value, args: Vec<Value>, delay_ms: u64) -> u64 {
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(delay_ms);
    REGISTRY.with(|r| {
        r.borrow_mut().insert(id, TimerEntry {
            due,
            callback,
            args,
            interval_ms: 0,
        });
    });
    id
}

/// Register a repeating timer. Returns immediately with timer id.
#[allow(non_snake_case)]
pub fn setInterval(callback: Value, args: Vec<Value>, delay_ms: u64) -> u64 {
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(delay_ms);
    REGISTRY.with(|r| {
        r.borrow_mut().insert(id, TimerEntry {
            due,
            callback,
            args,
            interval_ms: delay_ms,
        });
    });
    id
}

/// Remove a timer. No-op if already fired or invalid.
#[allow(non_snake_case)]
pub fn clearTimer(id: u64) {
    REGISTRY.with(|r| {
        r.borrow_mut().remove(&id);
    });
}

/// Take all due timers and return (id, callback, args, interval_ms). Removes them from registry.
/// Caller should run callbacks; for interval_ms > 0, caller should re-register.
pub fn take_due_timers() -> Vec<(u64, Value, Vec<Value>, u64)> {
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

/// Re-register an interval timer (called after running its callback).
pub fn re_register_interval(id: u64, callback: Value, args: Vec<Value>, interval_ms: u64) {
    let due = Instant::now() + Duration::from_millis(interval_ms);
    REGISTRY.with(|r| {
        r.borrow_mut().insert(id, TimerEntry {
            due,
            callback,
            args,
            interval_ms,
        });
    });
}

/// Check if any timers are still pending.
pub fn has_pending_timers() -> bool {
    REGISTRY.with(|r| !r.borrow().is_empty())
}

/// Return the instant when the next timer is due, or None if registry is empty.
pub fn next_due_instant() -> Option<Instant> {
    REGISTRY.with(|r| {
        let reg = r.borrow();
        reg.values().map(|e| e.due).min()
    })
}
