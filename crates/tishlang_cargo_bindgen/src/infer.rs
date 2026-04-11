//! Infer bindgen targets from project `package.json` (`tish.rustDependencies`) and glue `Cargo.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Glue crate identity + upstream Cargo dependency to wrap.
#[derive(Debug, Clone)]
pub struct InferredProjectBindgen {
    /// `tish.rustDependencies` key and `[package].name` for the generated crate.
    pub output_crate_name: String,
    pub out_dir: PathBuf,
    pub dependency_name: String,
    pub dependency_version_req: String,
}

/// Pick the path-based glue crate from `package.json` → `tish.rustDependencies`.
pub fn select_glue_crate_from_package_json(
    project_root: &Path,
    rust_dep_key: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let pkg_json_path = project_root.join("package.json");
    let raw = fs::read_to_string(&pkg_json_path).map_err(|e| {
        format!(
            "could not read {}: {} (for automatic mode, run from the Tish project root or pass --project-root)",
            pkg_json_path.display(),
            e
        )
    })?;
    let j: Value = serde_json::from_str(&raw).map_err(|e| format!("{}: {}", pkg_json_path.display(), e))?;
    let rust_deps = j
        .get("tish")
        .and_then(|t| t.get("rustDependencies"))
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            format!(
                "{} must contain `tish.rustDependencies` (object) to infer glue paths",
                pkg_json_path.display()
            )
        })?;

    let mut path_entries: Vec<(String, PathBuf)> = Vec::new();
    for (key, val) in rust_deps {
        let rel = match val {
            Value::Object(o) => o.get("path").and_then(|p| p.as_str()),
            _ => None,
        };
        if let Some(p) = rel {
            let abs = project_root.join(p);
            path_entries.push((key.clone(), abs));
        }
    }

    path_entries.sort_by(|a, b| a.0.cmp(&b.0));

    if path_entries.is_empty() {
        return Err(
            "no path-based entries in tish.rustDependencies (only version strings? bindgen updates local path crates)"
                .into(),
        );
    }

    match rust_dep_key {
        Some(k) => {
            let found = path_entries
                .iter()
                .find(|(key, _)| key == k)
                .ok_or_else(|| {
                    format!(
                        "tish.rustDependencies has no path entry for key `{}` (have: {})",
                        k,
                        path_entries
                            .iter()
                            .map(|(k, _)| k.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            Ok(found.clone())
        }
        None => {
            if path_entries.len() != 1 {
                return Err(format!(
                    "multiple path rustDependencies — pass --crate-name <key> (keys: {})",
                    path_entries
                        .iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(path_entries[0].clone())
        }
    }
}

fn dep_version_req_string_glue(spec: &toml::Value) -> Result<String, String> {
    match spec {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Table(t) => {
            if t.get("workspace").and_then(|v| v.as_bool()) == Some(true) {
                return Err(
                    "glue Cargo.toml uses `workspace = true` for an upstream dep; set an explicit version in the glue crate or use --dependency with --out-dir"
                        .into(),
                );
            }
            if t.get("path").is_some() || t.get("git").is_some() {
                return Err(
                    "expected a registry-style upstream dep with a semver (string or version = \"…\"); path/git deps are not supported as bindgen upstream"
                        .into(),
                );
            }
            t.get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    "dependency entry needs a version string or `version = \"…\"` in a table".into()
                })
        }
        _ => Err("unsupported dependency entry in glue Cargo.toml".into()),
    }
}

/// Crates we ignore when inferring bindgen **upstream** from the **app** `Cargo.toml` (not the glue crate).
const ROOT_MANIFEST_SKIP_DEPS: &[&str] = &["tishlang_runtime", "tishlang_core", "tishlang_build_utils"];

/// Registry semver for bindgen metadata probe; skips path/git/workspace-only entries (project root fallback).
fn dep_version_req_string_root(spec: &toml::Value) -> Option<String> {
    match spec {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => {
            if t.get("workspace").and_then(|v| v.as_bool()) == Some(true) {
                return None;
            }
            if t.get("path").is_some() || t.get("git").is_some() {
                return None;
            }
            t.get("version").and_then(|v| v.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

fn collect_upstream_candidates_from_table(
    deps: &toml::Table,
    skip_names: &[&str],
    version_fn: fn(&toml::Value) -> Result<String, String>,
) -> Result<Vec<(String, String)>, String> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    for (name, spec) in deps {
        if name == "tishlang_runtime" || skip_names.iter().any(|s| *s == name) {
            continue;
        }
        let ver = version_fn(spec)?;
        candidates.push((name.clone(), ver));
    }
    Ok(candidates)
}

fn collect_upstream_candidates_from_table_root(deps: &toml::Table) -> Vec<(String, String)> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    for (name, spec) in deps {
        if name == "tishlang_runtime" || ROOT_MANIFEST_SKIP_DEPS.iter().any(|s| *s == name) {
            continue;
        }
        if let Some(ver) = dep_version_req_string_root(spec) {
            candidates.push((name.clone(), ver));
        }
    }
    candidates
}

fn disambiguate_upstream_candidates(
    candidates: Vec<(String, String)>,
    manifest_path: &Path,
    hint: &str,
) -> Result<(String, String), String> {
    match candidates.len() {
        0 => Err(format!(
            "{}: no registry semver dependency to use as bindgen upstream{}",
            manifest_path.display(),
            hint
        )),
        1 => Ok(candidates[0].clone()),
        2 => {
            let non_sj: Vec<_> = candidates
                .iter()
                .filter(|(n, _)| n != "serde_json")
                .cloned()
                .collect();
            if non_sj.len() == 1 {
                Ok(non_sj[0].clone())
            } else {
                Err(format!(
                    "{}: ambiguous dependencies (expected one upstream, or one non-serde_json + serde_json); use explicit --dependency",
                    manifest_path.display()
                ))
            }
        }
        _ => Err(format!(
            "{}: too many candidate upstream crates; narrow [dependencies] or use explicit --dependency",
            manifest_path.display()
        )),
    }
}

/// Read `[dependencies]` from the glue crate: upstream is everything except `tishlang_runtime`,
/// disambiguating when `serde_json` is present as a JSON-bridge helper alongside another crate.
pub fn parse_upstream_from_glue_cargo(cargo_toml_path: &Path) -> Result<(String, String), String> {
    let s = fs::read_to_string(cargo_toml_path).map_err(|e| e.to_string())?;
    let root: toml::Value = toml::from_str(&s).map_err(|e| format!("{}: {}", cargo_toml_path.display(), e))?;
    let deps = root
        .get("dependencies")
        .and_then(|d| d.as_table())
        .ok_or_else(|| format!("{} has no [dependencies]", cargo_toml_path.display()))?;

    let candidates = collect_upstream_candidates_from_table(deps, &[], dep_version_req_string_glue)?;
    disambiguate_upstream_candidates(
        candidates,
        cargo_toml_path,
        " (only tishlang_runtime?) — add e.g. `serde_json = \"1\"`",
    )
}

/// Infer upstream from the **project** `Cargo.toml` when the glue crate does not exist yet.
/// Uses `[dependencies]` and `[dev-dependencies]`, skips Tish toolchain path crates (`tishlang_core`, …),
/// and only keeps entries with a registry semver (skips bare `path =` / `git` deps).
pub fn parse_upstream_from_root_package_cargo(cargo_toml_path: &Path) -> Result<(String, String), String> {
    let s = fs::read_to_string(cargo_toml_path).map_err(|e| e.to_string())?;
    let root: toml::Value = toml::from_str(&s).map_err(|e| format!("{}: {}", cargo_toml_path.display(), e))?;

    let mut candidates = root
        .get("dependencies")
        .and_then(|d| d.as_table())
        .map(collect_upstream_candidates_from_table_root)
        .unwrap_or_default();
    if let Some(dev) = root.get("dev-dependencies").and_then(|d| d.as_table()) {
        candidates.extend(collect_upstream_candidates_from_table_root(dev));
    }

    // Same name in both tables: prefer first occurrence (dependencies before dev-dependencies).
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|(n, _)| seen.insert(n.clone()));

    disambiguate_upstream_candidates(
        candidates,
        cargo_toml_path,
        " — add a registry dep for the crate to wrap (e.g. serde_json = \"1\") or use --dependency",
    )
}

/// Full inference: `rustDependencies` path + glue `Cargo.toml`, or project-root `Cargo.toml` if glue is missing.
pub fn infer_from_project_root(
    project_root: &Path,
    rust_dep_key: Option<&str>,
) -> Result<InferredProjectBindgen, String> {
    let (output_crate_name, out_dir) = select_glue_crate_from_package_json(project_root, rust_dep_key)?;
    let glue_cargo = out_dir.join("Cargo.toml");
    let root_cargo = project_root.join("Cargo.toml");

    let (dependency_name, dependency_version_req) = if glue_cargo.is_file() {
        parse_upstream_from_glue_cargo(&glue_cargo)?
    } else if root_cargo.is_file() {
        parse_upstream_from_root_package_cargo(&root_cargo)?
    } else {
        return Err(format!(
            "no Cargo.toml at glue path {} and none at project root {} — add the upstream crate to the app Cargo.toml ([dependencies] or [dev-dependencies]) or bootstrap with:\n  --project-root {} --dependency <upstream_crate> --dependency-version 1.0",
            glue_cargo.display(),
            root_cargo.display(),
            project_root.display()
        ));
    };

    Ok(InferredProjectBindgen {
        output_crate_name,
        out_dir,
        dependency_name,
        dependency_version_req,
    })
}

/// Paths from `package.json` only (bootstrap before a glue `Cargo.toml` exists).
pub fn infer_glue_paths_only(
    project_root: &Path,
    rust_dep_key: Option<&str>,
) -> Result<(String, PathBuf), String> {
    select_glue_crate_from_package_json(project_root, rust_dep_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_upstream_single_serde_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cargo.toml");
        fs::write(
            &p,
            r#"[package]
name = "glue"
version = "0.1.0"
edition = "2021"
[dependencies]
tishlang_runtime = { path = "../rt" }
serde_json = "1.0.149"
"#,
        )
        .unwrap();
        let (n, v) = parse_upstream_from_glue_cargo(&p).unwrap();
        assert_eq!(n, "serde_json");
        assert_eq!(v, "1.0.149");
    }

    #[test]
    fn parse_upstream_two_deps_prefers_non_serde_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cargo.toml");
        fs::write(
            &p,
            r#"[dependencies]
tishlang_runtime = "0.1"
mylib = "2"
serde_json = "1"
"#,
        )
        .unwrap();
        let (n, _) = parse_upstream_from_glue_cargo(&p).unwrap();
        assert_eq!(n, "mylib");
    }

    #[test]
    fn parse_upstream_from_root_skips_tishlang_core_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cargo.toml");
        fs::write(
            &p,
            r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"
[dependencies]
tishlang_core = { path = "../tish/crates/tish_core" }
serde_json = "1.0.200"
"#,
        )
        .unwrap();
        let (n, v) = parse_upstream_from_root_package_cargo(&p).unwrap();
        assert_eq!(n, "serde_json");
        assert_eq!(v, "1.0.200");
    }

    #[test]
    fn parse_upstream_from_root_merges_dev_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cargo.toml");
        fs::write(
            &p,
            r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"
[dev-dependencies]
serde_json = "1"
"#,
        )
        .unwrap();
        let (n, _) = parse_upstream_from_root_package_cargo(&p).unwrap();
        assert_eq!(n, "serde_json");
    }
}
