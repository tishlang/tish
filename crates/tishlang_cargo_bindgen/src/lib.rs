//! Generate Rust glue for Tish `cargo:` imports by **reading the dependency crate’s source**
//! (via `cargo metadata` + `syn`), classifying each `pub fn` by **signature shape**, then emitting
//! `pub fn …(args: &[Value]) -> Value` shims.
//!
//! This avoids a fixed catalog of crate names: behavior follows **discovered** `pub fn` signatures
//! (e.g. `Serialize` + `Result<String, _>`, or `&str` + `Deserialize` + `Result`).
//!
//! **Standalone use:** the CLI does not link `tishlang_runtime` (it only emits source). For generated
//! crates, pass **`--tishlang-runtime-version`** so `Cargo.toml` uses a crates.io semver requirement
//! instead of a `path` into the Tish repo (no workspace checkout required to **build** the glue crate).
//!
//! **Project mode (default):** with **`--project-root`** and **no** `--dependency`, the tool reads
//! **`package.json` → `tish.rustDependencies`**, picks the path-based glue crate, and reads the
//! **upstream** crate + semver from that glue crate’s **`Cargo.toml`** `[dependencies]` if it
//! exists; otherwise from the **project root** **`Cargo.toml`** (`[dependencies]` and
//! **`[dev-dependencies]`**), skipping **`tishlang_core`** / **`tishlang_runtime`** and path-only entries.

mod classify;
mod discover;
pub mod infer;
mod metadata;

pub use classify::SignatureClass;
pub use discover::rust_public_fn_location;
pub use metadata::{resolve_dependency_from_manifest, resolve_registry_dependency, ResolvedDependency};

use std::fs;
use std::io;
use std::path::Path;

use classify::classify_public_fn;

/// How the generated crate depends on `tishlang_runtime`.
#[derive(Debug, Clone)]
pub enum TishlangRuntimeDep {
    /// `tishlang_runtime = { path = "..." }` (relative to `--out-dir`).
    Path(String),
    /// `tishlang_runtime = "1.0"` style crates.io requirement (standalone builds; publish `tishlang_runtime` first).
    Version(String),
}

/// Configuration for a generated wrapper crate on disk.
#[derive(Debug, Clone)]
pub struct BindgenConfig {
    /// Cargo `[package].name` of the **generated** crate (must match `cargo:` / `rustDependencies`).
    pub output_crate_name: String,
    pub tishlang_runtime: TishlangRuntimeDep,
    pub out_dir: std::path::PathBuf,
    /// Registry dependency name (e.g. `serde_json`) and semver req for the probe + generated dep.
    pub dependency_name: String,
    pub dependency_version_req: String,
    /// Tish export names to wrap (must match a `pub fn` in the dependency).
    pub exports: Vec<String>,
}

/// Full generation: resolve dependency, scan sources, classify, write crate.
pub fn generate_from_registry_dependency(cfg: &BindgenConfig) -> Result<(), String> {
    let resolved = resolve_registry_dependency(&cfg.dependency_name, &cfg.dependency_version_req)?;
    generate_from_resolved(cfg, &resolved)
}

/// Same as [`generate_from_registry_dependency`] but dependency is already in `manifest_path`’s workspace.
pub fn generate_from_manifest(
    cfg: &BindgenConfig,
    manifest_path: &Path,
    dependency_package_name: &str,
) -> Result<(), String> {
    let resolved = resolve_dependency_from_manifest(manifest_path, dependency_package_name)?;
    generate_from_resolved(cfg, &resolved)
}

