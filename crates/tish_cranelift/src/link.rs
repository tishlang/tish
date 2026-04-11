//! Link object file with runtime to produce final binary.
//!
//! Uses Cargo to build a small binary that links our .o and runs the chunk.

use std::fs;
use std::path::Path;

use crate::CraneliftError;

pub fn link_to_binary(
    object_path: &Path,
    output_path: &Path,
    features: &[String],
) -> Result<(), CraneliftError> {
    let workspace_root =
        tishlang_build_utils::find_workspace_root().map_err(|e| CraneliftError { message: e })?;
    let out_name = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let build_dir = tishlang_build_utils::create_build_dir("tishlang_cranelift_build", out_name)
        .map_err(|e| CraneliftError { message: e })?;

    let object_path_str = object_path
        .canonicalize()
        .map_err(|e| CraneliftError {
            message: format!("Cannot canonicalize object path: {}", e),
        })?
        .display()
        .to_string()
        .replace('\\', "/");

    // tishlang_cranelift_runtime crate lives in crates/tish_cranelift_runtime
    let runtime_path = workspace_root
        .join("crates")
        .join("tish_cranelift_runtime")
        .canonicalize()
        .map_err(|e| CraneliftError {
            message: format!("Cannot find tishlang_cranelift_runtime: {}", e),
        })?
        .display()
        .to_string()
        .replace('\\', "/");

    let features_str = if features.is_empty() {
        String::new()
    } else {
        format!(
            ", features = [{}]",
            features
                .iter()
                .map(|f| format!("{:?}", f))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let cargo_toml_fixed = format!(
        r#"[package]
name = "tishlang_cranelift_out"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{}"
path = "src/main.rs"

[dependencies]
tishlang_cranelift_runtime = {{ path = {:?}{} }}
"#,
        out_name, runtime_path, features_str
    );

    let main_rs = r#"
extern "C" {
    static tish_chunk_data: [u8; 1];
    static tish_chunk_len: u64;
}

fn main() {
    let len = unsafe { tish_chunk_len } as usize;
    let ptr = unsafe { tish_chunk_data.as_ptr() };
    let exit_code = tishlang_cranelift_runtime::tish_run_chunk(ptr, len);
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

    tishlang_build_utils::run_cargo_build(&build_dir, None)
        .map_err(|e| CraneliftError { message: e })?;

    let binary_dir = build_dir.join("target").join("release");
    let binary = tishlang_build_utils::find_release_binary(&binary_dir, out_name)
        .map_err(|e| CraneliftError { message: e })?;
    let target = tishlang_build_utils::resolve_output_path(output_path, out_name);
    tishlang_build_utils::copy_binary_to_output(&binary, &target)
        .map_err(|e| CraneliftError { message: e })?;

    Ok(())
}
