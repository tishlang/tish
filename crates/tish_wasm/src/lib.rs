//! WebAssembly backend for Tish.
//!
//! Compiles Tish to bytecode, then produces a .wasm VM binary + loader.
//! The VM runs in the browser; your program runs as serialized bytecode.

use std::collections::BTreeSet;
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

/// Map CLI / import capability names to `tishlang_wasm_runtime` Cargo features for wasm32-wasip1.
/// The full `http` stack (tokio/socket2/…) does not build on WASI here; `http` maps to `promise`
/// so `Promise` / `await` work. `ws` is skipped for the same reason.
fn insert_wasi_runtime_cap(out: &mut BTreeSet<String>, cap: &str) {
    match cap {
        "http" => {
            out.insert("promise".to_string());
        }
        "ws" => {}
        "fs" | "process" | "promise" | "timers" | "regex" => {
            out.insert(cap.to_string());
        }
        _ => {}
    }
}

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
        let prog = merge_modules(modules)
            .map(|m| m.program)
            .map_err(|e| WasmError {
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
    gpu: bool,
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
    emit_wasm_from_chunk(&chunk, output_path, gpu)
}

/// The HTML loader emitted next to the wasm + JS glue. `gpu = false` imports `run` and calls
/// `run(chunk)` (plain VM). `gpu = true` (#277) imports `start`, bootstraps WebGPU into a `host`
/// env object, and calls `start(chunk, host)`. The GPU loader is a working default — edit
/// `buildHost()` to add app assets, pick a specific canvas, or change the surface configuration.
fn loader_html(stem: &str, chunk_b64: &str, gpu: bool) -> String {
    let js_name = format!("{}.js", stem);
    if gpu {
        format!(
            r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>{title}</title><style>html,body{{margin:0;height:100%}}#tish-canvas{{width:100vw;height:100vh;display:block}}</style></head>
<body>
<canvas id="tish-canvas"></canvas>
<script type="module">
const CHUNK_B64 = "{chunk}";
const chunk = Uint8Array.from(atob(CHUNK_B64), c => c.charCodeAt(0));
import init, {{ start }} from './{js}';

// The `host` global your tish program reads. Customize freely.
async function buildHost() {{
  if (!navigator.gpu) throw new Error("WebGPU not available (use a WebGPU-capable browser / context).");
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error("No WebGPU adapter.");
  const device = await adapter.requestDevice();
  const canvas = document.getElementById("tish-canvas");
  const dpr = self.devicePixelRatio || 1;
  canvas.width = Math.floor(canvas.clientWidth * dpr);
  canvas.height = Math.floor(canvas.clientHeight * dpr);
  const context = canvas.getContext("webgpu");
  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({{ device, format, alphaMode: "opaque" }});
  return {{ gpu: navigator.gpu, adapter, device, queue: device.queue, context, format, canvas, assets: {{}} }};
}}

try {{
  const host = await buildHost();
  await init();
  start(chunk, host);
}} catch (e) {{
  document.body.innerHTML = "<pre style='color:#c00;padding:1rem;font:14px monospace'>" + (e && e.message || e) + "</pre>";
  throw e;
}}
</script>
</body>
</html>
"#,
            title = stem,
            chunk = chunk_b64,
            js = js_name
        )
    } else {
        format!(
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
        )
    }
}

/// Build the wasm runtime + emit the JS/HTML loader for a serialized `chunk`.
///
/// `gpu = false` → `--features browser`, plain `run(chunk)` VM entry (the default `--target wasm`).
/// `gpu = true`  → `--features gpu`, the reflection WebGPU bridge ([`tishlang_wasm_runtime`]'s
/// `gpu.rs`) and a `start(chunk, host)` entry; the emitted HTML bootstraps WebGPU (adapter/device/
/// canvas/context) into the `host` global the tish program reads. (#277, `--target wasm-gpu`.)
fn emit_wasm_from_chunk(chunk: &Chunk, output_path: &Path, gpu: bool) -> Result<(), WasmError> {
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
    let workspace_root =
        tishlang_build_utils::find_workspace_root().map_err(|e| WasmError { message: e })?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let build_status = Command::new(&cargo)
        .current_dir(&workspace_root)
        .args([
            "build",
            "-p",
            "tishlang_wasm_runtime",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--features",
            if gpu { "gpu" } else { "browser" },
        ])
        .status()
        .map_err(|e| WasmError {
            message: format!("Failed to run cargo: {}", e),
        })?;
    if !build_status.success() {
        return Err(WasmError {
            message: "Failed to build wasm runtime. Run: rustup target add wasm32-unknown-unknown"
                .to_string(),
        });
    }
    let wasm_artifact =
        workspace_root.join("target/wasm32-unknown-unknown/release/tishlang_wasm_runtime.wasm");
    if !wasm_artifact.exists() {
        return Err(WasmError {
            message: format!("Wasm artifact not found: {}", wasm_artifact.display()),
        });
    }
    let wasm_bindgen = std::env::var("WASM_BINDGEN").unwrap_or_else(|_| "wasm-bindgen".to_string());
    let out_name = stem.to_string();
    let bindgen_status = Command::new(&wasm_bindgen)
        .args([
            "--target",
            "web",
            "--out-dir",
            out_dir_abs.to_str().unwrap(),
            "--out-name",
            &out_name,
            wasm_artifact.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| WasmError {
            message: format!(
                "Failed to run wasm-bindgen: {}. Install with: cargo install wasm-bindgen-cli",
                e
            ),
        })?;
    if !bindgen_status.success() {
        return Err(WasmError {
            message: "wasm-bindgen failed".to_string(),
        });
    }
    let html = loader_html(stem, &chunk_b64, gpu);
    let html_path = out_dir_abs.join(format!("{}.html", stem));
    std::fs::write(&html_path, html).map_err(|e| WasmError {
        message: format!("Cannot write {}: {}", html_path.display(), e),
    })?;
    println!(
        "Built: {}_bg.wasm, {}.js, {}",
        stem,
        stem,
        html_path.display()
    );
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
/// `gpu = true` targets the WebGPU runtime (`--features gpu`, `start(chunk, host)` entry); see
/// [`emit_wasm_from_chunk`]. (#277, exposed as `--target wasm-gpu`.)
pub fn compile_to_wasm(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    optimize: bool,
    gpu: bool,
) -> Result<(), WasmError> {
    let (chunk, _) = resolve_and_compile_to_chunk(entry_path, project_root, optimize)?;
    emit_wasm_from_chunk(&chunk, output_path, gpu)
}

/// Compile a Tish project to a raw serialized bytecode chunk.
///
/// Writes a single `{output}` file of the exact bytes that the wasm/WASI runtime entry points
/// (`start` / `run`) deserialize directly — the same chunk `--target wasm` embeds as base64 in
/// its generated HTML loader, but written raw with no VM binary, JS glue, or HTML wrapper. Lets a
/// host that already ships the VM runtime (e.g. a bundler) consume the bytecode without the
/// throwaway standalone build.
pub fn compile_to_bytecode(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    optimize: bool,
) -> Result<(), WasmError> {
    let (chunk, _) = resolve_and_compile_to_chunk(entry_path, project_root, optimize)?;
    let bytes = serialize(&chunk);
    if let Some(parent) = output_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| WasmError {
            message: format!("Cannot create output directory: {}", e),
        })?;
    }
    std::fs::write(output_path, &bytes).map_err(|e| WasmError {
        message: format!("Cannot write {}: {}", output_path.display(), e),
    })?;
    println!("Built: {} ({} bytes)", output_path.display(), bytes.len());
    Ok(())
}

/// Compile a Tish project for Wasmtime/WASI.
///
/// Produces a single `{output}.wasm` with embedded bytecode. Run with:
/// `wasmtime {output}.wasm`
///
/// Requires: `rustup target add wasm32-wasip1`
///
/// `capabilities` is the same capability list as `tish build --target native` (e.g. from
/// `native_build_features_from_cli`): merged with `import`-inferred features so globals like
/// `Promise` / `fetch` work without a top-level `import … from 'http'`.
pub fn compile_to_wasi(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_path: &Path,
    optimize: bool,
    capabilities: &[String],
) -> Result<(), WasmError> {
    let (chunk, program) = resolve_and_compile_to_chunk(entry_path, project_root, optimize)?;
    if has_external_native_imports(&program) {
        return Err(WasmError {
            message: "WASI backend does not support external native imports (tish:egui, @scope/pkg). Built-in tish:fs, tish:http, tish:process, tish:timers are supported.".to_string(),
        });
    }
    let mut wasi_feature_set: BTreeSet<String> = BTreeSet::new();
    for f in extract_native_import_features(&program) {
        insert_wasi_runtime_cap(&mut wasi_feature_set, f.as_str());
    }
    for f in capabilities {
        insert_wasi_runtime_cap(&mut wasi_feature_set, f.as_str());
    }
    // Many scripts use global setTimeout without `import` from timers.
    wasi_feature_set.insert("timers".to_string());

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

    let workspace_root =
        tishlang_build_utils::find_workspace_root().map_err(|e| WasmError { message: e })?;

    // Create generated project: wasi_build/{stem}/
    let build_dir = out_dir_abs.join("wasi_build").join(stem);
    std::fs::create_dir_all(build_dir.join("src")).map_err(|e| WasmError {
        message: format!("Cannot create build dir: {}", e),
    })?;

    // Write chunk.bin
    std::fs::write(build_dir.join("chunk.bin"), &chunk_bytes).map_err(|e| WasmError {
        message: format!("Cannot write chunk: {}", e),
    })?;

    // Cargo.toml - path to tishlang_wasm_runtime (crate in crates/tish_wasm_runtime)
    let runtime_path = workspace_root.join("crates").join("tish_wasm_runtime");
    let runtime_path_str = runtime_path
        .canonicalize()
        .unwrap_or(runtime_path)
        .to_string_lossy()
        .replace('\\', "/");

    let features_str = format!(
        ", features = [{}]",
        wasi_feature_set
            .iter()
            .map(|f| format!("{:?}", f))
            .collect::<Vec<_>>()
            .join(", ")
    );
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

    // Build into a SHARED target dir (one per host), not per-program. The wasi runtime + embedded
    // VM then compile ONCE and are reused by every wasi build; only each program's tiny main is
    // rebuilt. Without this each program left its own multi-GB `target/` and a full-suite sweep
    // would fill the disk (same issue fixed for cranelift; see full-backend-parity-plan.md A3).
    // cargo's target lock serializes concurrent builds safely.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let bin_name = format!("tish_wasi_{}", stem);
    let target_dir = std::env::temp_dir().join("tishlang_wasi_target");
    let build_status = Command::new(&cargo)
        .current_dir(&build_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["build", "--target", "wasm32-wasip1", "--release"])
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

#[cfg(test)]
mod tests {
    use super::loader_html;

    #[test]
    fn gpu_loader_uses_start_and_bootstraps_webgpu() {
        // #277: the wasm-gpu loader must call the `start(chunk, host)` entry (not `run`) and build
        // the WebGPU `host` env (adapter/device/context) the tish program reads.
        let html = loader_html("viz", "QkFTRTY0", true);
        assert!(html.contains("import init, { start }"), "imports start");
        assert!(html.contains("start(chunk, host)"), "calls start with host");
        assert!(html.contains("navigator.gpu.requestAdapter()"), "bootstraps adapter");
        assert!(html.contains("getContext(\"webgpu\")"), "configures the canvas context");
        assert!(html.contains("device, queue: device.queue, context, format, canvas"), "host shape");
        assert!(html.contains("from './viz.js'"), "imports the right glue");
        assert!(!html.contains("run(chunk)"), "must NOT use the plain run() entry");
        assert!(html.contains("QkFTRTY0"), "embeds the chunk");
    }

    #[test]
    fn plain_loader_uses_run() {
        let html = loader_html("app", "QkFTRTY0", false);
        assert!(html.contains("import init, { run }"), "imports run");
        assert!(html.contains("run(chunk)"), "calls run");
        assert!(!html.contains("start("), "no gpu start entry");
        assert!(!html.contains("requestAdapter"), "no webgpu bootstrap");
        assert!(html.contains("from './app.js'"), "imports the right glue");
    }
}