fn generate_from_resolved(cfg: &BindgenConfig, resolved: &ResolvedDependency) -> Result<(), String> {
    let root = resolved.source_root();
    let fns = discover::discover_public_functions(&root)?;

    let mut need_json_helpers = false;
    let mut emitted = Vec::new();

    for export in &cfg.exports {
        let item = fns.get(export).ok_or_else(|| {
            format!(
                "no public fn `{}` found under {}/src (exports must match a `pub fn` in the dependency sources)",
                export,
                root.display()
            )
        })?;

        let class = classify_public_fn(item).ok_or_else(|| {
            format!(
                "public fn `{}` has no supported signature for automatic binding (need `&[Value] -> Value`, or `&T where T: Serialize` with `Result<String, _>`, or `&str` with `Deserialize` and `Result`)",
                export
            )
        })?;

        match class {
            SignatureClass::SerializeRefToResultString | SignatureClass::DeserializeStrToResult => {
                need_json_helpers = true;
            }
            SignatureClass::TishValueAbi => {}
        }

        emitted.push((export.clone(), class));
    }

    let lib_rs = render_generated_lib(
        &cfg.dependency_name,
        &emitted,
        need_json_helpers,
    )?;

    let cargo_toml = render_output_cargo_toml(
        cfg,
        resolved.version(),
        need_json_helpers,
    )?;

    fs::create_dir_all(cfg.out_dir.join("src")).map_err(|e| e.to_string())?;
    fs::write(cfg.out_dir.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;
    fs::write(cfg.out_dir.join("src").join("lib.rs"), lib_rs).map_err(|e| e.to_string())?;

    Ok(())
}

impl BindgenConfig {
    /// Write using [`generate_from_registry_dependency`].
    pub fn write_files(&self) -> io::Result<()> {
        generate_from_registry_dependency(self).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

fn render_output_cargo_toml(cfg: &BindgenConfig, dep_exact_version: &str, need_serde_json: bool) -> Result<String, String> {
    let rt_line = match &cfg.tishlang_runtime {
        TishlangRuntimeDep::Path(p) => format!(
            "tishlang_runtime = {{ path = {} }}\n",
            toml_string_value(p)
        ),
        TishlangRuntimeDep::Version(req) => format!(
            "tishlang_runtime = {}\n",
            toml_string_value(req)
        ),
    };
    let dep_line = format!(
        "{} = {}\n",
        cfg.dependency_name,
        toml_string_value(dep_exact_version)
    );
    // JSON bridge helpers use `serde_json::Value`; only add a second dep when the upstream crate is not serde_json.
    let extra_serde = if need_serde_json && cfg.dependency_name != "serde_json" {
        "serde_json = \"1.0\"\n"
    } else {
        ""
    };

    Ok(format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"

[dependencies]
{rt_line}{dep_line}{extra_serde}"#,
        name = cfg.output_crate_name,
        rt_line = rt_line,
        dep_line = dep_line,
        extra_serde = extra_serde
    ))
}

fn render_generated_lib(
    dependency_cargo_name: &str,
    exports: &[(String, SignatureClass)],
    need_json_helpers: bool,
) -> Result<String, String> {
    let crate_ident = dependency_cargo_name.replace('-', "_");
    let mut out = String::from(
        "//! Generated by `tishlang-cargo-bindgen` — do not edit.\n\
         //! Bindings inferred from the dependency crate’s `pub fn` signatures (syn + cargo metadata).\n\n\
         use std::cell::RefCell;\n\
         use std::rc::Rc;\n\
         use std::sync::Arc;\n\
         use tishlang_runtime::{ObjectMap, Value, VmRef};\n\n",
    );

    out.push_str(&format!(
        "use {} as _tish_upstream;\n\n",
        crate_ident
    ));

    if need_json_helpers {
        out.push_str(JSON_HELPERS);
    }

    for (name, class) in exports {
        let rust_fn = syn::parse_str::<syn::Ident>(name)
            .map_err(|e| format!("invalid export name {}: {}", name, e))?;
        let block = match class {
            SignatureClass::TishValueAbi => format!(
                "pub fn {name}(args: &[Value]) -> Value {{\n    _tish_upstream::{name}(args)\n}}\n\n",
                name = rust_fn
            ),
            SignatureClass::SerializeRefToResultString => format!(
                "pub fn {name}(args: &[Value]) -> Value {{\n    let Some(v) = args.first() else {{ return Value::Null }};\n    match _tish_upstream::{name}(&tish_to_json(v)) {{\n        Ok(s) => Value::String(Arc::from(s)),\n        Err(_) => Value::Null,\n    }}\n}}\n\n",
                name = rust_fn
            ),
            SignatureClass::DeserializeStrToResult => format!(
                "pub fn {name}(args: &[Value]) -> Value {{\n    let s = match args.first() {{\n        Some(Value::String(x)) => x.as_ref(),\n        _ => return Value::Null,\n    }};\n    match _tish_upstream::{name}::<serde_json::Value>(s) {{\n        Ok(j) => json_to_tish(j),\n        Err(_) => Value::Null,\n    }}\n}}\n\n",
                name = rust_fn
            ),
        };
        out.push_str(&block);
    }

    Ok(out)
}

const JSON_HELPERS: &str = r#"fn tish_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Array(a) => {
            serde_json::Value::Array(a.borrow().iter().map(|x| tish_to_json(x)).collect())
        }
        Value::Object(o) => {
            let mut m = serde_json::Map::new();
            for (k, v) in o.borrow().iter() {
                m.insert(k.to_string(), tish_to_json(v));
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::Value::Null,
    }
}

fn json_to_tish(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::String(s.into()),
        serde_json::Value::Array(a) => Value::Array(VmRef::new(
            a.into_iter().map(json_to_tish).collect(),
        )),
        serde_json::Value::Object(m) => {
            let mut om = ObjectMap::default();
            for (k, v) in m {
                om.insert(Arc::from(k), json_to_tish(v));
            }
            Value::Object(VmRef::new(om))
        }
    }
}

