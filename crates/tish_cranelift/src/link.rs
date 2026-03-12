//! Link object file with runtime to produce final binary.
//!
//! Uses Cargo to build a small binary that links our .o and runs the chunk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CraneliftError;

pub fn link_to_binary(object_path: &Path, output_path: &Path) -> Result<(), CraneliftError> {
    let workspace_root = find_workspace_root()?;
    let out_name = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let build_dir = std::env::temp_dir()
        .join("tish_cranelift_build")
        .join(format!("{}_{}", out_name, std::process::id()));

    fs::create_dir_all(&build_dir).map_err(|e| CraneliftError {
        message: format!("Cannot create build dir: {}", e),
    })?;
    fs::create_dir_all(build_dir.join("src"))
        .map_err(|e| CraneliftError {
            message: format!("Cannot create src: {}", e),
        })?;

    let object_path_str = object_path
        .canonicalize()
        .map_err(|e| CraneliftError {
            message: format!("Cannot canonicalize object path: {}", e),
        })?
        .display()
        .to_string()
        .replace('\\', "/");

    // tish_cranelift_runtime path (workspace/crates/tish_cranelift_runtime)
    let runtime_path = workspace_root
        .join("crates")
        .join("tish_cranelift_runtime")
        .canonicalize()
        .map_err(|e| CraneliftError {
            message: format!("Cannot find tish_cranelift_runtime: {}", e),
        })?
        .display()
        .to_string()
        .replace('\\', "/");

    let cargo_toml_fixed = format!(
        r#"[package]
name = "tish_cranelift_out"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{}"
path = "src/main.rs"

[dependencies]
tish_cranelift_runtime = {{ path = {:?} }}
"#,
        out_name, runtime_path
    );

    let main_rs = r#"
extern "C" {
    static tish_chunk_data: [u8; 1];
    static tish_chunk_len: u64;
}

fn main() {
    let len = unsafe { tish_chunk_len } as usize;
    let ptr = unsafe { tish_chunk_data.as_ptr() };
    let exit_code = tish_cranelift_runtime::tish_run_chunk(ptr, len);
    std::process::exit(exit_code);
}
"#;

    let build_rs = format!(
        r#"
fn main() {{
    println!("cargo:rustc-link-arg={}");
}}
"#,
        object_path_str
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml_fixed).map_err(|e| CraneliftError {
        message: format!("Cannot write Cargo.toml: {}", e),
    })?;
    fs::write(build_dir.join("src/main.rs"), main_rs).map_err(|e| CraneliftError {
        message: format!("Cannot write main.rs: {}", e),
    })?;
    fs::write(build_dir.join("build.rs"), build_rs).map_err(|e| CraneliftError {
        message: format!("Cannot write build.rs: {}", e),
    })?;

    let output = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(build_dir.join("target"))
        .current_dir(&build_dir)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .map_err(|e| CraneliftError {
            message: format!("Failed to run cargo: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(CraneliftError {
            message: format!(
                "Cargo build failed.\nstdout:\n{}\nstderr:\n{}",
                stdout, stderr
            ),
        });
    }

    let binary_dir = build_dir.join("target").join("release");
    let binary_no_ext = binary_dir.join(out_name);
    let binary_exe = binary_dir.join(format!("{}.exe", out_name));
    let binary = if binary_no_ext.exists() {
        binary_no_ext
    } else if binary_exe.exists() {
        binary_exe
    } else {
        return Err(CraneliftError {
            message: format!(
                "Binary not found at {} or {}",
                binary_no_ext.display(),
                binary_exe.display()
            ),
        });
    };

    let target = if output_path.extension().is_none()
        || output_path
            .extension()
            .map(|e| e.is_empty())
            .unwrap_or(true)
    {
        let mut p = output_path.to_path_buf();
        if cfg!(windows) {
            p.set_extension("exe");
        }
        p
    } else {
        output_path.to_path_buf()
    };

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| CraneliftError {
            message: format!("Cannot create output dir: {}", e),
        })?;
    }
    fs::copy(&binary, &target).map_err(|e| CraneliftError {
        message: format!("Cannot copy to {}: {}", target.display(), e),
    })?;

    Ok(())
}

fn find_workspace_root() -> Result<PathBuf, CraneliftError> {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = PathBuf::from(&manifest_dir);
        if let Some(root) = path.parent().and_then(|p| p.parent()) {
            return Ok(root.to_path_buf());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(mut current) = exe.parent() {
            for _ in 0..10 {
                let crates = current.join("crates").join("tish_cranelift_runtime");
                if crates.exists() {
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
    if let Ok(cwd) = std::env::current_dir() {
        let mut current = cwd;
        for _ in 0..10 {
            let crates = current.join("crates").join("tish_cranelift_runtime");
            if crates.exists() {
                return Ok(current);
            }
            if !current.pop() {
                break;
            }
        }
    }
    Err(CraneliftError {
        message: "Cannot find workspace root (crates/tish_cranelift_runtime). Run from tish workspace.".to_string(),
    })
}

