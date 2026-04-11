//! Full-stack integration tests: run .tish files with interpreter or each backend and compare
//! stdout to static expected files (e.g. `fn_any.tish.expected`).
//!
//! - Run: `cargo test -p tishlang` (or `cargo nextest run -p tishlang`).
//! - Generate/update expected files: `REGENERATE_EXPECTED=1 cargo test -p tishlangtest_mvp_programs_interpreter`
//!   then commit the new/updated `tests/core/*.tish.expected` files.
//! - Compiled outputs are cached under `target/integration_compile_cache/` per backend.

use std::collections::hash_map::DefaultHasher;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use rayon::prelude::*;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn core_dir() -> PathBuf {
    workspace_root().join("tests").join("core")
}

/// Path to the static expected stdout for a .tish file (e.g. fn_any.tish -> fn_any.tish.expected).
fn expected_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.expected",
        path.file_name().unwrap().to_string_lossy()
    ))
}

/// Read static expected stdout for a test file. Returns None if the file does not exist.
fn get_expected(path: &Path) -> Option<String> {
    let p = expected_path(path);
    std::fs::read_to_string(&p).ok()
}

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join("target"))
}

/// Cache dir for tish build outputs (under target/ so CI rust-cache restores it).
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
/// `tish build` when the file or backend changed. Returns path to the compiled artifact
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
            let ext = if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            };
            let cached = cache_base.join(format!("{}_{}{}", stem, hash8, ext));
            let args = vec![
                OsString::from("build"),
                OsString::from(path),
                OsString::from("-o"),
                OsString::from(&cached),
            ];
            (cached, args)
        }
        "cranelift" => {
            let ext = if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            };
            let cached = cache_base.join(format!("{}_{}{}", stem, hash8, ext));
            let args = vec![
                OsString::from("build"),
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
                OsString::from("build"),
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
                OsString::from("build"),
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
            .expect("run tish build");
        assert!(
            out.status.success(),
            "Compile failed for {} ({}): {}",
            path.display(),
            backend,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Copy to temp so caller can run and delete without touching cache.
    let ext = artifact_path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let temp_dest =
        std::env::temp_dir().join(format!("tish_cached_{}_{}_{}", backend, stem, hash8));
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
    let bin_name = if cfg!(target_os = "windows") {
        "tish.exe"
    } else {
        "tish"
    };
    let default = target_dir().join("debug").join(bin_name);
    if default.exists() {
        return default;
    }
    let llvm_cov = workspace_root()
        .join("target")
        .join("llvm-cov-target")
        .join("debug")
        .join(bin_name);
    if llvm_cov.exists() {
        return llvm_cov;
    }
    default
}

/// tish -V and --version print the version.
#[test]
fn test_tish_version_flag() {
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found. Run `cargo build -p tishlang` first."
    );
    let out = Command::new(&bin).arg("-V").output().expect("run tish -V");
    assert!(
        out.status.success(),
        "tish -V failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "tish -V should print version {}; got: {}",
        env!("CARGO_PKG_VERSION"),
        stdout
    );
    let out2 = Command::new(&bin)
        .arg("--version")
        .output()
        .expect("run tish --version");
    assert!(out2.status.success());
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout2.contains(env!("CARGO_PKG_VERSION")),
        "tish --version should print version"
    );
}

/// Parse async-await example (validates async fn parsing).
#[test]
fn test_async_await_parse() {
    let path = workspace_root()
        .join("examples")
        .join("async-await")
        .join("src")
        .join("main.tish");
    if path.exists() {
        let source = std::fs::read_to_string(&path).unwrap();
        let result = tishlang_parser::parse(&source);
        assert!(
            result.is_ok(),
            "Parse failed for {}: {:?}",
            path.display(),
            result.err()
        );
    }
}

/// Invoke tish binary to compile async-await and run compiled output (validates non-blocking pipeline).
#[test]
#[cfg(feature = "http")]
fn test_async_await_compile_via_binary() {
    let bin = tish_bin();
    let path = workspace_root()
        .join("examples")
        .join("async-await")
        .join("src")
        .join("main.tish");
    if path.exists() && bin.exists() {
        let out = std::env::temp_dir().join("tish_async_test_out");
        let compile_result = Command::new(&bin)
            .args([
                "build",
                path.to_string_lossy().as_ref(),
                "-o",
                out.to_string_lossy().as_ref(),
            ])
            .current_dir(workspace_root())
            .output();
        let compile_out = compile_result.expect("run tish build");
        assert!(
            compile_out.status.success(),
            "tish build failed: {}",
            String::from_utf8_lossy(&compile_out.stderr)
        );
        // Run compiled binary to validate non-blocking fetchAll executes correctly
        let run_result = Command::new(&out).current_dir(workspace_root()).output();
        let run_out = run_result.expect("run compiled async binary");
        assert!(
            run_out.status.success(),
            "compiled async binary failed: {}",
            String::from_utf8_lossy(&run_out.stderr)
        );
        let stdout = String::from_utf8_lossy(&run_out.stdout);
        assert!(
            stdout.contains("Fetching"),
            "expected output to mention fetching"
        );
        assert!(stdout.contains("Done"), "expected output to contain Done");
    }
}