"#;

fn toml_string_value(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

/// Resolve a path to `tishlang_runtime` for writing into generated `Cargo.toml`.
pub fn resolve_runtime_path_for_output(start: &Path) -> Result<String, String> {
    if let Ok(p) = std::env::var("TISHLANG_RUNTIME_PATH") {
        let pb = Path::new(&p);
        if pb.join("Cargo.toml").is_file() {
            return canonical_path_string(pb);
        }
        return Err(format!(
            "TISHLANG_RUNTIME_PATH does not point to tishlang_runtime: {}",
            p
        ));
    }

    let mut dir = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };

    for _ in 0..64 {
        let npm_rt = dir
            .join("node_modules")
            .join("@tishlang")
            .join("tish")
            .join("crates")
            .join("tish_runtime");
        if npm_rt.join("Cargo.toml").is_file() {
            return canonical_path_string(&npm_rt);
        }

        let ws_rt = dir.join("crates").join("tish_runtime");
        if ws_rt.join("Cargo.toml").is_file() {
            return canonical_path_string(&ws_rt);
        }

        if !dir.pop() {
            break;
        }
    }

    Err(
        "Could not find tishlang_runtime (set TISHLANG_RUNTIME_PATH or run from a project with node_modules/@tishlang/tish or a Tish repo checkout)"
            .into(),
    )
}

fn canonical_path_string(p: &Path) -> Result<String, String> {
    p.canonicalize()
        .map_err(|e| format!("Cannot canonicalize {}: {}", p.display(), e))
        .map(|p| p.display().to_string().replace('\\', "/"))
}

/// Relative path from `out_dir` to `runtime` for Cargo.toml `path =`.
pub fn runtime_path_relative_to_out_dir(out_dir: &Path, runtime: impl AsRef<Path>) -> Result<String, String> {
    let abs_rt = runtime
        .as_ref()
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize runtime path: {}", e))?;
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let abs_out = out_dir
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize out dir: {}", e))?;

    pathdiff::diff_paths(&abs_rt, &abs_out)
        .ok_or_else(|| "Could not compute relative path from out_dir to runtime".to_string())
        .map(|p| p.display().to_string().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_lib_contains_tish_abi_forward() {
        let s = render_generated_lib(
            "demo",
            &[("greet".into(), SignatureClass::TishValueAbi)],
            false,
        )
        .unwrap();
        assert!(s.contains("_tish_upstream::greet"));
    }
}
