//! Module resolver: resolves relative imports, builds dependency graph, detects cycles.
//! Supports native imports: `tish:…`, `cargo:…`, `@scope/pkg` (via package.json).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tishlang_ast::{ExportDeclaration, Expr, ImportSpecifier, Program, Statement};

/// Resolved native module: crate path and init expression.
#[derive(Debug, Clone)]
pub struct ResolvedNativeModule {
    pub spec: String,
    /// Cargo package name (e.g. tish-egui) for [dependencies]
    pub package_name: String,
    /// Rust crate name with underscores (e.g. tish_egui) for use in generated code
    pub crate_name: String,
    pub crate_path: PathBuf,
    pub export_fn: String,
    /// When false, omit `path = …` in the generated Cargo.toml (crate comes from `tish.rustDependencies` only).
    pub use_path_dependency: bool,
}

/// How codegen links a native import to Rust (`generateNativeWrapper` for `tish:*`; `cargo:*` always generated).
#[derive(Debug, Clone)]
pub enum NativeModuleInit {
    /// Call `external_crate::export_fn()` and read named exports from the returned object.
    Legacy {
        crate_name: String,
        export_fn: String,
    },
    /// Call `crate::generated_native::export_fn()` — object built from per-export fns on `shim_crate`.
    Generated {
        shim_crate: String,
        export_fn: String,
    },
}

/// Extra native build inputs produced alongside Rust source (Cargo merge + optional wrapper).
#[derive(Debug, Clone)]
pub struct NativeBuildArtifacts {
    /// Extra `[dependencies]` lines from `tish.rustDependencies`.
    pub rust_dependencies_toml: String,
    /// Generated `generated_native.rs` when using [`NativeModuleInit::Generated`].
    pub generated_native_rs: Option<String>,
    pub native_init: std::collections::HashMap<String, NativeModuleInit>,
}

/// Node-compatible aliases for built-in modules (fs -> tish:fs, etc.).
const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("fs", "tish:fs"),
    ("http", "tish:http"),
    ("process", "tish:process"),
    ("ws", "tish:ws"),
];

/// Normalize built-in spec to canonical form. E.g. "fs" -> "tish:fs".
pub fn normalize_builtin_spec(spec: &str) -> Option<String> {
    if spec.starts_with("tish:") {
        return Some(spec.to_string());
    }
    BUILTIN_ALIASES
        .iter()
        .find(|(alias, _)| *alias == spec)
        .map(|(_, canonical)| (*canonical).to_string())
}

/// Built-in modules that come from tishlang_runtime, not from package.json.
pub fn is_builtin_native_spec(spec: &str) -> bool {
    matches!(spec, "tish:fs" | "tish:http" | "tish:process" | "tish:ws")
        || matches!(spec, "fs" | "http" | "process" | "ws")
}

/// Resolve all native imports in a merged program via package.json lookup.
/// Built-in modules (tish:fs, tish:http, tish:process) are skipped - they use tishlang_runtime directly.
/// Handles both lowered `NativeModuleLoad` (merged modules) and raw `import { … } from 'tish:…'`.
pub fn resolve_native_modules(program: &Program, project_root: &Path) -> Result<Vec<ResolvedNativeModule>, String> {
    let root_canon = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize project root: {}", e))?;
    let mut seen = HashSet::new();
    let mut modules = Vec::new();
    for stmt in &program.statements {
        let specs: Vec<String> = match stmt {
            Statement::VarDecl {
                init: Some(Expr::NativeModuleLoad { spec, .. }),
                ..
            } => vec![spec.as_ref().to_string()],
            Statement::Import { from, .. } if is_native_import(from.as_ref()) => {
                vec![normalize_builtin_spec(from.as_ref()).unwrap_or_else(|| from.to_string())]
            }
            _ => continue,
        };
        for s in specs {
            if is_builtin_native_spec(&s) {
                continue;
            }
            if !seen.insert(s.clone()) {
                continue;
            }
            let m = if s.starts_with("cargo:") {
                resolve_cargo_native_module(&s, &root_canon)?
            } else {
                resolve_native_module(&s, &root_canon)?
            };
            modules.push(m);
        }
    }
    Ok(modules)
}

/// True for `cargo:…` specs (Cargo-backed imports; Rust native backend only).
pub fn is_cargo_native_spec(spec: &str) -> bool {
    spec.starts_with("cargo:")
}

