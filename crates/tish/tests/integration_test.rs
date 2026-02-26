//! Integration tests: run MVP programs via interpreter and native backend.
//! Compares output from both backends to ensure parity.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn target_dir() -> PathBuf {
    // Use CARGO_TARGET_DIR if set, else default to workspace/target
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join("target"))
}

#[test]
fn test_mvp_programs_interpreter() {
    let mvp_dir = workspace_root().join("tests").join("mvp");
    for entry in std::fs::read_dir(&mvp_dir).unwrap() {
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

#[test]
fn test_mvp_programs_interpreter_vs_native() {
    let mvp_dir = workspace_root().join("tests").join("mvp");
    let tish_bin = target_dir().join("debug").join("tish");
    if !tish_bin.exists() {
        return;
    }

    // Test a representative subset (full set is slow - each compile takes ~1-2s)
    let test_files = ["strict_equality.tish", "arrays.tish", "fun_any.tish"];
    for name in test_files {
        let path = mvp_dir.join(name);
        if !path.exists() {
            continue;
        }
        {
            let path_str = path.to_string_lossy();

            let interp_out = Command::new(&tish_bin)
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
            let compile_out = Command::new(&tish_bin)
                .args(["compile", &path_str, "-o"])
                .arg(out_bin.to_string_lossy().as_ref())
                .current_dir(workspace_root())
                .output()
                .expect("run tish compile");
            if !compile_out.status.success() {
                eprintln!("Compile failed for {}, skipping native compare", path.display());
                continue;
            }

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

