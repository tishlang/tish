//! Experimental LLVM backend for Tish.
//!
//! This crate is a placeholder for future LLVM-based code generation.
//! It would compile Tish bytecode or AST to LLVM IR, then to native machine code.
//!
//! Status: Not implemented. The crate exists to reserve the name and allow
//! experimentation without blocking the main codebase.

use std::path::Path;

/// Compile a Tish program to a native binary via LLVM.
///
/// Placeholder - returns an error. Implement when LLVM bindings are added.
pub fn compile_to_native(
    _entry_path: &Path,
    _project_root: Option<&Path>,
    _output_path: &Path,
) -> Result<(), LlvmError> {
    Err(LlvmError {
        message: "LLVM backend not implemented yet. Use --native-backend rust or cranelift.".to_string(),
    })
}

/// Error from LLVM compilation.
#[derive(Debug)]
pub struct LlvmError {
    pub message: String,
}

impl std::fmt::Display for LlvmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LlvmError {}
