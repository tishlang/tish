//! Native code generation backend for Tish.
//!
//! - **`rust`:** `tishlang_compile` emits Rust calling **`tishlang_runtime`** (`Value`, etc.),
//!   then `cargo build --release` links the user binary.
//! - **`cranelift`:** Embeds serialized bytecode in an object file and links **`tishlang_cranelift_runtime`**
//!   — the executable runs **`tishlang_vm`** on that chunk (same as `tish run --backend vm`), not CLIF lowering.
//! - **`llvm`:** Same embedded-bytecode + VM link path via `tishlang_llvm` / shared linker.
//!
//! **Future:** Lower bytecode (or typed IR) through Cranelift/LLVM to real machine code where semantics allow;
//! emit Rust using `Vec<f64>` / fixed primitives instead of `Value` on hot paths.

mod build;

use std::path::Path;
use tishlang_ast::Program;

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
/// - `native_backend == "rust"`: Rust source + `tishlang_runtime` + cargo (native imports).
/// - `native_backend == "cranelift"`: Embedded bytecode + VM binary (pure Tish only); not opcode AOT yet.
/// - `native_backend == "llvm"`: Embedded bytecode + VM via LLVM/clang link path.
pub fn compile_to_native(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    features: &[String],
    native_backend: &str,
    optimize: bool,
) -> Result<(), NativeError> {
    let backend = match native_backend {
        "rust" => Backend::Rust,
        "cranelift" => Backend::Cranelift,
        "llvm" => Backend::Llvm,
        _ => {
            return Err(NativeError {
                message: format!(
                    "Invalid native backend '{}'. Use 'rust', 'cranelift', or 'llvm'.",
                    native_backend
                ),
            });
        }
    };

    match backend {
        Backend::Rust => {
            let (rust_code, native_modules, effective_features, native_build) =
                tishlang_compile::compile_project_full(
                    entry_path,
                    project_root,
                    features,
                    optimize,
                )
                .map_err(|e| NativeError {
                    message: e.to_string(),
                })?;

            crate::build::build_via_cargo(
                &rust_code,
                native_modules,
                output_path,
                &effective_features,
                &native_build.rust_dependencies_toml,
                native_build.generated_native_rs.as_deref(),
                project_root,
            )
            .map_err(|e| NativeError { message: e })
        }
        Backend::Cranelift => {
            let modules = tishlang_compile::resolve_project(entry_path, project_root)
                .map_err(|e| NativeError {
                    message: e.to_string(),
                })?;
            tishlang_compile::detect_cycles(&modules).map_err(|e| NativeError {
                message: e.to_string(),
            })?;
            let program = {
                let prog = tishlang_compile::merge_modules(modules).map_err(|e| NativeError {
                    message: e.to_string(),
                })?;
                if optimize {
                    tishlang_opt::optimize(&prog)
                } else {
                    prog
                }
            };

            if tishlang_compile::has_external_native_imports(&program) {
                return Err(NativeError {
                    message: "Cranelift backend does not support external native imports (tish:…, cargo:…, @scope/pkg). Built-in tish:fs, tish:http, tish:process are supported. Use --native-backend rust for external modules.".to_string(),
                });
            }

            let chunk = if optimize {
                tishlang_bytecode::compile(&program).map_err(|e| NativeError {
                    message: e.to_string(),
                })?
            } else {
                tishlang_bytecode::compile_unoptimized(&program).map_err(|e| NativeError {
                    message: e.to_string(),
                })?
            };

            let cranelift_features = tishlang_compile::extract_native_import_features(&program);
            tishlang_cranelift::compile_chunk_to_native(&chunk, output_path, &cranelift_features)
                .map_err(|e| NativeError {
                    message: e.to_string(),
                })
        }
        Backend::Llvm => {
            let modules = tishlang_compile::resolve_project(entry_path, project_root)
                .map_err(|e| NativeError { message: e.to_string() })?;
            tishlang_compile::detect_cycles(&modules).map_err(|e| NativeError { message: e.to_string() })?;
            let program = {
                let prog = tishlang_compile::merge_modules(modules).map_err(|e| NativeError {
                    message: e.to_string(),
                })?;
                if optimize {
                    tishlang_opt::optimize(&prog)
                } else {
                    prog
                }
            };
            if tishlang_compile::has_external_native_imports(&program) {
                return Err(NativeError {
                    message: "LLVM backend does not support external native imports (tish:…, cargo:…, @scope/pkg). Built-in tish:fs, tish:http, tish:process are supported.".to_string(),
                });
            }
            let chunk = if optimize {
                tishlang_bytecode::compile(&program).map_err(|e| NativeError {
                    message: e.to_string(),
                })?
            } else {
                tishlang_bytecode::compile_unoptimized(&program).map_err(|e| NativeError {
                    message: e.to_string(),
                })?
            };
            let llvm_features = tishlang_compile::extract_native_import_features(&program);
            tishlang_llvm::compile_chunk_to_native(&chunk, output_path, &llvm_features)
                .map_err(|e| NativeError { message: e.message })
        }
    }
}

