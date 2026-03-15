//! Shared build utilities for Tish.
//!
//! Provides workspace discovery, path resolution, and Cargo build orchestration
//! used by tish_wasm, tish_cranelift, tish_native, and the tish CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the Tish workspace root using multiple strategies.
///
/// Returns the directory containing the workspace Cargo.toml (with [workspace]).
/// Used when building native binaries, WASM, or locating runtime crates.
pub fn find_workspace_root() -> Result<PathBuf, String> {
    // Strategy 1: CARGO_MANIFEST_DIR (works during cargo build/run from workspace)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(&manifest_dir);
        // For crates/tish_*, workspace root is parent.parent()
        if let Some(root) = path.parent().and_then(|p| p.parent()) {
            let root_buf = root.to_path_buf();
            if root_buf.join("Cargo.toml").exists() {
                return Ok(root_buf);
            }
        }
    }

    // Strategy 2: Walk from current executable (e.g. target/debug/tish)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(mut current) = exe.parent() {
            for _ in 0..15 {
                let crates_dir = current.join("crates");
                if crates_dir.join("tish_runtime").exists() || crates_dir.join("tish_cranelift_runtime").exists() {
                    return Ok(current.to_path_buf());
                }
                if let Some(p) = current.parent() {
                    current = p;
                } else {
                    break;
                }
            }
        }
    }

    // Strategy 3: Walk from current working directory
    if let Ok(mut current) = std::env::current_dir() {
        for _ in 0..15 {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        return Ok(current);
                    }
                }
                // Fallback: check for crates dir with known crates
                let crates_dir = current.join("crates");
                if crates_dir.join("tish_runtime").exists() {
                    return Ok(current);
                }
            }
            if !current.pop() {
                break;
            }
        }
    }

    Err("Cannot find Tish workspace root. Run from workspace root or use cargo run.".to_string())
}

/// Find the path to the tish_runtime crate.
///
/// Returns a canonical path string suitable for Cargo.toml path dependencies.
pub fn find_runtime_path() -> Result<String, String> {
    let workspace = find_workspace_root()?;
    let runtime = workspace.join("crates").join("tish_runtime");
    if !runtime.exists() {
        return Err("tish_runtime crate not found".to_string());
    }
    runtime
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize tish_runtime: {}", e))
        .map(|p| p.display().to_string().replace('\\', "/"))
}

/// Find the path to a crate within the workspace by name.
///
/// e.g. `find_crate_path("tish_cranelift_runtime")` returns the path to crates/tish_cranelift_runtime.
pub fn find_crate_path(crate_name: &str) -> Result<PathBuf, String> {
    let workspace = find_workspace_root()?;
    let crate_path = workspace.join("crates").join(crate_name);
    if !crate_path.exists() {
        return Err(format!("Crate {} not found", crate_name));
    }
    Ok(crate_path)
}

/// Create a temp build directory with src subdir.
pub fn create_build_dir(prefix: &str, out_name: &str) -> Result<PathBuf, String> {
    let build_dir = std::env::temp_dir().join(prefix).join(format!("{}_{}", out_name, std::process::id()));
    fs::create_dir_all(&build_dir).map_err(|e| format!("Cannot create build dir: {}", e))?;
    fs::create_dir_all(build_dir.join("src")).map_err(|e| format!("Cannot create src dir: {}", e))?;
    Ok(build_dir)
}

/// Run cargo build in the given directory.
/// If target_dir is Some, use that for --target-dir (e.g. workspace target for caching).
pub fn run_cargo_build(build_dir: &Path, target_dir: Option<&Path>) -> Result<(), String> {
    let target_dir = target_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| build_dir.join("target"));
    let output = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(&target_dir)
        .current_dir(build_dir)
        .env_remove("CARGO_TARGET_DIR")
        .env("CARGO_TERM_PROGRESS", "always")
        .output()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Compilation failed.\nstdout:\n{}\nstderr:\n{}", stdout, stderr));
    }
    Ok(())
}

/// Find the built binary in target/release.
pub fn find_release_binary(binary_dir: &Path, bin_name: &str) -> Result<PathBuf, String> {
    let binary_no_ext = binary_dir.join(bin_name);
    let binary_exe = binary_dir.join(format!("{}.exe", bin_name));
    if binary_no_ext.exists() {
        Ok(binary_no_ext)
    } else if binary_exe.exists() {
        Ok(binary_exe)
    } else {
        Err(format!(
            "Binary not found at {} or {}",
            binary_no_ext.display(),
            binary_exe.display()
        ))
    }
}

/// Resolve the output path for the binary (handles extension, directory).
pub fn resolve_output_path(output_path: &Path, bin_name: &str) -> PathBuf {
    if output_path.extension().is_none()
        || output_path.extension() == Some(std::ffi::OsStr::new(""))
    {
        let mut p = output_path.to_path_buf();
        if cfg!(windows) {
            p.set_extension("exe");
        }
        return p;
    }
    if output_path.to_string_lossy().ends_with('/') || output_path.is_dir() {
        let dir = if output_path.is_dir() {
            output_path.to_path_buf()
        } else {
            output_path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        return dir.join(if cfg!(windows) {
            format!("{}.exe", bin_name)
        } else {
            bin_name.to_string()
        });
    }
    output_path.to_path_buf()
}

/// Copy the built binary to the output path.
pub fn copy_binary_to_output(binary: &Path, output_path: &Path) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create output dir: {}", e))?;
    }
    fs::copy(binary, output_path).map_err(|e| format!("Cannot copy to {}: {}", output_path.display(), e))?;
    Ok(())
}
