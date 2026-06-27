//! Tish Core - Shared types and utilities for the Tish language.
//!
//! This crate provides the unified Value type and utilities used by both
//! the interpreter (tishlang_eval) and compiled runtime (tishlang_runtime).

mod console_style;
mod json;
mod macros;
mod shape;
mod uri;
mod value;
mod vmref;

pub use console_style::{format_value_styled, format_values_for_console, use_console_colors};
pub use json::{json_parse, json_stringify, json_stringify_into, write_json_number};
pub use shape::{ShapeId, DICT_SHAPE, EMPTY_SHAPE};
pub use uri::{percent_decode, percent_encode};
pub use arcstr::ArcStr;
pub use value::*;
pub use vmref::{VmReadGuard, VmRef, VmWriteGuard};

/// `process.argv` for the interpreter / VM. Defaults to the host process's own `std::env::args()`,
/// but `tish run <file> [args...]` overrides it (via [`set_process_argv`]) with a node-shaped argv
/// `[tish-exe, <file>, args...]` so a script sees its own args — not the `run` subcommand. Compiled
/// native binaries don't touch this; they read `std::env::args()` directly (which is already their
/// own argv). #88
static PROCESS_ARGV: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Override `process.argv` for the interpreter/VM (set once by `tish run` before executing). A
/// later call is ignored (the first override wins), matching the single-program-per-process model.
pub fn set_process_argv(argv: Vec<String>) {
    let _ = PROCESS_ARGV.set(argv);
}

/// The argv a script should see as `process.argv`: the [`set_process_argv`] override if present,
/// else the host process's `std::env::args()`.
pub fn process_argv() -> Vec<String> {
    match PROCESS_ARGV.get() {
        Some(v) => v.clone(),
        None => std::env::args().collect(),
    }
}

/// #303 — a thrown JS value parked while it unwinds across a boundary that can't carry a `Result`.
///
/// The native value-fn ABI is `Fn(&[Value]) -> Value`, and `Callable::call` likewise returns a bare
/// `Value`, so a `throw` crossing such a boundary can't ride a `Result`. It is parked here and picked
/// up at the next checkpoint: native codegen checks it after each call; the VM checks it after each
/// `Callable::call`; and the shared array builtins (`forEach`/`map`/`sort`/…) check it between
/// elements so they stop iterating promptly instead of running the callback for the rest of the
/// array. The slot lives in `tishlang_core` (rather than `tishlang_runtime`) so `tishlang_builtins`
/// can poll it without a `builtins -> runtime` dependency cycle; `tishlang_runtime` and `tishlang_vm`
/// share this one slot. First-throw-wins; drained exactly once by [`take_pending_throw`] at the frame
/// that converts it back into a `Result`.
thread_local! {
    static PENDING_THROW: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Park a thrown value to propagate across a non-`Result` boundary. First-throw-wins: if one is
/// already pending (an erroneous continuation reached a second throw before the slot was drained),
/// keep the first — that is the throw JS would have raised.
pub fn set_pending_throw(v: Value) {
    PENDING_THROW.with(|c| {
        let mut slot = c.borrow_mut();
        if slot.is_none() {
            *slot = Some(v);
        }
    });
}

/// Is a thrown value waiting to propagate?
pub fn has_pending_throw() -> bool {
    PENDING_THROW.with(|c| c.borrow().is_some())
}

/// Take the parked thrown value, clearing the slot (drains it exactly once).
pub fn take_pending_throw() -> Option<Value> {
    PENDING_THROW.with(|c| c.borrow_mut().take())
}