/// Stable Rust symbol for the generated namespace function, e.g. `cargo:my-crate` → `cargo_native_my_crate_object`.
pub fn cargo_export_fn_name(spec: &str) -> String {
    let tail = spec.strip_prefix("cargo:").unwrap_or(spec);
    let mut out = String::from("cargo_native_");
    for c in tail.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out == "cargo_native_" {
        out.push_str("unnamed");
    }
    out.push_str("_object");
    out
}

fn resolve_cargo_native_module(spec: &str, project_root: &Path) -> Result<ResolvedNativeModule, String> {
    let tail = spec
        .strip_prefix("cargo:")
        .ok_or_else(|| format!("Invalid cargo native spec: {}", spec))?;
    if tail.is_empty() {
        return Err("cargo: import needs a dependency name, e.g. import { x } from 'cargo:serde_json'".into());
    }
    let dep_key = tail.to_string();
    let tish = read_project_tish_config(project_root);
    let rust_deps = tish.get("rustDependencies").and_then(|v| v.as_object()).ok_or_else(|| {
        format!(
            "cargo:{} requires package.json \"tish\": {{ \"rustDependencies\": {{ \"{}\": \"…\" }} }}",
            tail, dep_key
        )
    })?;
    if !rust_deps.contains_key(&dep_key) {
        return Err(format!(
            "cargo:{}: add \"{}\" to tish.rustDependencies in package.json (version string or inline table)",
            tail, dep_key
        ));
    }
    let crate_name = dep_key.replace('-', "_");
    let export_fn = cargo_export_fn_name(spec);
    let crate_path = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    Ok(ResolvedNativeModule {
        spec: spec.to_string(),
        package_name: dep_key.clone(),
        crate_name,
        crate_path,
        export_fn,
        use_path_dependency: false,
    })
}

fn resolve_native_module(spec: &str, project_root: &Path) -> Result<ResolvedNativeModule, String> {
    let package_name = if spec.starts_with("tish:") {
        format!("tish-{}", spec.strip_prefix("tish:").unwrap_or(spec))
    } else if spec.starts_with('@') {
        spec.to_string()
    } else {
        return Err(format!("Unsupported native import spec: {}", spec));
    };
    let pkg_dir = find_package_dir(&package_name, project_root)?;
    let pkg_json = pkg_dir.join("package.json");
    let content = std::fs::read_to_string(&pkg_json)
        .map_err(|e| format!("Cannot read {}: {}", pkg_json.display(), e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Invalid JSON in {}: {}", pkg_json.display(), e))?;
    let tish = json
        .get("tish")
        .and_then(|v| v.as_object())
        .ok_or_else(|| format!("Package {} has no \"tish\" config in package.json", package_name))?;
    if !tish.get("module").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(format!("Package {} is not a Tish native module (tish.module must be true)", package_name));
    }
    let raw_crate = tish
        .get("crate")
        .and_then(|v| v.as_str())
        .unwrap_or(&package_name)
        .to_string();
    let module_part = spec.strip_prefix("tish:").unwrap_or(spec);
    let export_fn = tish
        .get("export")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("{}_object", str::replace(module_part, "-", "_")));
    let crate_path = pkg_dir.canonicalize().unwrap_or(pkg_dir);
    Ok(ResolvedNativeModule {
        spec: spec.to_string(),
        package_name: raw_crate.clone(),
        crate_name: raw_crate.replace('-', "_"),
        crate_path,
        export_fn,
        use_path_dependency: true,
    })
}

/// Read the `tish` object from the project root `package.json` (empty JSON object if missing).
pub fn read_project_tish_config(project_root: &Path) -> serde_json::Value {
    let path = project_root.join("package.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return serde_json::json!({});
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return serde_json::json!({});
    };
    json.get("tish").cloned().unwrap_or_else(|| serde_json::json!({}))
}

fn resolve_cargo_path_for_toml(project_root: &Path, raw: &str) -> String {
    let p = Path::new(raw);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else {
        project_root.join(p)
    };
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    resolved.display().to_string().replace('\\', "/")
}

