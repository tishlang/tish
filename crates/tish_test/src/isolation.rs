//! Per-file isolation hooks for multi-file `tish test` runs.
//!
//! Clears pending throws and the mock registry between files. Timers / HTTP static routes /
//! sockets are **not** reset in-process — suites that leak host state should use separate
//! processes.

use tishlang_core::take_pending_throw;

/// Reset host state between test files in the same process.
pub fn reset_between_files() {
    let _ = take_pending_throw();
    // Otherwise `restoreAllMocks()` in one file drains handles registered by another.
    crate::mocks::reset_mock_registry();
}
