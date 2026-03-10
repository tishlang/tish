//! WASM backend for Tish.
//!
//! Target architecture (per plan):
//! - Bytecode -> WASM linear IR -> .wasm
//! - Runtime: small WASM module (Value, builtins) compiled once
//!
//! Current: Placeholder. Use `--target js` for browser output until WASM backend is implemented.

use std::path::Path;

/// Error from WASM compilation.
#[derive(Debug)]
pub struct WasmError {
    pub message: String,
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for WasmError {}

/// Compile a Tish project to WebAssembly (.wasm).
///
/// Not yet implemented. Use `tish compile --target js` for browser output.
pub fn compile_to_wasm(
    _entry_path: &Path,
    _project_root: Option<&Path>,
    _output_path: &Path,
) -> Result<(), WasmError> {
    Err(WasmError {
        message: "WASM compilation not yet implemented. Use --target js for browser output."
            .to_string(),
    })
}
