//! setTimeout, setInterval, clearTimeout, clearInterval.
//! Non-blocking: setTimeout returns immediately; callbacks run in a drain phase
//! after the script yields (when run() finishes the synchronous program).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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

/// Maximum number of LIVE (registered, not-yet-fired) timers per worker thread. A program that keeps
/// scheduling `setTimeout`/`setInterval` without them draining would otherwise grow the registry (and
/// its retained callbacks + closed-over data) without bound (#384, ported to this twin by #699). Past
/// the cap a new timer is dropped rather than registered. Override with `TISH_MAX_TIMERS`; default
/// 100k — far above any real timer workload, low enough to bound memory.
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

/// Register a one-shot timer. Returns immediately with timer id.
#[allow(non_snake_case)]
pub fn setTimeout(callback: Value, args: Vec<Value>, delay_ms: u64) -> u64 {
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
                args,
                interval_ms: 0,
            },
        );
    });
    id
}

/// Register a repeating timer. Returns immediately with timer id.
#[allow(non_snake_case)]
pub fn setInterval(callback: Value, args: Vec<Value>, delay_ms: u64) -> u64 {
    let id = next_id();
    let due = Instant::now() + Duration::from_millis(delay_ms);
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        // #384: bound the live-timer count (see setTimeout).
        if reg.len() >= max_live_timers() {
            return;
        }
        reg.insert(
            id,
            TimerEntry {
                due,
                callback,
                args,
                interval_ms: delay_ms,
            },
        );
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
///
/// Deliberately uncapped, matching the tish_runtime twin: this re-inserts an entry the drain
/// phase just removed, so it cannot grow the registry beyond what registration allowed, and
/// capping it would silently kill a live interval whenever the registry sits at the cap.
pub fn re_register_interval(id: u64, callback: Value, args: Vec<Value>, interval_ms: u64) {
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

#[cfg(test)]
mod timer_cap_tests_699 {
    use super::*;

    fn noop_callback() -> Value {
        Value::Native(|_| Ok(Value::Null))
    }

    #[test]
    fn registration_is_bounded_by_max_timers() {
        // Each `#[test]` runs on its own thread, so REGISTRY (thread_local) starts empty here.
        // One test (not one per entry point): TISH_MAX_TIMERS is process-global, and parallel
        // tests mutating it would race.
        std::env::set_var("TISH_MAX_TIMERS", "5");
        for _ in 0..20 {
            let _ = setTimeout(noop_callback(), Vec::new(), 10_000);
        }
        assert_eq!(
            registry_len(),
            5,
            "live timers must be capped at TISH_MAX_TIMERS"
        );
        for _ in 0..20 {
            let _ = setInterval(noop_callback(), Vec::new(), 10_000);
        }
        assert_eq!(registry_len(), 5, "setInterval must honor the same cap");
        std::env::remove_var("TISH_MAX_TIMERS");
    }
}
