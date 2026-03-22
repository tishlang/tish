//! WebAssembly backend for Tish.
//!
//! Compiles Tish to bytecode, then produces a .wasm VM binary + loader.
//! The VM runs in the browser; your program runs as serialized bytecode.

use std::path::Path;
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use tishlang_ast::Program;
use tishlang_bytecode::{serialize, Chunk};
use tishlang_compile::{
    detect_cycles, extract_native_import_features, has_external_native_imports, merge_modules,
    resolve_project,
};

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

/// Resolve project, merge modules, and compile to bytecode chunk.
/// Returns (Chunk, Program) so WASI can extract features for the runtime.
fn resolve_and_compile_to_chunk(
    entry_path: &Path,
    project_root: Option<&Path>,
    optimize: bool,
) -> Result<(Chunk, Program), WasmError> {
    let modules = resolve_project(entry_path, project_root).map_err(|e| WasmError {
        message: e.to_string(),
    })?;
    detect_cycles(&modules).map_err(|e| WasmError {
        message: e.to_string(),
    })?;
    let program = {
        let prog = merge_modules(modules).map_err(|e| WasmError {
            message: e.to_string(),
        })?;
        if optimize {
            tishlang_opt::optimize(&prog)
        } else {
            prog
        }
    };
    let chunk = if optimize {
        tishlang_bytecode::compile(&program).map_err(|e| WasmError {
            message: e.to_string(),
        })?
    } else {
        tishlang_bytecode::compile_unoptimized(&program).map_err(|e| WasmError {
            message: e.to_string(),
        })?
    };
    Ok((chunk, program))
}

/// Compile a single Program (e.g. from tishlang_js_to_tish) for WebAssembly.
pub fn compile_program_to_wasm(
    program: &Program,
    output_path: &Path,
    optimize: bool,
) -> Result<(), WasmError> {
    let program = if optimize {
        tishlang_opt::optimize(program)
    } else {
        program.clone()
    };
    let chunk = if optimize {
        tishlang_bytecode::compile(&program).map_err(|e| WasmError {
            message: e.to_string(),
        })?
    } else {
        tishlang_bytecode::compile_unoptimized(&program).map_err(|e| WasmError {
            message: e.to_string(),
        })?
    };
    emit_wasm_from_chunk(&chunk, output_path)
}

fn emit_wasm_from_chunk(chunk: &Chunk, output_path: &Path) -> Result<(), WasmError> {
    let chunk_bytes = serialize(chunk);
    let chunk_b64 = BASE64.encode(&chunk_bytes);
    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main");
    let out_dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let out_dir_abs = out_dir
        .canonicalize()
        .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(out_dir)))
        .map_err(|e| WasmError {
            message: format!("Cannot resolve output dir: {}", e),
        })?;
    std::fs::create_dir_all(&out_dir_abs).map_err(|e| WasmError {
        message: format!("Cannot create output directory: {}", e),
    })?;
    let workspace_root = tishlang_build_utils::find_workspace_root().map_err(|e| WasmError {
        message: e,
    })?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let build_status = Command::new(&cargo)
        .current_dir(&workspace_root)
        .args([
            "build", "-p", "tishlang_wasm_runtime",
            "--target", "wasm32-unknown-unknown",
            "--release", "--features", "browser",
        ])
        .status()
        .map_err(|e| WasmError { message: format!("Failed to run cargo: {}", e) })?;
    if !build_status.success() {
        return Err(WasmError {
            message: "Failed to build wasm runtime. Run: rustup target add wasm32-unknown-unknown"
                .to_string(),
        });
    }
    let wasm_artifact = workspace_root
        .join("target/wasm32-unknown-unknown/release/tishlang_wasm_runtime.wasm");
    if !wasm_artifact.exists() {
        return Err(WasmError {
            message: format!("Wasm artifact not found: {}", wasm_artifact.display()),
        });
    }
    let wasm_bindgen = std::env::var("WASM_BINDGEN").unwrap_or_else(|_| "wasm-bindgen".to_string());
    let out_name = stem.to_string();
    let bindgen_status = Command::new(&wasm_bindgen)
        .args([
            "--target", "web",
            "--out-dir", out_dir_abs.to_str().unwrap(),
            "--out-name", &out_name,
            wasm_artifact.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| WasmError {
            message: format!("Failed to run wasm-bindgen: {}. Install with: cargo install wasm-bindgen-cli", e),
        })?;
    if !bindgen_status.success() {
        return Err(WasmError { message: "wasm-bindgen failed".to_string() });
    }
    let js_name = format!("{}.js", stem);
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>{}</title></head>
<body>
<script type="module">
const CHUNK_B64 = "{}";
const chunk = Uint8Array.from(atob(CHUNK_B64), c => c.charCodeAt(0));
import init, {{ run }} from './{}';
await init();
run(chunk);
</script>
</body>
</html>
"#,
        stem, chunk_b64, js_name
    );
    let html_path = out_dir_abs.join(format!("{}.html", stem));
    std::fs::write(&html_path, html).map_err(|e| WasmError {
        message: format!("Cannot write {}: {}", html_path.display(), e),
    })?;
    println!("Built: {}_bg.wasm, {}.js, {}", stem, stem, html_path.display());
    Ok(())
}

