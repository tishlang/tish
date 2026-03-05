//! setTimeout, setInterval, clearTimeout, clearInterval.
//! Uses blocking sleep. Callbacks run synchronously via the Evaluator.
//! These are invoked from eval via run_set_timeout etc., not as plain natives.

use std::sync::atomic::{AtomicU64, Ordering};

/// Generate next timer id.
pub fn next_timer_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}
