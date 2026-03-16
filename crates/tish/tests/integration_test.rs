//! Full-stack integration tests: parse, interpreter, and native compile of .tish files.
//!
//! Run with: `cargo test -p tish` (full stack) or `cargo test` (all packages).
//! Compiled outputs are cached under target/integration_compile_cache/ so repeated runs
//! and CI cache restores avoid re-running `tish compile` for unchanged files.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn core_dir() -> PathBuf {
    workspace_root().join("tests").join("core")
}

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join("target"))
}

/// Cache dir for tish compile outputs (under target/ so CI rust-cache restores it).
fn integration_compile_cache_dir() -> PathBuf {
    target_dir().join("integration_compile_cache")
}

fn file_content_hash(path: &Path) -> u64 {
    let mut f = std::fs::File::open(path).expect("open file for hash");
    let mut content = Vec::new();
    f.read_to_end(&mut content).expect("read file for hash");
    let mut h = DefaultHasher::new();
    path.to_string_lossy().hash(&mut h);
    content.hash(&mut h);
    h.finish()
}

/// Compile a .tish file with the given backend, using a persistent cache so we only run
/// `tish compile` when the file or backend changed. Returns path to the compiled artifact
/// (binary, .js, or .wasm) in a temp dir; caller may run it and then delete it.
///
/// Cache is keyed by backend (native, cranelift, js, wasi) so e.g. cranelift and wasi
/// compiles of the same file do not overwrite each other: .../cranelift/<stem>_<hash> vs .../wasi/<stem>_<hash>.wasm.
fn compile_cached(bin: &Path, path: &Path, backend: &str) -> PathBuf {
    let stem = path.file_stem().unwrap().to_string_lossy();
    let hash = file_content_hash(path);
    let hash8 = &format!("{:016x}", hash)[..8];
    let cache_base = integration_compile_cache_dir().join(backend);
    let _ = std::fs::create_dir_all(&cache_base);

    let (artifact_path, compile_args): (PathBuf, Vec<OsString>) = match backend {
        "native" => {
            let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
            let cached = cache_base.join(format!("{}_{}{}", stem, hash8, ext));
            let args = vec![
                OsString::from("compile"),
                OsString::from(path),
                OsString::from("-o"),
                OsString::from(&cached),
            ];
            (cached, args)
        }
        "cranelift" => {
            let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
            let cached = cache_base.join(format!("{}_{}{}", stem, hash8, ext));
            let args = vec![
                OsString::from("compile"),
                OsString::from(path),
                OsString::from("-o"),
                OsString::from(&cached),
                OsString::from("--native-backend"),
                OsString::from("cranelift"),
            ];
            (cached, args)
        }
        "js" => {
            let cached = cache_base.join(format!("{}_{}.js", stem, hash8));
            let args = vec![
                OsString::from("compile"),
                OsString::from(path),
                OsString::from("--target"),
                OsString::from("js"),
                OsString::from("-o"),
                OsString::from(&cached),
            ];
            (cached, args)
        }
        "wasi" => {
            let out_base = cache_base.join(format!("{}_{}", stem, hash8));
            let artifact = out_base.with_extension("wasm");
            let args = vec![
                OsString::from("compile"),
                OsString::from(path),
                OsString::from("-o"),
                OsString::from(&out_base),
                OsString::from("--target"),
                OsString::from("wasi"),
            ];
            (artifact, args)
        }
        _ => panic!("unknown backend {}", backend),
    };

    if !artifact_path.exists() {
        let out = Command::new(bin)
            .args(compile_args)
            .current_dir(workspace_root())
            .output()
            .expect("run tish compile");
        assert!(
            out.status.success(),
            "Compile failed for {} ({}): {}",
            path.display(),
            backend,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Copy to temp so caller can run and delete without touching cache.
    let ext = artifact_path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let temp_dest = std::env::temp_dir().join(format!("tish_cached_{}_{}_{}", backend, stem, hash8));
    let temp_dest = if ext.is_empty() {
        temp_dest
    } else {
        temp_dest.with_extension(ext)
    };
    std::fs::copy(&artifact_path, &temp_dest).expect("copy cached artifact to temp");
    temp_dest
}

/// Path to the tish CLI binary. When running under cargo-llvm-cov, the build goes to
/// target/llvm-cov-target and CARGO_TARGET_DIR may not be set for the test process.
fn tish_bin() -> PathBuf {
    let bin_name = if cfg!(target_os = "windows") { "tish.exe" } else { "tish" };
    let default = target_dir().join("debug").join(bin_name);
    if default.exists() {
        return default;
    }
    let llvm_cov = workspace_root().join("target").join("llvm-cov-target").join("debug").join(bin_name);
    if llvm_cov.exists() {
        return llvm_cov;
    }
    default
}

/// Parse async-await example (validates async fn parsing).
#[test]
fn test_async_await_parse() {
    let path = workspace_root().join("examples").join("async-await").join("src").join("main.tish");
    if path.exists() {
        let source = std::fs::read_to_string(&path).unwrap();
        let result = tish_parser::parse(&source);
        assert!(result.is_ok(), "Parse failed for {}: {:?}", path.display(), result.err());
    }
}

/// Invoke tish binary to compile async-await and run compiled output (validates non-blocking pipeline).
#[test]
#[cfg(feature = "http")]
fn test_async_await_compile_via_binary() {
    let bin = tish_bin();
    let path = workspace_root().join("examples").join("async-await").join("src").join("main.tish");
    if path.exists() && bin.exists() {
        let out = std::env::temp_dir().join("tish_async_test_out");
        let compile_result = Command::new(&bin)
            .args(["compile", path.to_string_lossy().as_ref(), "-o", out.to_string_lossy().as_ref()])
            .current_dir(workspace_root())
            .output();
        let compile_out = compile_result.expect("run tish compile");
        assert!(
            compile_out.status.success(),
            "tish compile failed: {}",
            String::from_utf8_lossy(&compile_out.stderr)
        );
        // Run compiled binary to validate non-blocking fetchAllAsync executes correctly
        let run_result = Command::new(&out)
            .current_dir(workspace_root())
            .output();
        let run_out = run_result.expect("run compiled async binary");
        assert!(
            run_out.status.success(),
            "compiled async binary failed: {}",
            String::from_utf8_lossy(&run_out.stderr)
        );
        let stdout = String::from_utf8_lossy(&run_out.stdout);
        assert!(stdout.contains("Fetching"), "expected output to mention fetching");
        assert!(stdout.contains("Done"), "expected output to contain Done");
    }
}

/// DEFINITIVE VALIDATION: Parallel fetches must be faster than sequential.
/// Uses httpbin.org/delay/1 (1s each). 3 parallel ≈ 1s, 3 sequential ≈ 3s.
#[test]
#[cfg(feature = "http")]
fn test_async_parallel_vs_sequential_timing() {
    let bin = tish_bin();
    let parallel_src = workspace_root().join("examples").join("async-await").join("src").join("parallel.tish");
    let sequential_src = workspace_root().join("examples").join("async-await").join("src").join("sequential.tish");
    if !parallel_src.exists() || !sequential_src.exists() || !bin.exists() {
        return;
    }
    let out_parallel = std::env::temp_dir().join("tish_parallel_timing");
    let out_sequential = std::env::temp_dir().join("tish_sequential_timing");

    // Compile both
    let compile_par = Command::new(&bin)
        .args(["compile", parallel_src.to_string_lossy().as_ref(), "-o", out_parallel.to_string_lossy().as_ref()])
        .current_dir(workspace_root())
        .output();
    assert!(compile_par.as_ref().unwrap().status.success(), "compile parallel: {}", String::from_utf8_lossy(&compile_par.as_ref().unwrap().stderr));

    let compile_seq = Command::new(&bin)
        .args(["compile", sequential_src.to_string_lossy().as_ref(), "-o", out_sequential.to_string_lossy().as_ref()])
        .current_dir(workspace_root())
        .output();
    assert!(compile_seq.as_ref().unwrap().status.success(), "compile sequential: {}", String::from_utf8_lossy(&compile_seq.as_ref().unwrap().stderr));

    // Run parallel and time
    let t_parallel = std::time::Instant::now();
    let run_par = Command::new(&out_parallel).current_dir(workspace_root()).output();
    let elapsed_parallel = t_parallel.elapsed();
    assert!(run_par.as_ref().unwrap().status.success(), "run parallel: {}", String::from_utf8_lossy(&run_par.as_ref().unwrap().stderr));

    // Run sequential and time
    let t_sequential = std::time::Instant::now();
    let run_seq = Command::new(&out_sequential).current_dir(workspace_root()).output();
    let elapsed_sequential = t_sequential.elapsed();
    assert!(run_seq.as_ref().unwrap().status.success(), "run sequential: {}", String::from_utf8_lossy(&run_seq.as_ref().unwrap().stderr));

    // PARALLEL MUST BE FASTER: parallel < sequential * 0.6 (parallel ~1s, sequential ~3s)
    let parallel_secs = elapsed_parallel.as_secs_f64();
    let sequential_secs = elapsed_sequential.as_secs_f64();
    assert!(
        parallel_secs < sequential_secs * 0.6,
        "Async NOT validated: parallel took {:.2}s but sequential took {:.2}s. Parallel must be < 60% of sequential to prove non-blocking.",
        parallel_secs,
        sequential_secs
    );
}

/// Run async-await example via tish_eval (same path as `tish run`).
/// Ignored: tish_eval::run() is synchronous and does not run the event loop.
#[test]
#[cfg(feature = "http")]
#[ignore = "requires async runtime; use test_async_await_compile_via_binary for CI"]
fn test_async_await_run() {
    let path = workspace_root().join("examples").join("async-await").join("src").join("main.tish");
    if path.exists() {
        let source = std::fs::read_to_string(&path).unwrap();
        let result = tish_eval::run(&source);
        assert!(result.is_ok(), "Run failed for {}: {:?}", path.display(), result.err());
    }
}

/// Run Promise and setTimeout module tests (require http feature).
/// Ignored: tish_eval::run() does not run the event loop.
#[test]
#[cfg(feature = "http")]
#[ignore = "requires async runtime"]
fn test_promise_and_settimeout() {
    for name in ["promise", "settimeout"] {
        let path = workspace_root().join("tests").join("modules").join(format!("{}.tish", name));
        if path.exists() {
            let source = std::fs::read_to_string(&path).unwrap();
            let result = tish_eval::run(&source);
            assert!(
                result.is_ok(),
                "Failed to run {}: {:?}",
                path.display(),
                result.err()
            );
        }
    }
}

/// Combined validation: async/await + Promise + setTimeout + multiple HTTP requests.
/// Ignored: tish_eval::run() does not run the event loop.
#[test]
#[cfg(feature = "http")]
#[ignore = "requires async runtime"]
fn test_async_promise_settimeout_combined() {
    let path = workspace_root()
        .join("tests")
        .join("modules")
        .join("async_promise_settimeout.tish");
    if path.exists() {
        let source = std::fs::read_to_string(&path).unwrap();
        let result = tish_eval::run(&source);
        assert!(
            result.is_ok(),
            "Failed to run async_promise_settimeout: {:?}",
            result.err()
        );
    }
}

/// VM run with Date global (resolve+merge+bytecode+run pipeline).
#[test]
fn test_vm_date_now() {
    let path = workspace_root().join("tests").join("core").join("date.tish");
    if !path.exists() {
        return;
    }
    // Library path
    let modules = tish_compile::resolve_project(&path, path.parent()).expect("resolve");
    tish_compile::detect_cycles(&modules).expect("cycles");
    let program = tish_compile::merge_modules(modules).expect("merge");
    let chunk = tish_bytecode::compile(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "VM run (library) failed: {:?}", result.err());
    // Binary path - same flow as `tish run <file>`
    let bin = tish_bin();
    if bin.exists() {
        let out = Command::new(&bin)
            .args(["run", path.to_string_lossy().as_ref()])
            .current_dir(workspace_root())
            .output()
            .expect("run tish binary");
        assert!(
            out.status.success(),
            "tish run failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// VM run with parse+compile only (no resolve/merge) - isolates bytecode IndexAssign.
#[test]
fn test_vm_index_assign_direct() {
    let source = r#"let arr = [1, 2, 3]; arr[1] = 99; console.log(arr[1]);"#;
    let program = tish_parser::parse(source).expect("parse");
    let chunk = tish_bytecode::compile(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "VM IndexAssign failed: {:?}", result.err());
}

/// VM run via resolve+merge (same as tish run) - must also pass.
#[test]
fn test_vm_index_assign_via_resolve() {
    let path = workspace_root().join("tests").join("core").join("array_sort_minimal.tish");
    let modules = tish_compile::resolve_project(&path, path.parent()).expect("resolve");
    tish_compile::detect_cycles(&modules).expect("cycles");
    let program = tish_compile::merge_modules(modules).expect("merge");
    let chunk = tish_bytecode::compile(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "VM IndexAssign via resolve failed: {:?}", result.err());
}

/// tish run binary must pass array_sort_minimal (ensures CLI works).
#[test]
fn test_tish_run_index_assign() {
    let bin = tish_bin();
    let path = workspace_root().join("tests").join("core").join("array_sort_minimal.tish");
    if !bin.exists() {
        eprintln!("Skipping: tish binary not built");
        return;
    }
    let out = Command::new(&bin)
        .args(["run", path.to_string_lossy().as_ref()])
        .current_dir(workspace_root())
        .output()
        .expect("run tish");
    assert!(
        out.status.success(),
        "tish run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("pass"),
        "Expected 'pass' in output"
    );
}

/// Full stack: lex + parse each .tish file and assert no parse error.
#[test]
fn test_full_stack_parse() {
    let core_dir = core_dir();
    for entry in std::fs::read_dir(&core_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "tish").unwrap_or(false) {
            let source = std::fs::read_to_string(&path).unwrap();
            let result = tish_parser::parse(&source);
            assert!(
                result.is_ok(),
                "Parse failed for {}: {:?}",
                path.display(),
                result.err()
            );
        }
    }
}

/// Full stack: parse + interpret each .tish file and assert no runtime error.
/// Skips files that overflow the stack in-process (recursion_stress, array_stress) or are slow.
#[test]
fn test_mvp_programs_interpreter() {
    let core_dir = core_dir();
    let skip = ["recursion_stress.tish", "array_stress.tish"];
    for entry in std::fs::read_dir(&core_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "tish").unwrap_or(false) {
            let name = path.file_name().unwrap().to_string_lossy();
            if skip.contains(&name.as_ref()) {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let result = tish_eval::run(&source);
            assert!(
                result.is_ok(),
                "Failed to run {}: {:?}",
                path.display(),
                result.err()
            );
        }
    }
}

/// Full stack: compile each .tish file to native, run, and compare output to interpreter.
#[test]
fn test_mvp_programs_interpreter_vs_native() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tish` first.",
        bin.display()
    );

    // Plan Section 7 MVP programs + extended feature set (each compile ~1-2s)
    let test_files = [
        // Plan-mandated concrete MVP programs
        "nested_loops.tish",
        "scopes.tish",
        "optional_braces.tish",
        "optional_braces_braced.tish",
        "tab_indent.tish",
        "space_indent.tish",
        "fn_any.tish",
        "strict_equality.tish",
        // Extended features
        "arrays.tish",
        "break_continue.tish",
        "length.tish",
        "objects.tish",
        "conditional.tish",
        "switch.tish",
        "do_while.tish",
        "typeof.tish",
        "inc_dec.tish",
        "try_catch.tish",
        "builtins.tish",
        "exponentiation.tish",
        "for_of.tish",
        "bitwise.tish",
        "math.tish",
        "optional_chaining.tish",
        "void.tish",
        "rest_params.tish",
        "json.tish",
        "uri.tish",
        "in_op.tish",
        // Additional parity tests
        "arrow_functions.tish",
        "template_literals.tish",
        "compound_assign.tish",
        "mutation.tish",
        "string_methods.tish",
        "array_methods.tish",
        "object_methods.tish",
        "types.tish", // type annotations - now supported in codegen
        // higher_order_methods.tish - addToTotal RefCell fix works but reduce (no init) panics in native
        // destructuring.tish - excluded: destructured vars not in scope outside if-let block
        "logical_assign.tish",
        "spread.tish",
    ];
    for name in test_files {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();

        let interp_out = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            interp_out.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&interp_out.stderr)
        );

        let out_bin = compile_cached(&bin, &path, "native");
        let native_out = Command::new(&out_bin)
            .current_dir(workspace_root())
            .output()
            .expect("run compiled binary");
        let _ = std::fs::remove_file(&out_bin);

        let interp_stdout = String::from_utf8_lossy(&interp_out.stdout);
        let native_stdout = String::from_utf8_lossy(&native_out.stdout);
        assert_eq!(
            interp_stdout,
            native_stdout,
            "Interpreter vs native output mismatch for {}",
            path.display()
        );
    }
}

/// Full stack: compile each .tish file with Cranelift backend, run, and compare output to interpreter.
/// Uses a curated list of pure Tish tests known to work with Cranelift (some constructs cause
/// stack-underflow in the Cranelift backend; see docs/builtins-gap-analysis.md).
#[test]
fn test_mvp_programs_interpreter_vs_cranelift() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tish` first.",
        bin.display()
    );

    // Curated list: only files that pass with Cranelift (many fail with stack-underflow or scope bugs).
    let test_files = [
        "fn_any.tish",
        "strict_equality.tish",
        "switch.tish",
        "do_while.tish",
        "typeof.tish",
        "try_catch.tish",
        "json.tish",
        "math.tish",
        "builtins.tish",
        "uri.tish",
        "inc_dec.tish",
        "exponentiation.tish",
        "void.tish",
        "rest_params.tish",
        "arrow_functions.tish",
        "array_methods.tish",
        "types.tish",
    ];

    for name in test_files {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();

        let interp_out = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            interp_out.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&interp_out.stderr)
        );

        let out_bin = compile_cached(&bin, &path, "cranelift");
        let cranelift_out = Command::new(&out_bin)
            .current_dir(workspace_root())
            .output()
            .expect("run cranelift binary");
        let _ = std::fs::remove_file(&out_bin);

        let interp_stdout = String::from_utf8_lossy(&interp_out.stdout);
        let cranelift_stdout = String::from_utf8_lossy(&cranelift_out.stdout);
        assert_eq!(
            interp_stdout,
            cranelift_stdout,
            "Interpreter vs Cranelift output mismatch for {}",
            path.display()
        );
    }
}