/// Compile a Tish project for WebAssembly.
///
/// Produces:
/// - `{output}.wasm` — VM binary (runs your program)
/// - `{output}.js` — wasm-bindgen glue
/// - `{output}.html` — loader (open in browser)
///
/// Requires: `rustup target add wasm32-unknown-unknown`, `wasm-bindgen-cli`
pub fn compile_to_wasm(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    optimize: bool,
) -> Result<(), WasmError> {
    let (chunk, _) = resolve_and_compile_to_chunk(entry_path, project_root, optimize)?;
    emit_wasm_from_chunk(&chunk, output_path)
}

/// Compile a Tish project for Wasmtime/WASI.
///
/// Produces a single `{output}.wasm` with embedded bytecode. Run with:
/// `wasmtime {output}.wasm`
///
/// Requires: `rustup target add wasm32-wasip1`
pub fn compile_to_wasi(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    optimize: bool,
) -> Result<(), WasmError> {
    let (chunk, program) = resolve_and_compile_to_chunk(entry_path, project_root, optimize)?;
    if has_external_native_imports(&program) {
        return Err(WasmError {
            message: "WASI backend does not support external native imports (tish:egui, @scope/pkg). Built-in tish:fs, tish:http, tish:process are supported.".to_string(),
        });
    }
    let wasi_features = extract_native_import_features(&program);
    let chunk_bytes = serialize(&chunk);

    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main");
    let out_dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let out_dir_abs = out_dir
        .canonicalize()
        .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(out_dir)))
        .map_err(|e| WasmError {
            message: format!("Cannot resolve output dir: {}", e),
        })?;

    std::fs::create_dir_all(&out_dir_abs).map_err(|e| WasmError {
        message: format!("Cannot create output directory: {}", e),
    })?;

    let workspace_root = tishlang_build_utils::find_workspace_root().map_err(|e| WasmError {
        message: e,
    })?;

    // Create generated project: wasi_build/{stem}/
    let build_dir = out_dir_abs.join("wasi_build").join(stem);
    std::fs::create_dir_all(build_dir.join("src")).map_err(|e| WasmError {
        message: format!("Cannot create build dir: {}", e),
    })?;

    // Write chunk.bin
    std::fs::write(build_dir.join("chunk.bin"), &chunk_bytes).map_err(|e| WasmError {
        message: format!("Cannot write chunk: {}", e),
    })?;

    // Cargo.toml - path to tishlang_wasm_runtime from build_dir
    let runtime_path = workspace_root
        .join("crates")
        .join("tishlang_wasm_runtime");
    let runtime_path_str = runtime_path
        .canonicalize()
        .unwrap_or(runtime_path)
        .to_string_lossy()
        .replace('\\', "/");

    let features_str = if wasi_features.is_empty() {
        String::new()
    } else {
        format!(
            ", features = [{}]",
            wasi_features
                .iter()
                .map(|f| format!("{:?}", f))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let cargo_toml = format!(
        r#"[package]
name = "tish_wasi_{stem}"
version = "0.1.0"
edition = "2021"

[workspace]

[[bin]]
name = "tish_wasi_{stem}"
path = "src/main.rs"

[dependencies]
tishlang_wasm_runtime = {{ path = "{runtime_path_str}"{features_str} }}
"#,
        stem = stem,
        runtime_path_str = runtime_path_str,
        features_str = features_str
    );
    std::fs::write(build_dir.join("Cargo.toml"), cargo_toml).map_err(|e| WasmError {
        message: format!("Cannot write Cargo.toml: {}", e),
    })?;

    // main.rs
    let main_rs = r#"
fn main() {
    let chunk = include_bytes!("../chunk.bin");
    if let Err(e) = tishlang_wasm_runtime::run_wasi(chunk) {
        eprintln!("Runtime error: {}", e);
        std::process::exit(1);
    }
}
"#;
    std::fs::write(build_dir.join("src").join("main.rs"), main_rs).map_err(|e| WasmError {
        message: format!("Cannot write main.rs: {}", e),
    })?;

    // Build - use explicit target-dir so we know where the artifact is
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let bin_name = format!("tish_wasi_{}", stem);
    let target_dir = build_dir.join("target");
    let build_status = Command::new(&cargo)
        .current_dir(&build_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args([
            "build",
            "--target",
            "wasm32-wasip1",
            "--release",
        ])
        .status()
        .map_err(|e| WasmError {
            message: format!("Failed to run cargo: {}", e),
        })?;

    if !build_status.success() {
        return Err(WasmError {
            message: "Failed to build WASI binary. Run: rustup target add wasm32-wasip1"
                .to_string(),
        });
    }

    let wasm_artifact = target_dir
        .join("wasm32-wasip1")
        .join("release")
        .join(format!("{}.wasm", bin_name));

    if !wasm_artifact.exists() {
        return Err(WasmError {
            message: format!("WASI artifact not found: {}", wasm_artifact.display()),
        });
    }

    let final_wasm = out_dir_abs.join(format!("{}.wasm", stem));
    std::fs::copy(&wasm_artifact, &final_wasm).map_err(|e| WasmError {
        message: format!("Cannot copy wasm: {}", e),
    })?;

    println!(
        "Built: {} (run with: wasmtime {})",
        final_wasm.display(),
        final_wasm.display()
    );
    Ok(())
}

