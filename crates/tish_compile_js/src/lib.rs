//! Tish to JavaScript transpiler backend.

mod codegen;
mod resolve;

pub use codegen::{compile, compile_project, CompileError};
