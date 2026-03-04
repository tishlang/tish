//! Shared builtin implementations for Tish.
//!
//! Used by the compiled runtime (tish_runtime). The interpreter (tish_eval)
//! implements builtins inline due to different Value and native signatures.

pub mod array;
pub mod string;
pub mod object;
pub mod math;
pub mod helpers;

pub use tish_core::Value;