/// Compile a single Program (e.g. from tishlang_js_to_tish) to native.
pub fn compile_program_to_native(
    program: &Program,
    project_root: Option<&Path>,
    output_path: &Path,
    features: &[String],
    native_backend: &str,
    optimize: bool,
) -> Result<(), NativeError> {
    let backend = match native_backend {
        "rust" => Backend::Rust,
        "cranelift" => Backend::Cranelift,
        "llvm" => Backend::Llvm,
        _ => {
            return Err(NativeError {
                message: format!(
                    "Invalid native backend '{}'. Use 'rust', 'cranelift', or 'llvm'.",
                    native_backend
                ),
            });
        }
    };

    match backend {
        Backend::Rust => {
            let program = if optimize { tishlang_opt::optimize(program) } else { program.clone() };
            let root = project_root.unwrap_or_else(|| Path::new("."));
            let native_modules = tishlang_compile::resolve_native_modules(&program, root)
                .map_err(|e| NativeError { message: e })?;
            let native_build = tishlang_compile::compute_native_build_artifacts(&program, root, &native_modules)
                .map_err(|e| NativeError { message: e })?;
            let mut all_features = features.to_vec();
            for f in tishlang_compile::extract_native_import_features(&program) {
                if !all_features.contains(&f) {
                    all_features.push(f);
                }
            }
            let rust_code = tishlang_compile::compile_with_native_modules(
                &program,
                project_root,
                &all_features,
                &native_modules,
                &native_build.native_init,
                optimize,
            )
            .map_err(|e| NativeError {
                message: e.message,
            })?;
            crate::build::build_via_cargo(
                &rust_code,
                native_modules,
                output_path,
                &all_features,
                &native_build.rust_dependencies_toml,
                native_build.generated_native_rs.as_deref(),
                Some(root),
            )
            .map_err(|e| NativeError { message: e })
        }
        Backend::Cranelift => {
            if tishlang_compile::has_external_native_imports(program) {
                return Err(NativeError {
                    message: "Cranelift backend does not support external native imports (tish:…, cargo:…, @scope/pkg). Built-in tish:fs, tish:http, tish:process are supported.".to_string(),
                });
            }
            let program = if optimize { tishlang_opt::optimize(program) } else { program.clone() };
            let chunk = if optimize {
                tishlang_bytecode::compile(&program).map_err(|e| NativeError { message: e.to_string() })?
            } else {
                tishlang_bytecode::compile_unoptimized(&program).map_err(|e| NativeError { message: e.to_string() })?
            };
            let cranelift_features = tishlang_compile::extract_native_import_features(&program);
            tishlang_cranelift::compile_chunk_to_native(&chunk, output_path, &cranelift_features)
                .map_err(|e| NativeError { message: e.to_string() })
        }
        Backend::Llvm => {
            if tishlang_compile::has_external_native_imports(program) {
                return Err(NativeError {
                    message: "LLVM backend does not support external native imports (tish:…, cargo:…, @scope/pkg).".to_string(),
                });
            }
            let program = if optimize { tishlang_opt::optimize(program) } else { program.clone() };
            let chunk = if optimize {
                tishlang_bytecode::compile(&program).map_err(|e| NativeError { message: e.to_string() })?
            } else {
                tishlang_bytecode::compile_unoptimized(&program).map_err(|e| NativeError { message: e.to_string() })?
            };
            let llvm_features = tishlang_compile::extract_native_import_features(&program);
            tishlang_llvm::compile_chunk_to_native(&chunk, output_path, &llvm_features)
                .map_err(|e| NativeError { message: e.message })
        }
    }
}

enum Backend {
    Rust,
    Cranelift,
    Llvm,
}
