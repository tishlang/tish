//! Native compiler backend for Tish.
//!
//! Emits Rust source that links to tish_runtime.

mod codegen;

pub use codegen::compile;
pub use codegen::CompileError;
