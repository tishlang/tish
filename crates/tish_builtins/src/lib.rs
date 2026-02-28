//! Shared builtin implementations for Tish.
//!
//! This crate provides shared implementations for builtin functions
//! that are used by both the interpreter (tish_eval) and the compiled
//! runtime (tish_runtime).

pub mod array;
pub mod string;
pub mod object;
pub mod math;
pub mod helpers;

pub use tish_core::Value;