/// DEFINITIVE VALIDATION: Parallel fetches must be faster than sequential.
/// Uses httpbin.org/delay/1 (1s each). 3 parallel ≈ 1s, 3 sequential ≈ 3s.
#[test]
#[cfg(feature = "http")]
#[ignore = "timing and network sensitive; run manually: cargo test test_async_parallel_vs_sequential_timing -p tishlang--features http -- --ignored"]
fn test_async_parallel_vs_sequential_timing() {
    let bin = tish_bin();
    let parallel_src = workspace_root()
        .join("examples")
        .join("async-await")
        .join("src")
        .join("parallel.tish");
    let sequential_src = workspace_root()
        .join("examples")
        .join("async-await")
        .join("src")
        .join("sequential.tish");
    if !parallel_src.exists() || !sequential_src.exists() || !bin.exists() {
        return;
    }
    let out_parallel = std::env::temp_dir().join("tish_parallel_timing");
    let out_sequential = std::env::temp_dir().join("tish_sequential_timing");

    // Compile both
    let compile_par = Command::new(&bin)
        .args([
            "build",
            parallel_src.to_string_lossy().as_ref(),
            "-o",
            out_parallel.to_string_lossy().as_ref(),
        ])
        .current_dir(workspace_root())
        .output();
    assert!(
        compile_par.as_ref().unwrap().status.success(),
        "compile parallel: {}",
        String::from_utf8_lossy(&compile_par.as_ref().unwrap().stderr)
    );

    let compile_seq = Command::new(&bin)
        .args([
            "build",
            sequential_src.to_string_lossy().as_ref(),
            "-o",
            out_sequential.to_string_lossy().as_ref(),
        ])
        .current_dir(workspace_root())
        .output();
    assert!(
        compile_seq.as_ref().unwrap().status.success(),
        "compile sequential: {}",
        String::from_utf8_lossy(&compile_seq.as_ref().unwrap().stderr)
    );

    // Run parallel and time
    let t_parallel = std::time::Instant::now();
    let run_par = Command::new(&out_parallel)
        .current_dir(workspace_root())
        .output();
    let elapsed_parallel = t_parallel.elapsed();
    assert!(
        run_par.as_ref().unwrap().status.success(),
        "run parallel: {}",
        String::from_utf8_lossy(&run_par.as_ref().unwrap().stderr)
    );

    // Run sequential and time
    let t_sequential = std::time::Instant::now();
    let run_seq = Command::new(&out_sequential)
        .current_dir(workspace_root())
        .output();
    let elapsed_sequential = t_sequential.elapsed();
    assert!(
        run_seq.as_ref().unwrap().status.success(),
        "run sequential: {}",
        String::from_utf8_lossy(&run_seq.as_ref().unwrap().stderr)
    );

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

/// Run async-await example via tishlang_eval (same path as `tish run`).
/// Ignored: tishlang_eval::run() is synchronous and does not run the event loop.
#[test]
#[cfg(feature = "http")]
#[ignore = "requires async runtime; use test_async_await_compile_via_binary for CI"]
fn test_async_await_run() {
    let path = workspace_root()
        .join("examples")
        .join("async-await")
        .join("src")
        .join("main.tish");
    if path.exists() {
        let source = std::fs::read_to_string(&path).unwrap();
        let result = tishlang_eval::run(&source);
        assert!(
            result.is_ok(),
            "Run failed for {}: {:?}",
            path.display(),
            result.err()
        );
    }
}

/// Run Promise and setTimeout module tests (require http feature).
/// Ignored: tishlang_eval::run() does not run the event loop.
#[test]
#[cfg(feature = "http")]
#[ignore = "requires async runtime"]
fn test_promise_and_settimeout() {
    for name in ["promise", "settimeout"] {
        let path = workspace_root()
            .join("tests")
            .join("modules")
            .join(format!("{}.tish", name));
        if path.exists() {
            let source = std::fs::read_to_string(&path).unwrap();
            let result = tishlang_eval::run(&source);
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
/// Ignored: tishlang_eval::run() does not run the event loop.
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
        let result = tishlang_eval::run(&source);
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
    let path = workspace_root()
        .join("tests")
        .join("core")
        .join("date.tish");
    if !path.exists() {
        return;
    }
    // Library path
    let modules = tishlang_compile::resolve_project(&path, path.parent()).expect("resolve");
    tishlang_compile::detect_cycles(&modules).expect("cycles");
    let program = tishlang_compile::merge_modules(modules).expect("merge");
    let chunk = tishlang_bytecode::compile(&program).expect("compile");
    let result = tishlang_vm::run(&chunk);
    assert!(
        result.is_ok(),
        "VM run (library) failed: {:?}",
        result.err()
    );
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
    let program = tishlang_parser::parse(source).expect("parse");
    let chunk = tishlang_bytecode::compile(&program).expect("compile");
    let result = tishlang_vm::run(&chunk);
    assert!(result.is_ok(), "VM IndexAssign failed: {:?}", result.err());
}

/// VM run via resolve+merge (same as tish run) - must also pass.
#[test]
fn test_vm_index_assign_via_resolve() {
    let path = workspace_root()
        .join("tests")
        .join("core")
        .join("array_sort_minimal.tish");
    let modules = tishlang_compile::resolve_project(&path, path.parent()).expect("resolve");
    tishlang_compile::detect_cycles(&modules).expect("cycles");
    let program = tishlang_compile::merge_modules(modules).expect("merge");
    let chunk = tishlang_bytecode::compile(&program).expect("compile");
    let result = tishlang_vm::run(&chunk);
    assert!(
        result.is_ok(),
        "VM IndexAssign via resolve failed: {:?}",
        result.err()
    );
}

/// tish run binary must pass array_sort_minimal (ensures CLI works).
#[test]
fn test_tish_run_index_assign() {
    let bin = tish_bin();
    let path = workspace_root()
        .join("tests")
        .join("core")
        .join("array_sort_minimal.tish");
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
            let result = tishlang_parser::parse(&source);
            assert!(
                result.is_ok(),
                "Parse failed for {}: {:?}",
                path.display(),
                result.err()
            );
        }
    }
}

/// Shared list of MVP test files used for static comparison (interpreter and native).
const MVP_TEST_FILES: &[&str] = &[
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
    "fn_param_destructuring.tish",
];

/// Run each .tish file with interpreter and compare stdout to static expected.
/// Set REGENERATE_EXPECTED=1 to write .expected files from interpreter output (run once, then commit).
#[test]
fn test_mvp_programs_interpreter() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    let regenerate = std::env::var("REGENERATE_EXPECTED").as_deref() == Ok("1");
    for name in MVP_TEST_FILES {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();
        let out = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            out.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if regenerate {
            std::fs::write(expected_path(&path), &stdout).expect("write expected");
        } else {
            let expected = get_expected(&path).unwrap_or_else(|| {
                panic!(
                    "missing expected file for {}; run with REGENERATE_EXPECTED=1 to generate",
                    path.display()
                )
            });
            assert_eq!(
                stdout,
                expected,
                "Interpreter output mismatch for {}",
                path.display()
            );
        }
    }
}

