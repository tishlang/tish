//! Shared build utilities for Tish.
//!
//! Provides workspace discovery, path resolution, and Cargo build orchestration
//! used by tishlang_wasm, tishlang_cranelift, tishlang_native, and the tish CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// True if `root` looks like the Tish language repo (has `crates/tish_runtime`).
///
/// Used so we do not treat unrelated workspaces (e.g. a parent `zectre-platform` repo) as Tish
/// when `CARGO_MANIFEST_DIR` or cwd points at another Rust workspace.
fn is_tish_workspace_root(root: &Path) -> bool {
    root.join("crates").join("tish_runtime").is_dir()
}

/// True if `line` (trimmed) opens a Cargo.toml table whose body may contain path dependencies.
fn cargo_section_may_contain_path_deps(header: &str) -> bool {
    let h = header.trim();
    if h == "dependencies"
        || h == "dev-dependencies"
        || h == "build-dependencies"
        || h == "workspace.dependencies"
    {
        return true;
    }
    h.starts_with("dependencies.")
        || h.starts_with("dev-dependencies.")
        || h.starts_with("build-dependencies.")
        || h.starts_with("workspace.dependencies.")
        || h.starts_with("patch.")
}

/// Collect `path = "..."` / `path = '...'` strings from lines in dependency-related sections.
fn path_values_from_cargo_toml(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_section = cargo_section_may_contain_path_deps(rest);
            continue;
        }
        if !in_section {
            continue;
        }
        extract_path_assignments_from_line(trimmed, &mut out);
    }
    out
}

fn extract_path_assignments_from_line(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(idx) = rest.find("path") {
        let after = rest[idx + 4..].trim_start();
        let after = match after.strip_prefix('=') {
            Some(a) => a.trim_start(),
            None => {
                rest = &rest[idx + 4..];
                continue;
            }
        };
        let quote = match after.chars().next() {
            Some('"') => '"',
            Some('\'') => '\'',
            _ => {
                rest = &rest[idx + 4..];
                continue;
            }
        };
        let after = &after[quote.len_utf8()..];
        let end = after.find(quote);
        let Some(end) = end else {
            rest = &rest[idx + 4..];
            continue;
        };
        out.push(after[..end].to_string());
        rest = &after[end + quote.len_utf8()..];
    }
}

