//! Build native binary via cargo (interim path until Cranelift backend is ready).

use std::fs;
use std::path::Path;
use std::process::Command;

use tish_compile::ResolvedNativeModule;

pub fn build_via_cargo(
    rust_code: &str,
    native_modules: Vec<ResolvedNativeModule>,
    output_path: &Path,
    features: &[String],
) -> Result<(), String> {
    let out_name = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let build_dir = std::env::temp_dir()
        .join(format!("tish_build_{}_{}", out_name, std::process::id()));

    fs::create_dir_all(&build_dir).map_err(|e| format!("Cannot create build dir: {}", e))?;
    fs::create_dir_all(build_dir.join("src"))
        .map_err(|e| format!("Cannot create src: {}", e))?;

    let runtime_path = find_runtime_path()?;

    let runtime_features: Vec<&str> = features
        .iter()
        .filter(|f| ["http", "fs", "process", "regex"].contains(&f.as_str()))
        .map(String::as_str)
        .collect();
    let features_str = if runtime_features.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", runtime_features)
    };

    let needs_tokio = rust_code.contains("#[tokio::main]");
    let tokio_dep = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n"
    } else {
        ""
    };

    let native_deps: String = native_modules
        .iter()
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    let cargo_toml = format!(
        r#"[package]
name = "tish_output"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{}"
path = "src/main.rs"

[dependencies]
tish_runtime = {{ path = {:?}{} }}{}{}
"#,
        out_name,
        runtime_path,
        features_str,
        tokio_dep,
        if native_deps.is_empty() {
            String::new()
        } else {
            format!("\n{}", native_deps)
        }
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;
    fs::write(build_dir.join("src/main.rs"), rust_code)
        .map_err(|e| format!("Cannot write main.rs: {}", e))?;

    let workspace_target = Path::new(&runtime_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|ws| ws.join("target"));
    let (target_dir, binary_dir) = if let Some(ref wt) = workspace_target.filter(|p| p.exists()) {
        (wt.clone(), wt.join("release"))
    } else {
        let td = build_dir.join("target");
        (td.clone(), td.join("release"))
    };

    let status = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(&target_dir)
        .current_dir(&build_dir)
        .env_remove("CARGO_TARGET_DIR")
        .env("CARGO_TERM_PROGRESS", "always")
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !status.success() {
        return Err("Compilation failed".to_string());
    }

    let binary_no_ext = binary_dir.join(out_name);
    let binary_exe = binary_dir.join(format!("{}.exe", out_name));
    let binary = if binary_no_ext.exists() {
        binary_no_ext
    } else if binary_exe.exists() {
        binary_exe
    } else {
        return Err(format!(
            "Binary not found at {} or {}",
            binary_no_ext.display(),
            binary_exe.display()
        ));
    };

    let target = if output_path.extension().is_none()
        || output_path.extension() == Some(std::ffi::OsStr::new(""))
    {
        let mut p = output_path.to_path_buf();
        if cfg!(windows) {
            p.set_extension("exe");
        }
        p
    } else if output_path.to_string_lossy().ends_with('/') || output_path.is_dir() {
        let dir = if output_path.is_dir() {
            output_path.to_path_buf()
        } else {
            output_path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        dir.join(if cfg!(windows) {
            format!("{}.exe", out_name)
        } else {
            out_name.to_string()
        })
    } else {
        output_path.to_path_buf()
    };

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create output dir: {}", e))?;
    }
    fs::copy(&binary, &target)
        .map_err(|e| format!("Cannot copy to {}: {}", target.display(), e))?;

    Ok(())
}

fn find_runtime_path() -> Result<String, String> {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let path = Path::new(&manifest_dir).join("..").join("tish_runtime");
        if path.canonicalize().is_ok() {
            return Ok(path.canonicalize().unwrap().display().to_string().replace('\\', "/"));
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let path = exe_dir
                .join("..")
                .join("..")
                .join("crates")
                .join("tish_runtime");
            if path.canonicalize().is_ok() {
                return Ok(path.canonicalize().unwrap().display().to_string().replace('\\', "/"));
            }
        }
    }
    let cwd_based = Path::new("crates").join("tish_runtime");
    if cwd_based.canonicalize().is_ok() {
        return Ok(cwd_based.canonicalize().unwrap().display().to_string().replace('\\', "/"));
    }
    if let Ok(mut current) = std::env::current_dir() {
        for _ in 0..10 {
            if current.join("Cargo.toml").exists() {
                let runtime = current.join("crates").join("tish_runtime");
                if runtime.exists() {
                    return Ok(runtime.display().to_string().replace('\\', "/"));
                }
            }
            if !current.pop() {
                break;
            }
        }
    }
    Err("Could not find tish_runtime crate".to_string())
}
