//! Build native binary via cargo (interim path until Cranelift backend is ready).

use std::fs;
use std::path::Path;

use tishlang_compile::ResolvedNativeModule;

/// `tishlang_runtime` Cargo feature names (subset of CLI / compile feature names).
const RUNTIME_CARGO_FEATURES: &[&str] = &["http", "fs", "process", "regex", "ws"];

/// Map CLI/compile features to flags passed to `tishlang_runtime` in the temp crate's Cargo.toml.
/// `full` enables every optional runtime capability (matches `tish build --feature full` / LANGUAGE.md).
fn runtime_features_for_cargo(features: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for f in features {
        if f == "full" {
            for name in RUNTIME_CARGO_FEATURES {
                if !out.iter().any(|x: &String| x == *name) {
                    out.push((*name).to_string());
                }
            }
            continue;
        }
        if RUNTIME_CARGO_FEATURES.contains(&f.as_str()) && !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

/// Inject `mod generated_native;` after the crate attribute so the binary crate can call `crate::generated_native::…`.
fn inject_generated_native_mod(rust_code: &str) -> String {
    if let Some(pos) = rust_code.find("\n\n") {
        let (a, b) = rust_code.split_at(pos + 2);
        format!("{}mod generated_native;\n{}", a, b)
    } else {
        format!("{}\n\nmod generated_native;\n", rust_code)
    }
}

pub fn build_via_cargo(
    rust_code: &str,
    native_modules: Vec<ResolvedNativeModule>,
    output_path: &Path,
    features: &[String],
    extra_dependencies_toml: &str,
    generated_native_rs: Option<&str>,
    project_root: Option<&Path>,
) -> Result<(), String> {
    let out_name = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let build_dir = tishlang_build_utils::create_build_dir("tish_build", out_name)?;

    let runtime_path = tishlang_build_utils::find_runtime_path_for_project(project_root)?;

    let runtime_features = runtime_features_for_cargo(features);
    let runtime_refs: Vec<&str> = runtime_features.iter().map(String::as_str).collect();
    let features_str = if runtime_refs.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", runtime_refs)
    };

    let needs_tokio = rust_code.contains("#[tokio::main]");
    let tokio_dep = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n"
    } else {
        ""
    };

    let native_deps: String = native_modules
        .iter()
        .filter(|m| m.use_path_dependency)
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    let mut more_deps = String::new();
    more_deps.push_str(&tokio_dep);
    if !native_deps.is_empty() {
        more_deps.push_str(&format!("\n{}", native_deps));
    }
    if !extra_dependencies_toml.trim().is_empty() {
        more_deps.push_str(&format!("\n{}", extra_dependencies_toml));
    }

    let rust_main = if generated_native_rs.is_some() {
        inject_generated_native_mod(rust_code)
    } else {
        rust_code.to_string()
    };

    let tish_ui_path = std::path::Path::new(&runtime_path)
        .parent()
        .ok_or_else(|| "invalid tishlang_runtime path (no parent)".to_string())?
        .join("tish_ui");
    let ui_dep = if rust_code.contains("tishlang_ui") {
        format!(
            "\ntishlang_ui = {{ path = {:?}, default-features = false, features = [\"runtime\"] }}\n",
            tish_ui_path.display().to_string().replace('\\', "/")
        )
    } else {
        String::new()
    };

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
tishlang_runtime = {{ path = {:?}{} }}
{}{}"#,
        out_name, runtime_path, features_str, more_deps, ui_dep
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;
    if let Some(gen) = generated_native_rs {
        fs::write(build_dir.join("src/generated_native.rs"), gen)
            .map_err(|e| format!("Cannot write generated_native.rs: {}", e))?;
    }
    fs::write(build_dir.join("src/main.rs"), rust_main)
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

#[cfg(test)]
mod tests {
    use super::runtime_features_for_cargo;

    #[test]
    fn runtime_features_full_expands() {
        let f = runtime_features_for_cargo(&["full".to_string()]);
        assert!(f.contains(&"http".to_string()));
        assert!(f.contains(&"fs".to_string()));
        assert!(f.contains(&"process".to_string()));
        assert!(f.contains(&"regex".to_string()));
        assert!(f.contains(&"ws".to_string()));
    }

    #[test]
    fn runtime_features_merges_full_and_specific() {
        let f = runtime_features_for_cargo(&["full".to_string(), "http".to_string()]);
        assert_eq!(f.len(), 5);
    }
}
