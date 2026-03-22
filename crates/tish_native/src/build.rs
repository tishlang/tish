//! Build native binary via cargo (interim path until Cranelift backend is ready).

use std::fs;
use std::path::Path;

use tishlang_compile::ResolvedNativeModule;

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
    let build_dir = tishlang_build_utils::create_build_dir("tish_build", out_name)?;

    let runtime_path = tishlang_build_utils::find_runtime_path()?;

    let runtime_features: Vec<&str> = features
        .iter()
        .filter(|f| ["http", "fs", "process", "regex", "ws"].contains(&f.as_str()))
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

[profile.release]
# Reduce binary size: strip symbols, abort on panic (no unwinding), single codegen unit
strip = true
panic = "abort"
codegen-units = 1
lto = "thin"

[dependencies]
tishlang_runtime = {{ path = {:?}{} }}{}{}
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
    let target_dir = workspace_target.filter(|p| p.exists());
    let binary_dir = target_dir
        .as_ref()
        .map(|t| t.join("release"))
        .unwrap_or_else(|| build_dir.join("target").join("release"));

    tishlang_build_utils::run_cargo_build(&build_dir, target_dir.as_deref())?;

    let binary = tishlang_build_utils::find_release_binary(&binary_dir, out_name)?;
    let target = tishlang_build_utils::resolve_output_path(output_path, out_name);
    tishlang_build_utils::copy_binary_to_output(&binary, &target)?;

    Ok(())
}