fn json_to_cargo_inline_value(v: &serde_json::Value, project_root: &Path) -> Result<String, String> {
    match v {
        serde_json::Value::String(s) => Ok(format!("{:?}", s.as_str())),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Array(arr) => {
            let mut inner = Vec::new();
            for item in arr {
                inner.push(json_to_cargo_inline_value(item, project_root)?);
            }
            Ok(format!("[{}]", inner.join(", ")))
        }
        serde_json::Value::Object(map) => {
            let mut parts = Vec::new();
            for (k, v) in map {
                let rhs = if k == "path" && v.as_str().is_some() {
                    let s = v.as_str().unwrap();
                    format!("{:?}", resolve_cargo_path_for_toml(project_root, s))
                } else {
                    json_to_cargo_inline_value(v, project_root)?
                };
                parts.push(format!("{} = {}", k, rhs));
            }
            Ok(format!("{{ {} }}", parts.join(", ")))
        }
        serde_json::Value::Null => Err("null is not valid in a Cargo dependency value".to_string()),
    }
}

/// Serialize `tish.rustDependencies` from project `package.json` into Cargo.toml `[dependencies]` lines.
/// Relative `path = "…"` entries in inline tables are resolved against `project_root` so the temp build crate can find them.
pub fn format_rust_dependencies_toml(tish: &serde_json::Value, project_root: &Path) -> Result<String, String> {
    let Some(obj) = tish.get("rustDependencies").and_then(|v| v.as_object()) else {
        return Ok(String::new());
    };
    let mut out = String::new();
    for (name, val) in obj {
        match val {
            serde_json::Value::String(_) | serde_json::Value::Object(_) => {
                out.push_str(&format!(
                    "{} = {}\n",
                    name,
                    json_to_cargo_inline_value(val, project_root)?
                ));
            }
            _ => {
                return Err(format!(
                    "tish.rustDependencies.{} must be a string (version) or object (inline table)",
                    name
                ));
            }
        }
    }
    Ok(out)
}

/// Map a Tish export name to a Rust identifier (e.g. `readFile` → `read_file`) for shim crate symbols.
pub fn export_name_to_rust_ident(export_name: &str) -> String {
    let mut out = String::new();
    for (i, c) in export_name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        for lower in c.to_lowercase() {
            out.push(lower);
        }
    }
    if out.is_empty() {
        "native_export".to_string()
    } else {
        out
    }
}