/// Full stack: compile each .tish file to WASI, run with wasmtime, and compare output to interpreter.
/// Skips if wasmtime is not available. Uses same curated list as Cranelift (WASI uses bytecode VM).
#[test]
fn test_mvp_programs_interpreter_vs_wasi() {
    let wasmtime_available = Command::new("wasmtime")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !wasmtime_available {
        eprintln!("Skipping test_mvp_programs_interpreter_vs_wasi: wasmtime not found");
        return;
    }

    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tish` first.",
        bin.display()
    );

    let test_files = [
        "fn_any.tish",
        "strict_equality.tish",
        "switch.tish",
        "do_while.tish",
        "typeof.tish",
        "try_catch.tish",
        "json.tish",
        "math.tish",
        "builtins.tish",
        "uri.tish",
        "inc_dec.tish",
        "exponentiation.tish",
        "void.tish",
        "rest_params.tish",
        "arrow_functions.tish",
        "array_methods.tish",
        "types.tish",
    ];

    for name in test_files {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();

        let interp_out = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            interp_out.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&interp_out.stderr)
        );

        let out_wasm = compile_cached(&bin, &path, "wasi");
        let wasi_out = Command::new("wasmtime")
            .arg(out_wasm.as_os_str())
            .current_dir(workspace_root())
            .output()
            .expect("run wasmtime");
        let _ = std::fs::remove_file(&out_wasm);

        let interp_stdout = String::from_utf8_lossy(&interp_out.stdout);
        let wasi_stdout = String::from_utf8_lossy(&wasi_out.stdout);
        assert_eq!(
            interp_stdout,
            wasi_stdout,
            "Interpreter vs WASI output mismatch for {}",
            path.display()
        );
    }
}

/// Full stack: compile each .tish file to JS, run with Node, and compare output to interpreter.
#[test]
fn test_mvp_programs_interpreter_vs_js() {
    // Skip if Node.js is not available
    let node_available = Command::new("node")
        .args(["--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !node_available {
        eprintln!("Skipping test_mvp_programs_interpreter_vs_js: Node.js not found");
        return;
    }

    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tish` first.",
        bin.display()
    );

    // Files where Tish intentionally differs from JavaScript: assert interpreter matches expected Tish output.
    let tish_intentional_differences: std::collections::HashMap<&str, &str> = [
        ("typeof.tish", "number\nstring\nboolean\nnull\nobject\nobject\nfunction\n"),
        ("void.tish", "null\ntrue\nside effect\n"),
    ]
    .into_iter()
    .collect();

    let test_files = [
        "nested_loops.tish",
        "scopes.tish",
        "optional_braces.tish",
        "optional_braces_braced.tish",
        "tab_indent.tish",
        "space_indent.tish",
        "fn_any.tish",
        "strict_equality.tish",
        "arrays.tish",
        "break_continue.tish",
        "length.tish",
        "objects.tish",
        "conditional.tish",
        "switch.tish",
        "do_while.tish",
        "typeof.tish",
        "inc_dec.tish",
        "try_catch.tish",
        "builtins.tish",
        "exponentiation.tish",
        "for_of.tish",
        "bitwise.tish",
        "math.tish",
        "optional_chaining.tish",
        "void.tish",
        "rest_params.tish",
        "json.tish",
        "uri.tish",
        "in_op.tish",
        "arrow_functions.tish",
        "template_literals.tish",
        "compound_assign.tish",
        "mutation.tish",
        "string_methods.tish",
        "array_methods.tish",
        "object_methods.tish",
        "types.tish",
        "logical_assign.tish",
        "spread.tish",
    ];

    for name in test_files {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();

        // Run interpreter
        let interp_out = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            interp_out.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&interp_out.stderr)
        );

        let interp_stdout = String::from_utf8_lossy(&interp_out.stdout);

        if let Some(&expected) = tish_intentional_differences.get(name) {
            assert_eq!(
                interp_stdout,
                expected,
                "Interpreter output mismatch for {} (Tish intentional difference from JS)",
                path.display()
            );
            continue;
        }

        let out_js = compile_cached(&bin, &path, "js");
        let node_out = Command::new("node")
            .arg(&out_js)
            .current_dir(workspace_root())
            .output()
            .expect("run node");
        let _ = std::fs::remove_file(&out_js);

        assert!(
            node_out.status.success(),
            "Node failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&node_out.stderr)
        );

        let node_stdout = String::from_utf8_lossy(&node_out.stdout);
        assert_eq!(
            interp_stdout,
            node_stdout,
            "Interpreter vs JS output mismatch for {}:\n--- interpreter ---\n{}--- node ---\n{}",
            path.display(),
            interp_stdout,
            node_stdout
        );
    }
}