/// Starting from a filesystem path (crate dir or file), walk ancestors for `crates/tish_runtime`.
fn tish_root_from_path_hint(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    dir = fs::canonicalize(&dir).unwrap_or(dir);
    let mut cur = dir.as_path();
    for _ in 0..32 {
        if is_tish_workspace_root(cur) {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
    None
}

/// Scan `dir/Cargo.toml` for path dependencies; if any resolves inside a Tish workspace, return that root.
fn tish_root_from_cargo_manifest_dir(dir: &Path) -> Option<PathBuf> {
    let cargo_toml = dir.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return None;
    }
    let content = fs::read_to_string(&cargo_toml).ok()?;
    let base = dir;
    for rel in path_values_from_cargo_toml(&content) {
        let joined = base.join(&rel);
        let resolved = match joined.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Some(root) = tish_root_from_path_hint(&resolved) {
            return Some(root);
        }
    }
    None
}

/// Walk from `start` upward; at each directory try [`tish_root_from_cargo_manifest_dir`].
fn tish_root_from_project_cargo_files(mut start: PathBuf) -> Option<PathBuf> {
    for _ in 0..32 {
        if let Some(root) = tish_root_from_cargo_manifest_dir(&start) {
            return Some(root);
        }
        if !start.pop() {
            break;
        }
    }
    None
}

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
            if root_buf.join("Cargo.toml").exists() && is_tish_workspace_root(&root_buf) {
                return Ok(root_buf);
            }
        }
        // Consumer workspace: manifest is the app crate; path deps point at Tish checkout.
        if let Some(root) = tish_root_from_project_cargo_files(path.clone()) {
            return Ok(root);
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

    // Strategy 3: Walk from current working directory (path deps on a consumer crate)
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = tish_root_from_project_cargo_files(cwd.clone()) {
            return Ok(root);
        }
    }

    // Strategy 4: Walk from current working directory
    if let Ok(mut current) = std::env::current_dir() {
        for _ in 0..15 {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") && is_tish_workspace_root(&current) {
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

/// Path to `crates/tish_runtime` inside a locally installed `@tishlang/tish` npm package.
pub fn npm_package_runtime_path(project_root: &Path) -> Option<PathBuf> {
    let p = project_root
        .join("node_modules")
        .join("@tishlang")
        .join("tish")
        .join("crates")
        .join("tish_runtime");
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

/// Find the path to the tishlang_runtime crate.
///
/// Returns a canonical path string suitable for Cargo.toml path dependencies.
pub fn find_runtime_path() -> Result<String, String> {
    let workspace = find_workspace_root()?;
    let runtime = workspace.join("crates").join("tish_runtime");
    if !runtime.exists() {
        return Err("tishlang_runtime crate not found".to_string());
    }
    runtime
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize tishlang_runtime: {}", e))
        .map(|p| p.display().to_string().replace('\\', "/"))
}

/// Resolve `tishlang_runtime` for a Cargo build, preferring the npm install under `project_root`.
///
/// When a Tish app lives next to a checkout of the language repo (e.g. `…/tish/tish-cargo-example`),
/// [`find_workspace_root`] can return the checkout while `rustDependencies` point at
/// `node_modules/@tishlang/tish/crates/tish_core`. Using the npm tree for **both** runtime and shim
/// avoids Cargo lockfile "package collision" for the same crate name/version at two paths.
pub fn find_runtime_path_for_project(project_root: Option<&Path>) -> Result<String, String> {
    if let Some(root) = project_root {
        if let Some(rt) = npm_package_runtime_path(root) {
            return rt
                .canonicalize()
                .map_err(|e| format!("Cannot canonicalize tishlang_runtime (npm): {}", e))
                .map(|p| p.display().to_string().replace('\\', "/"));
        }
    }
    find_runtime_path()
}

/// Crate package name -> directory name (directories kept as tish_* for historical reasons).
const CRATE_NAME_TO_DIR: &[(&str, &str)] = &[
    ("tishlang_runtime", "tish_runtime"),
    ("tishlang_cranelift_runtime", "tish_cranelift_runtime"),
    ("tishlang_wasm_runtime", "tish_wasm_runtime"),
]; // directory names kept as tish_* for historical reasons

/// Find the path to a crate within the workspace by name.
///
/// e.g. `find_crate_path("tishlang_cranelift_runtime")` returns the path to crates/tish_cranelift_runtime.
pub fn find_crate_path(crate_name: &str) -> Result<PathBuf, String> {
    let workspace = find_workspace_root()?;
    let dir_name = CRATE_NAME_TO_DIR
        .iter()
        .find(|(name, _)| *name == crate_name)
        .map(|(_, dir)| *dir)
        .unwrap_or(crate_name);
    let crate_path = workspace.join("crates").join(dir_name);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_values_dependencies_section_only() {
        let toml = r#"
[package]
name = "app"
path = "ignored-outside-deps"

[dependencies]
tishlang_runtime = { path = "../tish/crates/tish_runtime" }

[metadata]
path = "also-ignored"
"#;
        let paths = path_values_from_cargo_toml(toml);
        assert_eq!(paths, vec!["../tish/crates/tish_runtime"]);
    }

    #[test]
    fn path_values_workspace_dependencies() {
        let toml = r#"
[workspace.dependencies]
tishlang_runtime = { path = "../../tish/tish/crates/tish_runtime" }
"#;
        let paths = path_values_from_cargo_toml(toml);
        assert_eq!(paths, vec!["../../tish/tish/crates/tish_runtime"]);
    }

    #[test]
    fn path_values_patch_section() {
        let toml = r#"
[patch.crates-io]
tishlang_runtime = { path = "../vendor/tish_runtime" }
"#;
        let paths = path_values_from_cargo_toml(toml);
        assert_eq!(paths, vec!["../vendor/tish_runtime"]);
    }
}