/// Collect `(spec, export_name)` for every non-builtin native import in the program.
pub fn infer_native_module_exports(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut map: HashMap<String, HashSet<String>> = HashMap::new();
    for stmt in &program.statements {
        match stmt {
            Statement::VarDecl {
                init: Some(Expr::NativeModuleLoad { spec, export_name, .. }),
                ..
            } => {
                let s = spec.as_ref();
                if is_builtin_native_spec(s) {
                    continue;
                }
                map.entry(s.to_string())
                    .or_default()
                    .insert(export_name.to_string());
            }
            Statement::Import { specifiers, from, .. } if is_native_import(from.as_ref()) => {
                let spec = normalize_builtin_spec(from.as_ref()).unwrap_or_else(|| from.to_string());
                if is_builtin_native_spec(&spec) {
                    continue;
                }
                for sp in specifiers {
                    if let ImportSpecifier::Named { name, .. } = sp {
                        map.entry(spec.clone())
                            .or_default()
                            .insert(name.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    map
}

/// Emit `generated_native.rs` for [`NativeModuleInit::Generated`] modules.
pub fn generate_native_wrapper_rs(
    modules: &[ResolvedNativeModule],
    inferred: &HashMap<String, HashSet<String>>,
    init_by_spec: &HashMap<String, NativeModuleInit>,
) -> String {
    let mut file = String::from(
        "//! Generated by `tish build` — do not edit.\n\
         use std::cell::RefCell;\n\
         use std::rc::Rc;\n\
         use std::sync::Arc;\n\
         use tishlang_runtime::{ObjectMap, Value};\n\n",
    );
    let mut any = false;
    for m in modules {
        let Some(NativeModuleInit::Generated { shim_crate, export_fn }) = init_by_spec.get(&m.spec) else {
            continue;
        };
        let Some(names) = inferred.get(&m.spec) else {
            continue;
        };
        if names.is_empty() {
            continue;
        }
        any = true;
        let mut keys: Vec<_> = names.iter().cloned().collect();
        keys.sort();
        file.push_str(&format!("pub fn {}() -> Value {{\n", export_fn));
        file.push_str("    let mut m = ObjectMap::default();\n");
        for export_name in keys {
            let rust_fn = export_name_to_rust_ident(&export_name);
            let key_lit = format!("{:?}", export_name);
            file.push_str(&format!(
                "    m.insert(Arc::from({}), Value::Function(Rc::new(|args: &[Value]| {{\n        {}::{}(args)\n    }})));\n",
                key_lit, shim_crate, rust_fn
            ));
        }
        file.push_str("    Value::Object(Rc::new(RefCell::new(m)))\n}\n\n");
    }
    if !any {
        return String::new();
    }
    file
}

/// Combine project `package.json`, inferred exports, and resolved native modules into build artifacts.
pub fn compute_native_build_artifacts(
    program: &Program,
    project_root: &Path,
    native_modules: &[ResolvedNativeModule],
) -> Result<NativeBuildArtifacts, String> {
    let tish = read_project_tish_config(project_root);
    let rust_dependencies_toml = format_rust_dependencies_toml(&tish, project_root)?;
    let inferred = infer_native_module_exports(program);
    let gen_tish = tish
        .get("generateNativeWrapper")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut native_init: HashMap<String, NativeModuleInit> = HashMap::new();
    for m in native_modules {
        let use_gen = if is_cargo_native_spec(&m.spec) {
            inferred.get(&m.spec).map(|s| !s.is_empty()).unwrap_or(false)
        } else {
            gen_tish && inferred.get(&m.spec).map(|s| !s.is_empty()).unwrap_or(false)
        };
        let init = if use_gen {
            NativeModuleInit::Generated {
                shim_crate: m.crate_name.clone(),
                export_fn: m.export_fn.clone(),
            }
        } else {
            NativeModuleInit::Legacy {
                crate_name: m.crate_name.clone(),
                export_fn: m.export_fn.clone(),
            }
        };
        native_init.insert(m.spec.clone(), init);
    }

    let generated_native_rs = {
        let s = generate_native_wrapper_rs(native_modules, &inferred, &native_init);
        if s.trim().is_empty() {
            None
        } else {
            Some(s)
        }
    };

    Ok(NativeBuildArtifacts {
        rust_dependencies_toml,
        generated_native_rs,
        native_init,
    })
}

fn find_package_dir(package_name: &str, project_root: &Path) -> Result<PathBuf, String> {
    let mut search = project_root.to_path_buf();
    loop {
        let node_mod = search.join("node_modules").join(package_name);
        if node_mod.join("package.json").exists()
            && read_package_name(&node_mod.join("package.json")) == Some(package_name.to_string())
        {
            return Ok(node_mod);
        }
        let sibling = search.join(package_name);
        if sibling.join("package.json").exists()
            && read_package_name(&sibling.join("package.json")) == Some(package_name.to_string())
        {
            return Ok(sibling);
        }
        if search.join("package.json").exists()
            && read_package_name(&search.join("package.json")) == Some(package_name.to_string())
        {
            return Ok(search);
        }
        if let Some(parent) = search.parent() {
            search = parent.to_path_buf();
        } else {
            break;
        }
    }
    Err(format!(
        "Native module {} not found. Add it as a dependency or place it in node_modules/ or as a sibling directory.",
        package_name
    ))
}

fn read_package_name(pkg_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("name").and_then(|v| v.as_str()).map(String::from)
}

fn stmt_native_specs(stmt: &Statement) -> Vec<String> {
    match stmt {
        Statement::VarDecl {
            init: Some(Expr::NativeModuleLoad { spec, .. }),
            ..
        } => vec![spec.to_string()],
        Statement::Import { from, .. } if is_native_import(from.as_ref()) => {
            vec![normalize_builtin_spec(from.as_ref()).unwrap_or_else(|| from.to_string())]
        }
        _ => vec![],
    }
}

/// Extract Cargo feature names from native imports in a merged program.
/// Used to enable tishlang_runtime features based on `import { x } from 'tish:egui'` etc.
pub fn extract_native_import_features(program: &Program) -> Vec<String> {
    let mut features = std::collections::HashSet::new();
    for stmt in &program.statements {
        for spec in stmt_native_specs(stmt) {
            if let Some(f) = native_spec_to_feature(spec.as_ref()) {
                features.insert(f);
            }
        }
    }
    features.into_iter().collect()
}

/// Returns true if the merged program contains native imports (tish:*, @scope/pkg).
pub fn has_native_imports(program: &Program) -> bool {
    program
        .statements
        .iter()
        .any(|stmt| !stmt_native_specs(stmt).is_empty())
}

/// Returns true if the merged program contains external native imports (not built-in tish:fs/http/process).
/// Cranelift/LLVM reject these; bytecode VM supports built-ins only.
pub fn has_external_native_imports(program: &Program) -> bool {
    for stmt in &program.statements {
        for spec in stmt_native_specs(stmt) {
            if !is_builtin_native_spec(spec.as_ref()) {
                return true;
            }
        }
    }
    false
}

/// A resolved module: path and its parsed program.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub path: PathBuf,
    pub program: Program,
}

/// Resolve all modules starting from the entry file. Returns modules in dependency order
/// (dependencies first, then dependents). Entry module is last.
pub fn resolve_project(
    entry_path: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<ResolvedModule>, String> {
    let project_root = project_root.unwrap_or_else(|| entry_path.parent().unwrap_or(Path::new(".")));
    let entry_canon = entry_path
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize entry {}: {}", entry_path.display(), e))?;
    let root_canon = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize project root {}: {}", project_root.display(), e))?;

    let mut visited = HashSet::new();
    let mut path_to_module: HashMap<PathBuf, Program> = HashMap::new();
    let mut load_order: Vec<PathBuf> = Vec::new();

    load_module_recursive(
        &entry_canon,
        &root_canon,
        &mut visited,
        &mut path_to_module,
        &mut load_order,
    )?;

    Ok(load_order
        .into_iter()
        .map(|p| {
            let program = path_to_module.remove(&p).unwrap();
            ResolvedModule { path: p, program }
        })
        .collect())
}

/// Resolve modules when the entry program is read from stdin (`tish run -`).
/// Relative file imports resolve from `project_root` (typically [`std::env::current_dir()`]).
/// The synthetic entry path `<stdin>` is not a real file; dependencies load from disk as usual.
pub fn resolve_project_from_stdin(
    source: &str,
    project_root: &Path,
) -> Result<Vec<ResolvedModule>, String> {
    let root_canon = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize project root {}: {}", project_root.display(), e))?;

    let stdin_path = root_canon.join("<stdin>");
    let program = tishlang_parser::parse(source)
        .map_err(|e| format!("Parse error (stdin): {}", e))?;

    let mut visited = HashSet::new();
    let mut path_to_module: HashMap<PathBuf, Program> = HashMap::new();
    let mut load_order: Vec<PathBuf> = Vec::new();

    let from_dir = stdin_path
        .parent()
        .unwrap_or_else(|| Path::new("."));

    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from.as_ref()) {
                continue;
            }
            let dep_path = resolve_import_path(from.as_ref(), from_dir, &root_canon)?;
            if !path_to_module.contains_key(&dep_path) {
                load_module_recursive(
                    &dep_path,
                    &root_canon,
                    &mut visited,
                    &mut path_to_module,
                    &mut load_order,
                )?;
            }
        }
    }

    path_to_module.insert(stdin_path.clone(), program);
    load_order.push(stdin_path);

    Ok(load_order
        .into_iter()
        .map(|p| {
            let program = path_to_module.remove(&p).unwrap();
            ResolvedModule { path: p, program }
        })
        .collect())
}

fn load_module_recursive(
    module_path: &Path,
    project_root: &Path,
    visited: &mut HashSet<PathBuf>,
    path_to_module: &mut HashMap<PathBuf, Program>,
    load_order: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let canonical = module_path
        .canonicalize()
        .map_err(|e| format!("Cannot read {}: {}", module_path.display(), e))?;

    if visited.contains(&canonical) {
        return Ok(());
    }
    visited.insert(canonical.clone());

    let source = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("Cannot read {}: {}", canonical.display(), e))?;
    let program = tishlang_parser::parse(&source)
        .map_err(|e| format!("Parse error in {}: {}", canonical.display(), e))?;

    // Collect imports and load dependencies first (skip native imports)
    let dir = canonical.parent().unwrap_or(Path::new("."));
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from.as_ref()) {
                continue; // Native imports don't load files
            }
            let dep_path = resolve_import_path(from.as_ref(), dir, project_root)?;
            if !path_to_module.contains_key(&dep_path) {
                load_module_recursive(
                    &dep_path,
                    project_root,
                    visited,
                    path_to_module,
                    load_order,
                )?;
            }
        }
    }

    path_to_module.insert(canonical.clone(), program);
    load_order.push(canonical);
    Ok(())
}

