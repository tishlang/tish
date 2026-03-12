//! Bytecode to native via Cranelift.
//!
//! Compiles Tish bytecode to native object files and links with a minimal runtime.

mod link;
mod lower;

use std::path::Path;

use tish_bytecode::Chunk;

/// Error from Cranelift compilation.
#[derive(Debug)]
pub struct CraneliftError {
    pub message: String,
}

impl std::fmt::Display for CraneliftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CraneliftError {}

/// Compile a bytecode chunk to a native binary.
pub fn compile_chunk_to_native(chunk: &Chunk, output_path: &Path) -> Result<(), CraneliftError> {
    let object_path = output_path.with_extension("o");
    lower::lower_and_emit(chunk, &object_path)?;
    link::link_to_binary(&object_path, output_path)?;
    // Clean up .o file
    let _ = std::fs::remove_file(&object_path);
    Ok(())
}
