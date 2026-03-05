//! Utilities for exposing native Rust modules to Tish.
//!
//! Re-exports `TishOpaque` and related types from tish_core.
//! The `TishNativeModule` trait lives in tish_eval for interpreter registration.

pub use tish_core::{NativeFn, TishOpaque, Value};