/// Returns true for native module imports that don't resolve to files.
/// - fs, http, process, ws (Node-compatible aliases for tish:fs, tish:http, tish:process, tish:ws)
/// - tish:egui, tish:polars, etc.
/// - cargo:… (Cargo `rustDependencies` + generated wrapper; Rust native backend)
/// - @scope/package (npm-style)
pub fn is_native_import(spec: &str) -> bool {
    spec.starts_with("tish:")
        || spec.starts_with("cargo:")
        || spec.starts_with('@')
        || matches!(spec, "fs" | "http" | "process" | "ws")
}

/// Map native spec to Cargo feature name for built-in tish:* modules.
pub fn native_spec_to_feature(spec: &str) -> Option<String> {
    let canonical = normalize_builtin_spec(spec)?;
    canonical.strip_prefix("tish:").map(|s| s.to_string())
}

/// Resolve a bare specifier (e.g. "lattish") to a path via node_modules.
fn resolve_bare_spec(spec: &str, from_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let mut search = from_dir.to_path_buf();
    loop {
        let node_mod = search.join("node_modules").join(spec);
        let pkg_json = node_mod.join("package.json");
        if pkg_json.exists() {
            if let Some(name) = read_package_name(&pkg_json) {
                if name == spec {
                    let content = std::fs::read_to_string(&pkg_json).ok()?;
                    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
                    let entry = json
                        .get("tish")
                        .and_then(|t| t.get("module"))
                        .and_then(|m| m.as_str())
                        .or_else(|| json.get("main").and_then(|m| m.as_str()));
                    let entry = entry.unwrap_or("index.tish");
                    let entry_clean = entry.trim_start_matches("./");
                    let resolved = node_mod.join(entry_clean);
                    if resolved.exists() {
                        return resolved.canonicalize().ok();
                    }
                }
            }
        }
        if let Some(parent) = search.parent() {
            if parent == search {
                break;
            }
            search = parent.to_path_buf();
        } else {
            break;
        }
    }
    None
}