/// Default bytecode VM must match the tree-walking interpreter for every MVP program.
#[test]
fn test_mvp_programs_interp_vm_stdout_parity() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    for name in MVP_TEST_FILES {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy();
        let out_interp = Command::new(&bin)
            .args(["run", path_str.as_ref(), "--backend", "interp"])
            .current_dir(workspace_root())
            .output()
            .expect("run tish interpreter");
        assert!(
            out_interp.status.success(),
            "Interpreter failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&out_interp.stderr)
        );
        let out_vm = Command::new(&bin)
            .args(["run", path_str.as_ref()])
            .current_dir(workspace_root())
            .output()
            .expect("run tish VM");
        assert!(
            out_vm.status.success(),
            "VM failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&out_vm.stderr)
        );
        let s_interp = String::from_utf8_lossy(&out_interp.stdout);
        let s_vm = String::from_utf8_lossy(&out_vm.stdout);
        assert_eq!(
            s_interp,
            s_vm,
            "interp vs VM stdout mismatch for {}",
            path.display()
        );
    }
}

/// Compile each .tish file to native, run, and compare stdout to static expected (parallelized).
#[test]
fn test_mvp_programs_native() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    let errors: Vec<String> = MVP_TEST_FILES
        .par_iter()
        .filter_map(|name| {
            let path = core_dir.join(name);
            if !path.exists() {
                return None;
            }
            let expected = match get_expected(&path) {
                Some(e) => e,
                None => return Some(format!("missing expected: {}", path.display())),
            };
            let out_bin = compile_cached(&bin, &path, "native");
            let out = match Command::new(&out_bin)
                .current_dir(workspace_root())
                .output()
            {
                Ok(o) => o,
                Err(e) => {
                    let _ = std::fs::remove_file(&out_bin);
                    return Some(format!("{}: run failed: {}", path.display(), e));
                }
            };
            let _ = std::fs::remove_file(&out_bin);
            if !out.status.success() {
                return Some(format!(
                    "{}: {}",
                    path.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout != expected {
                return Some(format!("{}: output mismatch", path.display()));
            }
            None
        })
        .collect();
    assert!(errors.is_empty(), "native failures:\n{}", errors.join("\n"));
}

/// Curated list: files that pass with Cranelift (some constructs cause stack-underflow; see docs/builtins-gap-analysis.md).
const CRANELIFT_TEST_FILES: &[&str] = &[
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

/// Compile each .tish file with Cranelift backend, run, and compare stdout to static expected (parallelized).
#[test]
fn test_mvp_programs_cranelift() {
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    let errors: Vec<String> = CRANELIFT_TEST_FILES
        .par_iter()
        .filter_map(|name| {
            let path = core_dir.join(name);
            if !path.exists() {
                return None;
            }
            let expected = match get_expected(&path) {
                Some(e) => e,
                None => return Some(format!("missing expected: {}", path.display())),
            };
            let out_bin = compile_cached(&bin, &path, "cranelift");
            let out = match Command::new(&out_bin)
                .current_dir(workspace_root())
                .output()
            {
                Ok(o) => o,
                Err(e) => {
                    let _ = std::fs::remove_file(&out_bin);
                    return Some(format!("{}: run failed: {}", path.display(), e));
                }
            };
            let _ = std::fs::remove_file(&out_bin);
            if !out.status.success() {
                return Some(format!(
                    "{}: {}",
                    path.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout != expected {
                return Some(format!("{}: output mismatch", path.display()));
            }
            None
        })
        .collect();
    assert!(
        errors.is_empty(),
        "cranelift failures:\n{}",
        errors.join("\n")
    );
}

/// Compile each .tish file to WASI, run with wasmtime, and compare stdout to static expected (parallelized).
/// Skips if wasmtime is not available.
#[test]
fn test_mvp_programs_wasi() {
    let wasmtime_available = Command::new("wasmtime")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !wasmtime_available {
        eprintln!("Skipping test_mvp_programs_wasi: wasmtime not found");
        return;
    }
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    let errors: Vec<String> = CRANELIFT_TEST_FILES
        .par_iter()
        .filter_map(|name| {
            let path = core_dir.join(name);
            if !path.exists() {
                return None;
            }
            let expected = match get_expected(&path) {
                Some(e) => e,
                None => return Some(format!("missing expected: {}", path.display())),
            };
            let out_wasm = compile_cached(&bin, &path, "wasi");
            let out = match Command::new("wasmtime")
                .arg(out_wasm.as_os_str())
                .current_dir(workspace_root())
                .output()
            {
                Ok(o) => o,
                Err(e) => {
                    let _ = std::fs::remove_file(&out_wasm);
                    return Some(format!("{}: wasmtime failed: {}", path.display(), e));
                }
            };
            let _ = std::fs::remove_file(&out_wasm);
            if !out.status.success() {
                return Some(format!(
                    "{}: {}",
                    path.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout != expected {
                return Some(format!("{}: output mismatch", path.display()));
            }
            None
        })
        .collect();
    assert!(errors.is_empty(), "wasi failures:\n{}", errors.join("\n"));
}

/// Files where Tish intentionally differs from JavaScript (typeof, void); skip in JS test since we compare to Tish expected.
const JS_SKIP_FILES: &[&str] = &["typeof.tish", "void.tish"];

/// Compile each .tish file to JS, run with Node, and compare stdout to static expected.
#[test]
fn test_mvp_programs_js() {
    let node_available = Command::new("node")
        .args(["--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !node_available {
        eprintln!("Skipping test_mvp_programs_js: Node.js not found");
        return;
    }
    let core_dir = core_dir();
    let bin = tish_bin();
    assert!(
        bin.exists(),
        "tish binary not found at {}. Run `cargo build -p tishlang` first.",
        bin.display()
    );
    for name in MVP_TEST_FILES {
        if JS_SKIP_FILES.contains(name) {
            continue;
        }
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        let expected = get_expected(&path).unwrap_or_else(|| {
            panic!(
                "missing expected file for {}; run with REGENERATE_EXPECTED=1 to generate",
                path.display()
            )
        });
        let out_js = compile_cached(&bin, &path, "js");
        let out = Command::new("node")
            .arg(&out_js)
            .current_dir(workspace_root())
            .output()
            .expect("run node");
        let _ = std::fs::remove_file(&out_js);
        assert!(
            out.status.success(),
            "Node failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout,
            expected,
            "JS output mismatch for {}",
            path.display()
        );
    }
}
