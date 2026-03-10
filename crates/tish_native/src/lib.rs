//! Native code generation backend for Tish.
//!
//! Target architecture (per plan):
//! - Phase 2: Bytecode -> Cranelift IR -> .o -> link with minimal runtime
//! - Current: Delegates to tish_compile (Rust codegen) + cargo build as interim path
//!
//! Once Cranelift backend is implemented, this crate will:
//! 1. Take Chunk (bytecode) as input
//! 2. Lower to Cranelift IR
//! 3. Emit .o via cranelift-object
//! 4. Link against prebuilt tish_runtime staticlib

mod build;

use std::path::Path;

/// Error from native compilation.
#[derive(Debug)]
pub struct NativeError {
    pub message: String,
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NativeError {}

/// Compile a Tish project to a native binary.
///
/// - `native_backend == "rust"`: Full Rust codegen + cargo build (supports native imports).
/// - `native_backend == "cranelift"`: Bytecode -> Cranelift -> native (pure Tish only).
pub fn compile_to_native(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    features: &[String],
    native_backend: &str,
) -> Result<(), NativeError> {
    let backend = match native_backend {
        "rust" => Backend::Rust,
        "cranelift" => Backend::Cranelift,
        _ => {
            return Err(NativeError {
                message: format!(
                    "Invalid native backend '{}'. Use 'rust' or 'cranelift'.",
                    native_backend
                ),
            });
        }
    };

    match backend {
        Backend::Rust => {
            let (rust_code, native_modules) = tish_compile::compile_project_full(
                entry_path,
                project_root,
                features,
            )
            .map_err(|e| NativeError {
                message: e.to_string(),
            })?;

            crate::build::build_via_cargo(&rust_code, native_modules, output_path, features)
                .map_err(|e| NativeError { message: e })
        }
        Backend::Cranelift => {
            let modules = tish_compile::resolve_project(entry_path, project_root)
                .map_err(|e| NativeError {
                    message: e.to_string(),
                })?;
            tish_compile::detect_cycles(&modules).map_err(|e| NativeError {
                message: e.to_string(),
            })?;
            let program = tish_compile::merge_modules(modules).map_err(|e| NativeError {
                message: e.to_string(),
            })?;

            if tish_compile::has_native_imports(&program) {
                return Err(NativeError {
                    message: "Cranelift backend does not support native imports (tish:*, @scope/pkg). Use --native-backend rust.".to_string(),
                });
            }

            let chunk = tish_bytecode::compile(&program).map_err(|e| NativeError {
                message: e.to_string(),
            })?;

            tish_cranelift::compile_chunk_to_native(&chunk, output_path).map_err(|e| NativeError {
                message: e.to_string(),
            })
        }
    }
}

enum Backend {
    Rust,
    Cranelift,
}
