//! Full-stack integration tests: parse, interpreter, and native compile of .tish files.
//!
//! Run with: `cargo test -p tish` (full stack) or `cargo test` (all packages).

use std::path::PathBuf;
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

fn tish_bin() -> PathBuf {
    target_dir().join("debug").join("tish")
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
#[test]
fn test_mvp_programs_interpreter() {
    let core_dir = core_dir();
    for entry in std::fs::read_dir(&core_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "tish").unwrap_or(false) {
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
    ];
    for name in test_files {
        let path = core_dir.join(name);
        if !path.exists() {
            continue;
        }
        {
            let path_str = path.to_string_lossy();

            let interp_out = Command::new(&bin)
                .args(["run", &path_str])
                .current_dir(workspace_root())
                .output()
                .expect("run tish interpreter");
            assert!(
                interp_out.status.success(),
                "Interpreter failed for {}: {}",
                path.display(),
                String::from_utf8_lossy(&interp_out.stderr)
            );

            let out_bin = std::env::temp_dir().join(format!("tish_test_{}", path.file_stem().unwrap().to_string_lossy()));
            let compile_out = Command::new(&bin)
                .args(["compile", &path_str, "-o"])
                .arg(out_bin.to_string_lossy().as_ref())
                .current_dir(workspace_root())
                .output()
                .expect("run tish compile");
            assert!(
                compile_out.status.success(),
                "Compile failed for {}: {}",
                path.display(),
                String::from_utf8_lossy(&compile_out.stderr)
            );

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
}