/// Resolve an import specifier (e.g. "./foo.tish", "../lib/utils", "lattish") to an absolute path.
fn resolve_import_path(
    spec: &str,
    from_dir: &Path,
    project_root: &Path,
) -> Result<PathBuf, String> {
    if is_native_import(spec) {
        return Err(format!(
            "resolve_import_path called for native import (use merge_modules native branch): {}",
            spec
        ));
    }
    if !spec.starts_with("./") && !spec.starts_with("../") {
        if let Some(path) = resolve_bare_spec(spec, from_dir, project_root) {
            return Ok(path);
        }
        return Err(format!(
            "Package '{}' not found in node_modules. Install it with: npm install {}",
            spec, spec
        ));
    }
    let base = from_dir.join(spec);
    // Try with .tish extension if the path has no extension
    let path = if base.extension().is_none() {
        let with_ext = base.with_extension("tish");
        if with_ext.exists() {
            with_ext
        } else {
            base
        }
    } else {
        base
    };
    path.canonicalize().map_err(|e| {
        format!(
            "Cannot resolve import '{}' from {}: {}",
            spec,
            from_dir.display(),
            e
        )
    })
}

/// Check for cyclic imports. Returns Err if a cycle is detected.
pub fn detect_cycles(modules: &[ResolvedModule]) -> Result<(), String> {
    let path_to_idx: HashMap<_, _> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.path.clone(), i))
        .collect();

    for (idx, module) in modules.iter().enumerate() {
        let dir = module.path.parent().unwrap_or(Path::new("."));
        let mut stack = vec![idx];
        if has_cycle_from(
            dir,
            &module.program,
            &path_to_idx,
            modules,
            &mut stack,
            &mut HashSet::new(),
        )? {
            let path_names: Vec<_> = stack
                .iter()
                .map(|&i| modules[i].path.display().to_string())
                .collect();
            return Err(format!("Circular import detected: {}", path_names.join(" -> ")));
        }
    }
    Ok(())
}

