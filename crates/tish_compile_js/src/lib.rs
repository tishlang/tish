//! Tish to JavaScript transpiler backend.
//! Uses shared resolve from tish_compile for unified pipeline.

mod codegen;
mod error;
mod js_intrinsics;

pub use codegen::{compile, compile_project};
pub use error::CompileError;
