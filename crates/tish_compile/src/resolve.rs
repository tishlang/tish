//! Module resolver: resolves relative imports, builds dependency graph, detects cycles.
//! Supports native imports: `tish:…`, `cargo:…`, `@scope/pkg` (via package.json).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tishlang_ast::{
    ArrayElement, ArrowBody, CallArg, DestructElement, DestructPattern, ExportDeclaration, Expr,
    FunParam, ImportSpecifier, JsxAttrValue, JsxChild, JsxProp, MemberProp, ObjectProp, Program,
    Statement,
};

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

/// Node-compatible aliases for built-in modules (fs -> tish:fs, etc.). The `node:` prefix
/// (e.g. `node:fs`, `node:fs/promises`) is stripped before lookup.
const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("fs", "tish:fs"),
    ("fs/promises", "tish:fs/promises"),
    ("http", "tish:http"),
    ("timers", "tish:timers"),
    ("process", "tish:process"),
    ("ws", "tish:ws"),
    ("tty", "tish:tty"),
    ("pty", "tish:pty"),
];

/// Normalize built-in spec to canonical form. Handles the Node `node:` prefix
/// (`node:fs` -> `tish:fs`, `node:fs/promises` -> `tish:fs/promises`) and bare aliases.
pub fn normalize_builtin_spec(spec: &str) -> Option<String> {
    // Strip a leading `node:` so `node:fs` resolves the same as `fs`.
    let spec = spec.strip_prefix("node:").unwrap_or(spec);
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
    let spec = spec.strip_prefix("node:").unwrap_or(spec);
    matches!(
        spec,
        "tish:fs"
            | "tish:fs/promises"
            | "tish:http"
            | "tish:timers"
            | "tish:process"
            | "tish:ws"
            | "tish:tty"
            | "tish:pty"
    ) || matches!(
        spec,
        "fs" | "fs/promises" | "http" | "timers" | "process" | "ws" | "tty" | "pty"
    )
}

/// Resolve all native imports in a merged program via package.json lookup.
/// Built-in modules (tish:fs, tish:http, tish:process) are skipped - they use tishlang_runtime directly.
/// Handles both lowered `NativeModuleLoad` (merged modules) and raw `import { … } from 'tish:…'`.
pub fn resolve_native_modules(
    program: &Program,
    project_root: &Path,
) -> Result<Vec<ResolvedNativeModule>, String> {
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

/// True when merged Tish source references the browser global `document` (e.g. juke-cards).
pub fn program_uses_document(program: &Program) -> bool {
    use tishlang_ast::{ArrayElement, ArrowBody, JsxAttrValue, JsxChild, JsxProp, ObjectProp};

    fn expr_uses_document(e: &Expr) -> bool {
        match e {
            Expr::Ident { name, .. } => name.as_ref() == "document",
            Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => false,
            Expr::Binary { left, right, .. } => {
                expr_uses_document(left) || expr_uses_document(right)
            }
            Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } => {
                expr_uses_document(operand)
            }
            Expr::Delete { target, .. } => expr_uses_document(target),
            Expr::Call { callee, args, .. } => {
                expr_uses_document(callee)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => expr_uses_document(e),
                    })
            }
            Expr::New { callee, args, .. } => {
                expr_uses_document(callee)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => expr_uses_document(e),
                    })
            }
            Expr::Member { object, prop, .. } => {
                expr_uses_document(object)
                    || if let MemberProp::Expr(e) = prop {
                        expr_uses_document(e)
                    } else {
                        false
                    }
            }
            Expr::Index { object, index, .. } => {
                expr_uses_document(object) || expr_uses_document(index)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                expr_uses_document(cond)
                    || expr_uses_document(then_branch)
                    || expr_uses_document(else_branch)
            }
            Expr::NullishCoalesce { left, right, .. } => {
                expr_uses_document(left) || expr_uses_document(right)
            }
            Expr::Array { elements, .. } => elements.iter().any(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_uses_document(e),
            }),
            Expr::Object { props, .. } => props.iter().any(|p| match p {
                ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => expr_uses_document(e),
            }),
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. }
            | Expr::MemberAssign { value, .. }
            | Expr::IndexAssign { value, .. } => expr_uses_document(value),
            Expr::PostfixInc { .. }
            | Expr::PostfixDec { .. }
            | Expr::PrefixInc { .. }
            | Expr::PrefixDec { .. } => false,
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => expr_uses_document(e),
                ArrowBody::Block(s) => stmt_uses_document(s),
            },
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(expr_uses_document),
            Expr::Await { operand, .. } => expr_uses_document(operand),
            Expr::JsxElement { props, children, .. } => {
                props.iter().any(|p| match p {
                    JsxProp::Attr { value, .. } => match value {
                        JsxAttrValue::Expr(e) => expr_uses_document(e),
                        JsxAttrValue::String(_) | JsxAttrValue::ImplicitTrue => false,
                    },
                    JsxProp::Spread(e) => expr_uses_document(e),
                }) || children.iter().any(|c| match c {
                    JsxChild::Expr(e) => expr_uses_document(e),
                    JsxChild::Text(_) => false,
                })
            }
            Expr::JsxFragment { children, .. } => children.iter().any(|c| match c {
                JsxChild::Expr(e) => expr_uses_document(e),
                JsxChild::Text(_) => false,
            }),
        }
    }

    fn stmt_uses_document(s: &Statement) -> bool {
        match s {
            Statement::VarDecl { init, .. } => init.as_ref().is_some_and(expr_uses_document),
            Statement::VarDeclDestructure { init, .. } => expr_uses_document(init),
            Statement::ExprStmt { expr, .. } => expr_uses_document(expr),
            Statement::Return { value, .. } => value.as_ref().is_some_and(expr_uses_document),
            Statement::Throw { value, .. } => expr_uses_document(value),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                expr_uses_document(cond)
                    || stmt_uses_document(then_branch)
                    || else_branch
                        .as_ref()
                        .is_some_and(|b| stmt_uses_document(b.as_ref()))
            }
            Statement::While { cond, body, .. }
            | Statement::DoWhile { cond, body, .. } => {
                expr_uses_document(cond) || stmt_uses_document(body)
            }
            Statement::For { init, cond, update, body, .. } => {
                init.as_ref().is_some_and(|s| stmt_uses_document(s.as_ref()))
                    || cond.as_ref().is_some_and(expr_uses_document)
                    || update.as_ref().is_some_and(expr_uses_document)
                    || stmt_uses_document(body)
            }
            Statement::ForOf { iterable, body, .. }
            | Statement::ForIn {
                object: iterable,
                body,
                ..
            } => expr_uses_document(iterable) || stmt_uses_document(body),
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                expr_uses_document(expr)
                    || cases.iter().any(|(e, stmts)| {
                        e.as_ref().is_some_and(expr_uses_document)
                            || stmts.iter().any(stmt_uses_document)
                    })
                    || default_body
                        .as_ref()
                        .is_some_and(|stmts| stmts.iter().any(stmt_uses_document))
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(stmt_uses_document)
            }
            Statement::FunDecl { body, .. } => stmt_uses_document(body),
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                stmt_uses_document(body)
                    || catch_body
                        .as_ref()
                        .is_some_and(|b| stmt_uses_document(b.as_ref()))
                    || finally_body
                        .as_ref()
                        .is_some_and(|b| stmt_uses_document(b.as_ref()))
            }
            Statement::Import { .. }
            | Statement::Export { .. }
            | Statement::Break { .. }
            | Statement::Continue { .. }
            | Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => false,
        }
    }

    program.statements.iter().any(stmt_uses_document)
}

