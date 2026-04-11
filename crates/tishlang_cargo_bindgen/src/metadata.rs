//! Resolve a crates.io (or registry) dependency to its on-disk source tree via `cargo metadata`.

use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{MetadataCommand, Package, TargetKind};

/// Root directory of the resolved dependency (contains `Cargo.toml` and usually `src/`).
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    pub name: String,
    pub version: String,
    pub manifest_path: PathBuf,
}

impl ResolvedDependency {
    pub fn source_root(&self) -> PathBuf {
        self.manifest_path
            .parent()
            .expect("manifest has parent")
            .to_path_buf()
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

fn from_package(pkg: &Package) -> ResolvedDependency {
    ResolvedDependency {
        name: pkg.name.clone(),
        version: pkg.version.to_string(),
        manifest_path: pkg.manifest_path.as_std_path().to_path_buf(),
    }
}

/// Build a tiny probe crate, run `cargo metadata`, and return the named package.
pub fn resolve_registry_dependency(name: &str, version_req: &str) -> Result<ResolvedDependency, String> {
    let probe_dir = tempfile::tempdir().map_err(|e| format!("tempdir: {}", e))?;
    let probe_toml = format!(
        r#"[package]
name = "_tishlang_bindgen_probe"
version = "0.0.0"
edition = "2021"
[dependencies]
{} = "{}"
"#,
        name, version_req
    );
    let manifest = probe_dir.path().join("Cargo.toml");
    fs::write(&manifest, probe_toml).map_err(|e| format!("write probe Cargo.toml: {}", e))?;
    // `cargo metadata` requires at least one target in the root package.
    let src_dir = probe_dir.path().join("src");
    fs::create_dir_all(&src_dir).map_err(|e| format!("probe src dir: {}", e))?;
    fs::write(src_dir.join("lib.rs"), "// probe only\n")
        .map_err(|e| format!("write probe lib.rs: {}", e))?;

    let meta = MetadataCommand::new()
        .manifest_path(&manifest)
        .exec()
        .map_err(|e| format!("cargo metadata failed: {} (is `cargo` on PATH?)", e))?;

    let pkg = meta
        .packages
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| {
            format!(
                "dependency `{}` not found in metadata (check name and version req `{}`)",
                name, version_req
            )
        })?;

    let root = pkg.manifest_path.as_std_path().parent().unwrap();
    if !root.join("src").is_dir()
        && !pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| *k == TargetKind::Lib))
    {
        return Err(format!(
            "package `{}` has no src/ or lib target at {}",
            name,
            root.display()
        ));
    }

    Ok(from_package(pkg))
}

/// Resolve using an existing workspace manifest (no temporary probe).
pub fn resolve_dependency_from_manifest(
    manifest_path: &Path,
    package_name: &str,
) -> Result<ResolvedDependency, String> {
    let meta = MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .map_err(|e| format!("cargo metadata: {}", e))?;

    let pkg = meta
        .packages
        .iter()
        .find(|p| p.name == package_name)
        .ok_or_else(|| {
            format!(
                "package `{}` not found in metadata for {}",
                package_name,
                manifest_path.display()
            )
        })?;

    Ok(from_package(pkg))
}
