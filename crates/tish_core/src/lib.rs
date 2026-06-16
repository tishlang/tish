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
pub use json::{json_parse, json_stringify, json_stringify_into};
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