/// When Tish uses bare `document`, link `tish-canvas` even without `import from 'tish:canvas'`.
pub fn ensure_tish_canvas_module(
    native_modules: &mut Vec<ResolvedNativeModule>,
    project_root: &Path,
) -> Result<(), String> {
    if native_modules
        .iter()
        .any(|m| m.crate_name == "tish_canvas" || m.package_name == "tish-canvas")
    {
        return Ok(());
    }
    let m = resolve_native_module("tish:canvas", project_root)?;
    native_modules.push(m);
    Ok(())
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

fn resolve_cargo_native_module(
    spec: &str,
    project_root: &Path,
) -> Result<ResolvedNativeModule, String> {
    let tail = spec
        .strip_prefix("cargo:")
        .ok_or_else(|| format!("Invalid cargo native spec: {}", spec))?;
    if tail.is_empty() {
        return Err(
            "cargo: import needs a dependency name, e.g. import { x } from 'cargo:my_crate'".into(),
        );
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
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {}", pkg_json.display(), e))?;
    let tish = json
        .get("tish")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            format!(
                "Package {} has no \"tish\" config in package.json",
                package_name
            )
        })?;
    if !tish
        .get("module")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(format!(
            "Package {} is not a Tish native module (tish.module must be true)",
            package_name
        ));
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
    json.get("tish")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}))
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

fn json_to_cargo_inline_value(
    v: &serde_json::Value,
    project_root: &Path,
) -> Result<String, String> {
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
pub fn format_rust_dependencies_toml(
    tish: &serde_json::Value,
    project_root: &Path,
) -> Result<String, String> {
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
                init:
                    Some(Expr::NativeModuleLoad {
                        spec, export_name, ..
                    }),
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
            Statement::Import {
                specifiers, from, ..
            } if is_native_import(from.as_ref()) => {
                let spec =
                    normalize_builtin_spec(from.as_ref()).unwrap_or_else(|| from.to_string());
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
         use tishlang_runtime::{ObjectMap, Value, VmRef};\n\n",
    );
    let mut any = false;
    for m in modules {
        let Some(NativeModuleInit::Generated {
            shim_crate,
            export_fn,
        }) = init_by_spec.get(&m.spec)
        else {
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
                "    m.insert(Arc::from({}), Value::native(|args: &[Value]| {{\n        {}::{}(args)\n    }}));\n",
                key_lit, shim_crate, rust_fn
            ));
        }
        // `Value::object(m)` wraps the `ObjectMap` into the `ObjectData` that `Value::Object`
        // now holds; `Value::Object(VmRef::new(m))` (raw map) stopped type-checking after the
        // PropMap/ObjectData refactor (#78).
        file.push_str("    Value::object(m)\n}\n\n");
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
            inferred
                .get(&m.spec)
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        } else {
            gen_tish
                && inferred
                    .get(&m.spec)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
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
            // `ffi:` is portable (loadable on every backend via the C ABI), so it is NOT an
            // "external native import" that non-rust backends must reject — only `cargo:` is.
            if !is_builtin_native_spec(spec.as_ref()) && !is_ffi_native_spec(&spec) {
                return true;
            }
        }
    }
    false
}

