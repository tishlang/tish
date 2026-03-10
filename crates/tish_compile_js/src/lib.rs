//! Tish to JavaScript transpiler backend.
//! Uses shared resolve from tish_compile for unified pipeline.

mod codegen;

pub use codegen::{compile, compile_project, CompileError};
