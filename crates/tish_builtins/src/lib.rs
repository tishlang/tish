//! Shared builtin implementations for Tish.
//!
//! Used by the compiled runtime (tishlang_runtime) and bytecode VM (tishlang_vm). The
//! interpreter (tishlang_eval) implements builtins inline due to different Value
//! and native signatures.
#![cfg_attr(feature = "portable", no_std)]

extern crate alloc;

pub mod array;
pub mod collections;
pub mod construct;
pub mod date;
pub mod globals;
pub mod helpers;
pub mod iterator;
pub mod math;
pub mod number;
pub mod object;
pub mod string;
pub mod symbol;
pub mod typedarrays;

pub use tishlang_core::Value;