/// Every `ffi:…` spec imported anywhere in `program` (deduplicated, in first-seen order). The CLI
/// loads each cdylib with `tish_ffi::load_module` and registers it before running.
pub fn ffi_native_specs(program: &Program) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for stmt in &program.statements {
        for spec in stmt_native_specs(stmt) {
            if is_ffi_native_spec(&spec) && !out.contains(&spec) {
                out.push(spec);
            }
        }
    }
    out
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
    let project_root =
        project_root.unwrap_or_else(|| entry_path.parent().unwrap_or(Path::new(".")));
    let entry_canon = entry_path
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize entry {}: {}", entry_path.display(), e))?;
    let root_canon = project_root.canonicalize().map_err(|e| {
        format!(
            "Cannot canonicalize project root {}: {}",
            project_root.display(),
            e
        )
    })?;

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
    let root_canon = project_root.canonicalize().map_err(|e| {
        format!(
            "Cannot canonicalize project root {}: {}",
            project_root.display(),
            e
        )
    })?;

    let stdin_path = root_canon.join("<stdin>");
    let program =
        tishlang_parser::parse(source).map_err(|e| format!("Parse error (stdin): {}", e))?;

    let mut visited = HashSet::new();
    let mut path_to_module: HashMap<PathBuf, Program> = HashMap::new();
    let mut load_order: Vec<PathBuf> = Vec::new();

    let from_dir = stdin_path.parent().unwrap_or_else(|| Path::new("."));

    for stmt in &program.statements {
        if let Some(from) = stmt_module_dep(stmt) {
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

/// #305 — the module path a statement depends on: an `import ... from "p"` or a re-export
/// `export ... from "p"`. Used by dependency discovery + cycle detection so a module reached ONLY
/// through a re-export is still loaded. None for non-module statements.
fn stmt_module_dep(stmt: &Statement) -> Option<&std::sync::Arc<str>> {
    match stmt {
        Statement::Import { from, .. } => Some(from),
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            // A local named export (`export { a }`, from=None) has no module dependency.
            ExportDeclaration::ReExport { from, .. } => from.as_ref(),
            _ => None,
        },
        _ => None,
    }
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
        if let Some(from) = stmt_module_dep(stmt) {
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
/// - fs, http, timers, process, ws (Node-compatible aliases for tish:*)
/// - tish:egui, tish:polars, etc.
/// - cargo:… (Cargo `rustDependencies` + generated wrapper; Rust native backend)
/// - ffi:… (a C-ABI cdylib loaded via `tish_ffi::load_module` — portable across backends)
///
/// Scoped npm packages (`@scope/pkg`) are merged as Tish source unless imported via `tish:…`.
pub fn is_native_import(spec: &str) -> bool {
    // A leading `node:` (node:fs, node:fs/promises) resolves the same as the bare form.
    let spec = spec.strip_prefix("node:").unwrap_or(spec);
    spec.starts_with("tish:")
        || spec.starts_with("cargo:")
        || spec.starts_with("ffi:")
        || matches!(
            spec,
            "fs" | "fs/promises" | "http" | "timers" | "process" | "ws" | "tty"
        )
}

/// True for `ffi:…` specs (portable C-ABI cdylib extensions, loadable on every backend). The
/// path after `ffi:` is resolved relative to the importing program and loaded with
/// `tish_ffi::load_module`. Unlike `cargo:` (rust-AOT only), `ffi:` is allowed everywhere.
pub fn is_ffi_native_spec(spec: &str) -> bool {
    spec.starts_with("ffi:")
}

/// Map native spec to Cargo feature name for built-in tish:* modules.
pub fn native_spec_to_feature(spec: &str) -> Option<String> {
    let canonical = normalize_builtin_spec(spec)?;
    canonical.strip_prefix("tish:").map(|s| s.to_string())
}

/// Resolve `package.json` at `pkg_root` to the package's main `.tish` entry.
///
/// `require_name`: when `Some(spec)`, the package's `package.json` `name` must equal `spec` (used for
/// the sibling / walk-up heuristic, where a coincidental directory name would otherwise false-match).
/// When `None`, the directory is authoritative — used for a `node_modules/<spec>` lookup, because npm
/// installs a dependency under its *dependency key* (the directory), not the package's internal `name`
/// (e.g. an aliased / scoped package, or a `file:`/workspace link). This matches Node's resolution and
/// tish's own path-dep rewriting, so a workspace package linked in as `lattish` resolves even though its
/// own `name` is `@tishlang/lattish`.
fn resolve_package_entry(pkg_root: &Path, require_name: Option<&str>) -> Option<PathBuf> {
    let pkg_json = pkg_root.join("package.json");
    if !pkg_json.exists() {
        return None;
    }
    if let Some(spec) = require_name {
        if read_package_name(&pkg_json).as_deref() != Some(spec) {
            return None;
        }
    }
    let content = std::fs::read_to_string(&pkg_json).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let entry = json
        .get("tish")
        .and_then(|t| t.get("module"))
        .and_then(|m| m.as_str())
        .or_else(|| json.get("main").and_then(|m| m.as_str()))
        .unwrap_or("index.tish");
    let entry_clean = entry.trim_start_matches("./");
    let resolved = pkg_root.join(entry_clean);
    if !resolved.exists() {
        return None;
    }
    match resolved.canonicalize() {
        Ok(p) => Some(p),
        Err(_) => Some(resolved),
    }
}

/// Resolve a bare specifier (e.g. "lattish") to the package entry `.tish` file.
///
/// Walks upward from `from_dir` and, at each level, checks (same order as native [`find_package_dir`]):
/// - `node_modules/<spec>/`
/// - `<spec>/` as a sibling directory (monorepo: `…/tish/tish-candle` next to `…/tish/tish-hub`)
/// - the search directory itself if its `package.json` name matches `spec`
pub fn resolve_bare_spec(spec: &str, from_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let mut search = from_dir.to_path_buf();
    loop {
        // node_modules/<spec>: the directory is authoritative (npm installs by dependency key, like
        // Node) — do NOT require the package's internal `name` to match, so aliased/scoped/`file:`
        // workspace packages linked in under this name resolve.
        if let Some(p) = resolve_package_entry(&search.join("node_modules").join(spec), None) {
            return Some(p);
        }
        // sibling <spec>/ and the search dir itself: require name match (a bare directory name is a
        // weaker signal — guard against a coincidental same-named dir in a monorepo walk).
        if let Some(p) = resolve_package_entry(&search.join(spec), Some(spec)) {
            return Some(p);
        }
        if let Some(p) = resolve_package_entry(&search, Some(spec)) {
            return Some(p);
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
            "Package '{}' not found. Install with `npm install {}`, or place the package under node_modules/ or as a sibling directory (same layout as native `find_package_dir`).",
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
            return Err(format!(
                "Circular import detected: {}",
                path_names.join(" -> ")
            ));
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
        if let Some(from) = stmt_module_dep(stmt) {
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
                    if has_cycle_from(dep_dir, &dep.program, path_to_idx, modules, stack, visiting)?
                    {
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

/// Result of [`merge_modules`]: merged AST plus, per top-level statement, the originating `.tish` file.
#[derive(Debug)]
pub struct MergedProgram {
    pub program: Program,
    pub statement_sources: Vec<PathBuf>,
    /// #295: the ENTRY module's exported `(local_name, exported_name)` pairs, excluding `default`
    /// (handled separately as `export default`). Merge unwraps exports into plain top-level
    /// declarations, so `--target js --format bundle` re-emits these as a trailing `export { … }` to
    /// produce a valid ES module. Sorted by local name for deterministic output.
    pub entry_exports: Vec<(String, String)>,
}

fn merge_push(
    statements: &mut Vec<Statement>,
    statement_sources: &mut Vec<PathBuf>,
    stmt: Statement,
    source: PathBuf,
) {
    statements.push(stmt);
    statement_sources.push(source);
}

// ── #97: module-private top-level binding isolation ─────────────────────────────────────
//
// All modules are concatenated into one flat program, so two modules that declare the same
// *non-exported* top-level name (`let SHARED`, `fn err`, …) collide: a silent wrong value at
// runtime, and a duplicate `let` (SyntaxError) in the `--target js` bundle. We give each such
// private binding a module-unique name and rewrite references within that module. Exported
// and imported names are never touched, so the import/export resolution below is unaffected —
// and a name that doesn't collide is left exactly as-is (zero blast radius).

/// Names a module contributes to the merged flat namespace at its top level.
/// `decls` = names declared (`let`/`const`/`fn`/destructure), `exported` = the subset that is
/// exported, `imports` = local names bound by `import`. Type-only declarations are erased.
fn collect_module_top_level_names(
    stmts: &[Statement],
    decls: &mut HashSet<String>,
    exported: &mut HashSet<String>,
    imports: &mut HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            Statement::VarDecl { name, .. } | Statement::FunDecl { name, .. } => {
                decls.insert(name.to_string());
            }
            Statement::VarDeclDestructure { pattern, .. } => {
                collect_destructure_names(pattern, decls);
            }
            Statement::Multi { statements, .. } => {
                collect_module_top_level_names(statements, decls, exported, imports);
            }
            Statement::Export { declaration, .. } => {
                if let ExportDeclaration::Named(inner) = declaration.as_ref() {
                    match inner.as_ref() {
                        Statement::VarDecl { name, .. } | Statement::FunDecl { name, .. } => {
                            decls.insert(name.to_string());
                            exported.insert(name.to_string());
                        }
                        Statement::VarDeclDestructure { pattern, .. } => {
                            let mut names = HashSet::new();
                            collect_destructure_names(pattern, &mut names);
                            for n in names {
                                decls.insert(n.clone());
                                exported.insert(n);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Statement::Import { specifiers, .. } => {
                for spec in specifiers {
                    let n = match spec {
                        ImportSpecifier::Named { name, alias, .. } => {
                            alias.as_deref().unwrap_or(name).to_string()
                        }
                        ImportSpecifier::Namespace { name, .. }
                        | ImportSpecifier::Default { name, .. } => name.to_string(),
                    };
                    imports.insert(n);
                }
            }
            _ => {}
        }
    }
}

fn collect_destructure_names(pattern: &DestructPattern, out: &mut HashSet<String>) {
    let push = |el: &DestructElement, out: &mut HashSet<String>| match el {
        DestructElement::Ident(n, _) | DestructElement::Rest(n, _) => {
            out.insert(n.to_string());
        }
        DestructElement::Pattern(p) => collect_destructure_names(p, out),
    };
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements.iter().flatten() {
                push(el, out);
            }
        }
        DestructPattern::Object(props) => {
            for p in props {
                push(&p.value, out);
            }
        }
    }
}

/// Rename each module's non-exported top-level bindings whose name also occurs as a top-level
/// name in another module, isolating module-private declarations (#97).
fn isolate_private_top_level_bindings(modules: &mut [ResolvedModule]) {
    let n = modules.len();
    if n < 2 {
        return; // a single module cannot collide with another
    }
    let mut decls: Vec<HashSet<String>> = vec![HashSet::new(); n];
    let mut exported: Vec<HashSet<String>> = vec![HashSet::new(); n];
    // `occupancy[i]` = every top-level name module i contributes (decls ∪ import bindings).
    let mut occupancy: Vec<HashSet<String>> = vec![HashSet::new(); n];
    for (i, m) in modules.iter().enumerate() {
        let mut imports = HashSet::new();
        collect_module_top_level_names(
            &m.program.statements,
            &mut decls[i],
            &mut exported[i],
            &mut imports,
        );
        occupancy[i] = decls[i].union(&imports).cloned().collect();
    }
    // How many modules contribute each top-level name.
    let mut count: HashMap<&str, usize> = HashMap::new();
    for occ in &occupancy {
        for name in occ {
            *count.entry(name.as_str()).or_insert(0) += 1;
        }
    }
    for (i, m) in modules.iter_mut().enumerate() {
        let mut renames: HashMap<String, Arc<str>> = HashMap::new();
        for name in &decls[i] {
            if exported[i].contains(name) {
                continue; // exported names stay stable so imports keep resolving
            }
            if count.get(name.as_str()).copied().unwrap_or(0) > 1 {
                renames.insert(name.clone(), Arc::from(format!("{name}__m{i}")));
            }
        }
        if renames.is_empty() {
            continue;
        }
        for stmt in &mut m.program.statements {
            // Each top-level statement starts from the full rename set (module top level is a
            // single scope; nested scopes shadow within their own cloned set).
            let mut active = renames.clone();
            rewrite_stmt_scope(stmt, &mut active, true);
        }
    }
}

/// Apply the rename for a *declared* name. At module top level the binding is the canonical
/// private one — rename it. In a nested scope the same name is a shadow — drop it from `active`
/// so the inner binding and its references keep their own identity.
fn apply_binding(name: &mut Arc<str>, active: &mut HashMap<String, Arc<str>>, top_level: bool) {
    if top_level {
        if let Some(renamed) = active.get(name.as_ref()) {
            *name = Arc::clone(renamed);
        }
    } else {
        active.remove(name.as_ref());
    }
}

/// Rename / shadow the names bound by a destructuring pattern (mirrors [`apply_binding`]).
fn rewrite_destructure_binding(
    pattern: &mut DestructPattern,
    active: &mut HashMap<String, Arc<str>>,
    top_level: bool,
) {
    fn one(el: &mut DestructElement, active: &mut HashMap<String, Arc<str>>, top_level: bool) {
        match el {
            DestructElement::Ident(n, _) | DestructElement::Rest(n, _) => {
                if top_level {
                    if let Some(renamed) = active.get(n.as_ref()) {
                        *n = Arc::clone(renamed);
                    }
                } else {
                    active.remove(n.as_ref());
                }
            }
            DestructElement::Pattern(p) => rewrite_destructure_binding(p, active, top_level),
        }
    }
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements.iter_mut().flatten() {
                one(el, active, top_level);
            }
        }
        DestructPattern::Object(props) => {
            for p in props.iter_mut() {
                one(&mut p.value, active, top_level); // p.key is the source property — untouched
            }
        }
    }
}

/// Remove function/arrow parameter names from a (child-scope) rename set so the body's
/// references to them are not rewritten, and rewrite any default-value expressions in the
/// enclosing scope.
fn shadow_params(
    params: &mut [FunParam],
    child: &mut HashMap<String, Arc<str>>,
    parent: &HashMap<String, Arc<str>>,
) {
    for p in params.iter_mut() {
        match p {
            FunParam::Simple(tp) => {
                if let Some(d) = &mut tp.default {
                    rewrite_expr_scope(d, parent);
                }
                child.remove(tp.name.as_ref());
            }
            FunParam::Destructure {
                pattern, default, ..
            } => {
                if let Some(d) = default {
                    rewrite_expr_scope(d, parent);
                }
                let mut names = HashSet::new();
                collect_destructure_names(pattern, &mut names);
                for n in &names {
                    child.remove(n);
                }
            }
        }
    }
}

/// Scope-aware statement rewriter for [`isolate_private_top_level_bindings`].
/// Scope-aware statement rewriter: rename declared bindings and their free references
/// according to `active` (name → new Arc). Exposed `pub(crate)` so the #179 factory inliner
/// (infer.rs) can reuse this exhaustive, resolution-tested renamer for alpha-renaming a
/// spliced factory body instead of hand-rolling a completeness-sensitive walker.
pub(crate) fn rewrite_stmt_scope(
    stmt: &mut Statement,
    active: &mut HashMap<String, Arc<str>>,
    top_level: bool,
) {
    match stmt {
        Statement::VarDecl { name, init, .. } => {
            if let Some(e) = init {
                rewrite_expr_scope(e, active);
            }
            apply_binding(name, active, top_level);
        }
        Statement::VarDeclDestructure { pattern, init, .. } => {
            rewrite_expr_scope(init, active);
            rewrite_destructure_binding(pattern, active, top_level);
        }
        Statement::Multi { statements, .. } => {
            // Same-scope group (`let a = 1, b = 2`): thread `active`, keep `top_level`.
            for s in statements {
                rewrite_stmt_scope(s, active, top_level);
            }
        }
        Statement::ExprStmt { expr, .. } => rewrite_expr_scope(expr, active),
        Statement::Block { statements, .. } => {
            let mut child = active.clone();
            for s in statements {
                rewrite_stmt_scope(s, &mut child, false);
            }
        }
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            rewrite_expr_scope(cond, active);
            rewrite_stmt_scope(then_branch, &mut active.clone(), false);
            if let Some(e) = else_branch {
                rewrite_stmt_scope(e, &mut active.clone(), false);
            }
        }
        Statement::While { cond, body, .. } => {
            rewrite_expr_scope(cond, active);
            rewrite_stmt_scope(body, &mut active.clone(), false);
        }
        Statement::DoWhile { body, cond, .. } => {
            rewrite_stmt_scope(body, &mut active.clone(), false);
            rewrite_expr_scope(cond, active);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            let mut child = active.clone();
            if let Some(i) = init {
                rewrite_stmt_scope(i, &mut child, false);
            }
            if let Some(e) = cond {
                rewrite_expr_scope(e, &child);
            }
            if let Some(e) = update {
                rewrite_expr_scope(e, &child);
            }
            rewrite_stmt_scope(body, &mut child, false);
        }
        Statement::ForOf {
            name,
            iterable,
            body,
            ..
        }
        | Statement::ForIn {
            name,
            object: iterable,
            body,
            ..
        } => {
            rewrite_expr_scope(iterable, active);
            let mut child = active.clone();
            child.remove(name.as_ref()); // loop variable shadows
            rewrite_stmt_scope(body, &mut child, false);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                rewrite_expr_scope(e, active);
            }
        }
        Statement::Throw { value, .. } => rewrite_expr_scope(value, active),
        Statement::Break { .. } | Statement::Continue { .. } => {}
        Statement::FunDecl {
            name,
            params,
            rest_param,
            body,
            ..
        } => {
            apply_binding(name, active, top_level);
            let mut child = active.clone();
            shadow_params(params, &mut child, active);
            if let Some(rp) = rest_param {
                child.remove(rp.name.as_ref());
            }
            rewrite_stmt_scope(body, &mut child, false);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            rewrite_expr_scope(expr, active);
            // A switch body shares one block scope across all cases.
            let mut child = active.clone();
            for (test, body) in cases.iter_mut() {
                if let Some(t) = test {
                    rewrite_expr_scope(t, &child);
                }
                for s in body {
                    rewrite_stmt_scope(s, &mut child, false);
                }
            }
            if let Some(body) = default_body {
                for s in body {
                    rewrite_stmt_scope(s, &mut child, false);
                }
            }
        }
        Statement::Try {
            body,
            catch_param,
            catch_body,
            finally_body,
            ..
        } => {
            rewrite_stmt_scope(body, &mut active.clone(), false);
            if let Some(cb) = catch_body {
                let mut child = active.clone();
                if let Some(p) = catch_param {
                    child.remove(p.as_ref()); // catch binding shadows
                }
                rewrite_stmt_scope(cb, &mut child, false);
            }
            if let Some(fb) = finally_body {
                rewrite_stmt_scope(fb, &mut active.clone(), false);
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_mut() {
            // Exported declarations keep their name (not in `active`), but their initializer /
            // body can still reference module-private names that were renamed.
            ExportDeclaration::Named(inner) => rewrite_stmt_scope(inner, active, top_level),
            ExportDeclaration::Default(e) => rewrite_expr_scope(e, active),
            // #305: a re-export has no inner statement/expr to scope-rewrite; names are resolved by
            // the export-table merge against the dep's (already-rewritten) bindings.
            ExportDeclaration::ReExport { .. } => {}
        },
        Statement::Import { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => {}
    }
}

/// Scope-aware expression rewriter: rename every free reference to a name in `active`.
/// Expressions never declare module-level bindings, so `active` is read-only here; arrow
/// functions clone it for their own (shadowed) parameter scope.
/// `pub(crate)` so the #179 factory inliner can rename a spliced return expression.
pub(crate) fn rewrite_expr_scope(expr: &mut Expr, active: &HashMap<String, Arc<str>>) {
    let rename = |name: &mut Arc<str>| {
        if let Some(renamed) = active.get(name.as_ref()) {
            *name = Arc::clone(renamed);
        }
    };
    match expr {
        Expr::Ident { name, .. } => rename(name),
        Expr::Assign { name, value, .. }
        | Expr::CompoundAssign { name, value, .. }
        | Expr::LogicalAssign { name, value, .. } => {
            rename(name);
            rewrite_expr_scope(value, active);
        }
        Expr::PostfixInc { name, .. }
        | Expr::PostfixDec { name, .. }
        | Expr::PrefixInc { name, .. }
        | Expr::PrefixDec { name, .. } => rename(name),
        Expr::Binary { left, right, .. } => {
            rewrite_expr_scope(left, active);
            rewrite_expr_scope(right, active);
        }
        Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } | Expr::Await { operand, .. } => {
            rewrite_expr_scope(operand, active)
        }
        Expr::Delete { target, .. } => rewrite_expr_scope(target, active),
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            rewrite_expr_scope(callee, active);
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => rewrite_expr_scope(e, active),
                }
            }
        }
        Expr::Member { object, prop, .. } => {
            rewrite_expr_scope(object, active);
            if let MemberProp::Expr(e) = prop {
                rewrite_expr_scope(e, active); // computed key; `obj.name` is untouched
            }
        }
        Expr::Index { object, index, .. } => {
            rewrite_expr_scope(object, active);
            rewrite_expr_scope(index, active);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            rewrite_expr_scope(cond, active);
            rewrite_expr_scope(then_branch, active);
            rewrite_expr_scope(else_branch, active);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            rewrite_expr_scope(left, active);
            rewrite_expr_scope(right, active);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    ArrayElement::Expr(e) | ArrayElement::Spread(e) => rewrite_expr_scope(e, active),
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                        rewrite_expr_scope(e, active) // key is a property name; value recurses
                    }
                }
            }
        }
        Expr::MemberAssign { object, value, .. } => {
            rewrite_expr_scope(object, active);
            rewrite_expr_scope(value, active);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            rewrite_expr_scope(object, active);
            rewrite_expr_scope(index, active);
            rewrite_expr_scope(value, active);
        }
        Expr::ArrowFunction { params, body, .. } => {
            let mut child = active.clone();
            shadow_params(params, &mut child, active);
            match body {
                ArrowBody::Expr(e) => rewrite_expr_scope(e, &child),
                ArrowBody::Block(s) => rewrite_stmt_scope(s, &mut child, false),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                rewrite_expr_scope(e, active);
            }
        }
        Expr::JsxElement {
            tag,
            props,
            children,
            ..
        } => {
            // `<Component …>` references a binding; lowercase HTML tags aren't in `active`.
            if let Some(renamed) = active.get(tag.as_ref()) {
                *tag = Arc::clone(renamed);
            }
            for prop in props {
                match prop {
                    JsxProp::Attr { value, .. } => match value {
                        JsxAttrValue::Expr(e) => rewrite_expr_scope(e, active),
                        JsxAttrValue::String(_) | JsxAttrValue::ImplicitTrue => {}
                    },
                    JsxProp::Spread(e) => rewrite_expr_scope(e, active),
                }
            }
            for child in children {
                if let JsxChild::Expr(e) = child {
                    rewrite_expr_scope(e, active);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for child in children {
                if let JsxChild::Expr(e) = child {
                    rewrite_expr_scope(e, active);
                }
            }
        }
        Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => {}
    }
}

/// Merge all resolved modules into a single program. Dependencies are emitted first.
/// Import statements are rewritten as bindings from already-emitted dep exports.
/// Export statements are unwrapped (the inner declaration is emitted).
pub fn merge_modules(mut modules: Vec<ResolvedModule>) -> Result<MergedProgram, String> {
    // #469: give the ENTRY module a Go/Rust/Swift-style `main` entry point. The entry is the last
    // module (post-order dependency load order), and this runs BEFORE private-binding isolation so the
    // synthetic `main()` call is renamed in lockstep with the `main` declaration if isolation renames
    // it. Every backend consumes the merged program, so this keeps interp/vm/native/js consistent.
    if let Some(entry) = modules.last_mut() {
        tishlang_ast::append_main_entry(&mut entry.program.statements);
    }

    // #97: isolate module-private top-level bindings before they are flattened together.
    isolate_private_top_level_bindings(&mut modules);

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
                    // #305: re-export — map each re-exported name to the DEP's binding (the dep is
                    // earlier in load order, so its export table is already built). `export *` copies
                    // every dep export (without overriding an explicit one); `export { a as b }` maps
                    // b -> dep's binding for a. Downstream imports then resolve straight to the dep.
                    ExportDeclaration::ReExport {
                        specifiers,
                        all,
                        from: Some(from),
                        ..
                    } => {
                        let dir = module.path.parent().unwrap_or_else(|| Path::new("."));
                        let dep = resolve_import_path(from.as_ref(), dir, Path::new("."))
                            .ok()
                            .map(|p| p.canonicalize().unwrap_or(p))
                            .and_then(|p| path_to_idx.get(&p).copied())
                            .map(|dep_idx| module_exports[dep_idx].clone());
                        if let Some(dep) = dep {
                            if *all {
                                for (k, v) in &dep {
                                    module_exports[idx]
                                        .entry(k.clone())
                                        .or_insert_with(|| v.clone());
                                }
                            }
                            for spec in specifiers {
                                if let ImportSpecifier::Named { name, alias, .. } = spec {
                                    if let Some(binding) = dep.get(name.as_ref()) {
                                        let export_name =
                                            alias.as_deref().unwrap_or(name.as_ref()).to_string();
                                        module_exports[idx].insert(export_name, binding.clone());
                                    }
                                }
                            }
                        }
                    }
                    // #415 local named export (`export { a, b as c }`, no `from`): each specifier
                    // exports an already-declared LOCAL top-level binding, mapping export-name -> that
                    // binding. (`isolate_private_top_level_bindings` keeps exported names un-renamed.)
                    ExportDeclaration::ReExport {
                        specifiers,
                        from: None,
                        ..
                    } => {
                        for spec in specifiers {
                            if let ImportSpecifier::Named { name, alias, .. } = spec {
                                let export_name =
                                    alias.as_deref().unwrap_or(name.as_ref()).to_string();
                                module_exports[idx].insert(export_name, name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut statements = Vec::new();
    let mut statement_sources = Vec::new();
    for (idx, module) in modules.iter().enumerate() {
        let src_path = module.path.clone();
        let dir = module.path.parent().unwrap_or(Path::new("."));
        for stmt in &module.program.statements {
            match stmt {
                Statement::Import {
                    specifiers,
                    from,
                    span,
                } => {
                    if is_native_import(from.as_ref()) {
                        // Normalize fs/http/process -> tish:fs etc. for Node compatibility
                        let canonical_spec = normalize_builtin_spec(from.as_ref())
                            .unwrap_or_else(|| from.to_string());
                        // Emit VarDecl with NativeModuleLoad for each specifier
                        for spec in specifiers {
                            match spec {
                                ImportSpecifier::Named {
                                    name,
                                    name_span,
                                    alias,
                                    alias_span,
                                } => {
                                    let bind = alias.as_deref().unwrap_or(name.as_ref());
                                    let decl_name_span = alias_span.as_ref().unwrap_or(name_span);
                                    let init = Expr::NativeModuleLoad {
                                        spec: Arc::from(canonical_spec.clone()),
                                        export_name: name.clone(),
                                        span: *span,
                                    };
                                    merge_push(
                                        &mut statements,
                                        &mut statement_sources,
                                        Statement::VarDecl {
                                            name: Arc::from(bind),
                                            name_span: *decl_name_span,
                                            mutable: false,
                                            type_ann: None,
                                            init: Some(init),
                                            span: *span,
                                        },
                                        src_path.clone(),
                                    );
                                }
                                ImportSpecifier::Namespace { name, .. } => {
                                    return Err(format!(
                                        "Namespace import (* as {}) not supported for native module '{}'",
                                        name.as_ref(),
                                        from.as_ref()
                                    ));
                                }
                                ImportSpecifier::Default { name, .. } => {
                                    return Err(format!(
                                        "Default import '{}' not supported for native module '{}'. Use named import, e.g. import {{ egui }} from '{}'",
                                        name.as_ref(),
                                        from.as_ref(),
                                        from.as_ref()
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    let dep_path = resolve_import_path(from.as_ref(), dir, Path::new("."))?;
                    let dep_path = dep_path.canonicalize().unwrap_or(dep_path);
                    let dep_idx = *path_to_idx
                        .get(&dep_path)
                        .ok_or_else(|| format!("Resolved import '{}' not in module list", from))?;
                    let dep_exports = &module_exports[dep_idx];
                    for spec in specifiers {
                        match spec {
                            ImportSpecifier::Named {
                                name,
                                name_span,
                                alias,
                                alias_span,
                            } => {
                                let source = dep_exports
                                    .get(name.as_ref())
                                    .cloned()
                                    .unwrap_or_else(|| name.to_string());
                                let bind = alias.as_deref().unwrap_or(name.as_ref());
                                if bind != source {
                                    let decl_name_span = alias_span.as_ref().unwrap_or(name_span);
                                    merge_push(
                                        &mut statements,
                                        &mut statement_sources,
                                        Statement::VarDecl {
                                            name: Arc::from(bind),
                                            name_span: *decl_name_span,
                                            mutable: false,
                                            type_ann: None,
                                            init: Some(Expr::Ident {
                                                name: Arc::from(source),
                                                span: *span,
                                            }),
                                            span: *span,
                                        },
                                        src_path.clone(),
                                    );
                                }
                            }
                            ImportSpecifier::Namespace { name, name_span } => {
                                let mut props = Vec::new();
                                for (k, v) in dep_exports {
                                    props.push(tishlang_ast::ObjectProp::KeyValue(
                                        Arc::from(k.clone()),
                                        Expr::Ident {
                                            name: Arc::from(v.clone()),
                                            span: *span,
                                        },
                                        *name_span,
                                    ));
                                }
                                merge_push(
                                    &mut statements,
                                    &mut statement_sources,
                                    Statement::VarDecl {
                                        name: name.clone(),
                                        name_span: *name_span,
                                        mutable: false,
                                        type_ann: None,
                                        init: Some(Expr::Object { props, span: *span }),
                                        span: *span,
                                    },
                                    src_path.clone(),
                                );
                            }
                            ImportSpecifier::Default { name, name_span } => {
                                let source =
                                    dep_exports.get("default").cloned().ok_or_else(|| {
                                        format!("Module '{}' has no default export", from)
                                    })?;
                                merge_push(
                                    &mut statements,
                                    &mut statement_sources,
                                    Statement::VarDecl {
                                        name: name.clone(),
                                        name_span: *name_span,
                                        mutable: false,
                                        type_ann: None,
                                        init: Some(Expr::Ident {
                                            name: Arc::from(source),
                                            span: *span,
                                        }),
                                        span: *span,
                                    },
                                    src_path.clone(),
                                );
                            }
                        }
                    }
                }
                Statement::Export { declaration, .. } => match declaration.as_ref() {
                    ExportDeclaration::Named(s) => merge_push(
                        &mut statements,
                        &mut statement_sources,
                        *s.clone(),
                        src_path.clone(),
                    ),
                    ExportDeclaration::Default(e) => {
                        let default_name = format!("__default_{}", idx);
                        let espan = e.span();
                        merge_push(
                            &mut statements,
                            &mut statement_sources,
                            Statement::VarDecl {
                                name: Arc::from(default_name),
                                name_span: espan,
                                mutable: false,
                                type_ann: None,
                                init: Some((*e).clone()),
                                span: espan,
                            },
                            src_path.clone(),
                        );
                    }
                    // #305: re-export emits no code — `module_exports` already maps the re-exported
                    // names to the dep's bindings (in scope in the flattened bundle), so downstream
                    // imports resolve directly. Nothing to push here.
                    ExportDeclaration::ReExport { .. } => {}
                },
                _ => merge_push(
                    &mut statements,
                    &mut statement_sources,
                    stmt.clone(),
                    src_path.clone(),
                ),
            }
        }
    }
    // #295: capture the ENTRY module's exports (the entry is last — deps are emitted first). Merge
    // already built `module_exports[idx]` as exported_name -> local_binding; re-express as
    // (local, exported) for the JS bundle emitter, excluding `default` (emitted as `export default`).
    let entry_exports: Vec<(String, String)> = modules
        .len()
        .checked_sub(1)
        .map(|last| {
            let mut v: Vec<(String, String)> = module_exports[last]
                .iter()
                .filter(|(exported, _)| exported.as_str() != "default")
                .map(|(exported, local)| (local.clone(), exported.clone()))
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    Ok(MergedProgram {
        program: Program { statements },
        statement_sources,
        entry_exports,
    })
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
        let _ = merge_modules(modules).unwrap();
    }

    #[test]
    fn cargo_export_fn_name_sanitizes() {
        assert_eq!(
            cargo_export_fn_name("cargo:tish_serde_json"),
            "cargo_native_tish_serde_json_object"
        );
        assert_eq!(
            cargo_export_fn_name("cargo:my-crate"),
            "cargo_native_my_crate_object"
        );
    }
}