fn has_cycle_from(
    from_dir: &Path,
    program: &Program,
    path_to_idx: &HashMap<PathBuf, usize>,
    modules: &[ResolvedModule],
    stack: &mut Vec<usize>,
    visiting: &mut HashSet<usize>,
) -> Result<bool, String> {
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from.as_ref()) {
                continue;
            }
            let dep_path = resolve_import_path(from.as_ref(), from_dir, Path::new("."))?;
            if let Some(&dep_idx) = path_to_idx.get(&dep_path) {
                if stack.contains(&dep_idx) {
                    stack.push(dep_idx);
                    return Ok(true);
                }
                if !visiting.contains(&dep_idx) {
                    visiting.insert(dep_idx);
                    stack.push(dep_idx);
                    let dep = &modules[dep_idx];
                    let dep_dir = dep.path.parent().unwrap_or(Path::new("."));
                    if has_cycle_from(
                        dep_dir,
                        &dep.program,
                        path_to_idx,
                        modules,
                        stack,
                        visiting,
                    )? {
                        return Ok(true);
                    }
                    stack.pop();
                    visiting.remove(&dep_idx);
                }
            }
        }
    }
    Ok(false)
}

/// Merge all resolved modules into a single program. Dependencies are emitted first.
/// Import statements are rewritten as bindings from already-emitted dep exports.
/// Export statements are unwrapped (the inner declaration is emitted).
pub fn merge_modules(modules: Vec<ResolvedModule>) -> Result<Program, String> {
    let path_to_idx: HashMap<PathBuf, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.path.canonicalize().unwrap_or(m.path.clone()), i))
        .collect();

    let mut module_exports: Vec<HashMap<String, String>> = vec![HashMap::new(); modules.len()];
    for (idx, module) in modules.iter().enumerate() {
        for stmt in &module.program.statements {
            if let Statement::Export { declaration, .. } = stmt {
                match declaration.as_ref() {
                    ExportDeclaration::Named(s) => {
                        let name = match s.as_ref() {
                            Statement::VarDecl { name, .. } | Statement::FunDecl { name, .. } => {
                                name.to_string()
                            }
                            _ => continue,
                        };
                        module_exports[idx].insert(name.clone(), name);
                    }
                    ExportDeclaration::Default(_) => {
                        let default_name = format!("__default_{}", idx);
                        module_exports[idx].insert("default".to_string(), default_name);
                    }
                }
            }
        }
    }

    let mut statements = Vec::new();
    for (idx, module) in modules.iter().enumerate() {
        let dir = module.path.parent().unwrap_or(Path::new("."));
        for stmt in &module.program.statements {
            match stmt {
                Statement::Import { specifiers, from, span } => {
                    if is_native_import(from.as_ref()) {
                        // Normalize fs/http/process -> tish:fs etc. for Node compatibility
                        let canonical_spec =
                            normalize_builtin_spec(from.as_ref())
                                .unwrap_or_else(|| from.to_string());
                        // Emit VarDecl with NativeModuleLoad for each specifier
                        for spec in specifiers {
                            match spec {
                                ImportSpecifier::Named { name, alias } => {
                                    let bind = alias.as_deref().unwrap_or(name.as_ref());
                                    let init = Expr::NativeModuleLoad {
                                        spec: Arc::from(canonical_spec.clone()),
                                        export_name: name.clone(),
                                        span: *span,
                                    };
                                    statements.push(Statement::VarDecl {
                                        name: Arc::from(bind),
                                        mutable: false,
                                        type_ann: None,
                                        init: Some(init),
                                        span: *span,
                                    });
                                }
                                ImportSpecifier::Namespace(ns) => {
                                    return Err(format!(
                                        "Namespace import (* as {}) not supported for native module '{}'",
                                        ns.as_ref(),
                                        from.as_ref()
                                    ));
                                }
                                ImportSpecifier::Default(bind) => {
                                    return Err(format!(
                                        "Default import '{}' not supported for native module '{}'. Use named import, e.g. import {{ egui }} from '{}'",
                                        bind.as_ref(),
                                        from.as_ref(),
                                        from.as_ref()
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    let dep_path = resolve_import_path(from.as_ref(), dir, Path::new("."))?;
                    let dep_path = dep_path
                        .canonicalize()
                        .unwrap_or(dep_path);
                    let dep_idx = *path_to_idx
                        .get(&dep_path)
                        .ok_or_else(|| format!("Resolved import '{}' not in module list", from))?;
                    let dep_exports = &module_exports[dep_idx];
                    for spec in specifiers {
                        match spec {
                            ImportSpecifier::Named { name, alias } => {
                                let source = dep_exports
                                    .get(name.as_ref())
                                    .cloned()
                                    .unwrap_or_else(|| name.to_string());
                                let bind = alias.as_deref().unwrap_or(name.as_ref());
                                if bind != source {
                                    statements.push(Statement::VarDecl {
                                        name: Arc::from(bind),
                                        mutable: false,
                                        type_ann: None,
                                        init: Some(Expr::Ident {
                                            name: Arc::from(source),
                                            span: *span,
                                        }),
                                        span: *span,
                                    });
                                }
                            }
                            ImportSpecifier::Namespace(ns) => {
                                let mut props = Vec::new();
                                for (k, v) in dep_exports {
                                    props.push(tishlang_ast::ObjectProp::KeyValue(
                                        Arc::from(k.clone()),
                                        Expr::Ident {
                                            name: Arc::from(v.clone()),
                                            span: *span,
                                        },
                                    ));
                                }
                                statements.push(Statement::VarDecl {
                                    name: ns.clone(),
                                    mutable: false,
                                    type_ann: None,
                                    init: Some(Expr::Object {
                                        props,
                                        span: *span,
                                    }),
                                    span: *span,
                                });
                            }
                            ImportSpecifier::Default(bind) => {
                                let source = dep_exports
                                    .get("default")
                                    .cloned()
                                    .ok_or_else(|| {
                                        format!("Module '{}' has no default export", from)
                                    })?;
                                statements.push(Statement::VarDecl {
                                    name: bind.clone(),
                                    mutable: false,
                                    type_ann: None,
                                    init: Some(Expr::Ident {
                                        name: Arc::from(source),
                                        span: *span,
                                    }),
                                    span: *span,
                                });
                            }
                        }
                    }
                }
                Statement::Export { declaration, .. } => {
                    match declaration.as_ref() {
                        ExportDeclaration::Named(s) => statements.push(*s.clone()),
                        ExportDeclaration::Default(e) => {
                            let default_name = format!("__default_{}", idx);
                            statements.push(Statement::VarDecl {
                                name: Arc::from(default_name),
                                mutable: false,
                                type_ann: None,
                                init: Some((*e).clone()),
                                span: e.span(),
                            });
                        }
                    }
                }
                _ => statements.push(stmt.clone()),
            }
        }
    }
    Ok(Program { statements })
}

#[cfg(test)]
mod cargo_spec_tests {
    use std::sync::Arc;

    use super::cargo_export_fn_name;
    use super::is_native_import;

    #[test]
    fn is_native_import_accepts_arc_str_ref() {
        let from: &Arc<str> = &Arc::from("cargo:demo_shim");
        assert!(is_native_import(from));
    }

    #[test]
    fn detect_cycles_skips_cargo_import() {
        use super::{detect_cycles, resolve_project};
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("main.tish");
        let src = "import { greet } from 'cargo:demo_shim'\nconsole.log(1)\n";
        std::fs::write(&p, src).unwrap();
        let root = dir.path();
        let modules = resolve_project(&p, Some(root)).unwrap();
        detect_cycles(&modules).unwrap();
    }

    #[test]
    fn merge_modules_skips_cargo_import() {
        use super::{merge_modules, resolve_project};
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("main.tish");
        let src = "import { greet } from 'cargo:demo_shim'\nconsole.log(1)\n";
        std::fs::write(&p, src).unwrap();
        let root = dir.path();
        let modules = resolve_project(&p, Some(root)).unwrap();
        merge_modules(modules).unwrap();
    }

    #[test]
    fn cargo_export_fn_name_sanitizes() {
        assert_eq!(
            cargo_export_fn_name("cargo:serde_json"),
            "cargo_native_serde_json_object"
        );
        assert_eq!(
            cargo_export_fn_name("cargo:my-crate"),
            "cargo_native_my_crate_object"
        );
    }
}
