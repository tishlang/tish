//! Shared builtin implementations for Tish.
//!
//! Used by the compiled runtime (tishlang_runtime) and bytecode VM (tishlang_vm). The
//! interpreter (tishlang_eval) implements builtins inline due to different Value
//! and native signatures.

pub mod array;
pub mod string;
pub mod object;
pub mod math;
pub mod helpers;
pub mod globals;

pub use tishlang_core::Value;
