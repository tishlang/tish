//! Standalone native binary: embedded bytecode + VM (Cranelift used as object builder).
//!
//! Produces an executable that runs **`tishlang_vm`** on serialized bytecode embedded in
//! the binary — not lowering of Tish opcodes to CLIF/machine code (see module docs in
//! `lower.rs`). For Rust transpile + `tishlang_runtime`, use `--native-backend rust`.

mod link;
mod lower;

pub use link::link_to_binary;

use std::path::Path;

use tishlang_bytecode::Chunk;

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

/// Build a native binary that embeds `chunk` and runs it with the bytecode VM.
/// `features` are passed to `tishlang_cranelift_runtime` (e.g. fs, process, http).
pub fn compile_chunk_to_native(
    chunk: &Chunk,
    output_path: &Path,
    features: &[String],
) -> Result<(), CraneliftError> {
    let object_path = output_path.with_extension("o");
    lower::lower_and_emit(chunk, &object_path)?;
    link::link_to_binary(&object_path, output_path, features)?;
    // Clean up .o file
    let _ = std::fs::remove_file(&object_path);
    Ok(())
}
