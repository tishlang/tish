//! Module resolver: resolves relative imports, builds dependency graph, detects cycles.
//! Supports native imports: tish:egui, tish:polars, @scope/pkg (via package.json).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tish_ast::{ExportDeclaration, Expr, ImportSpecifier, Program, Statement};

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
}

/// Built-in modules that come from tish_runtime, not from package.json.
pub fn is_builtin_native_spec(spec: &str) -> bool {
    matches!(spec, "tish:fs" | "tish:http" | "tish:process")
}

/// Resolve all native imports in a merged program via package.json lookup.
/// Built-in modules (tish:fs, tish:http, tish:process) are skipped - they use tish_runtime directly.
pub fn resolve_native_modules(program: &Program, project_root: &Path) -> Result<Vec<ResolvedNativeModule>, String> {
    let root_canon = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize project root: {}", e))?;
    let mut seen = HashSet::new();
    let mut modules = Vec::new();
    for stmt in &program.statements {
        if let Statement::VarDecl {
            init: Some(Expr::NativeModuleLoad { spec, .. }),
            ..
        } = stmt
        {
            let s = spec.as_ref();
            if is_builtin_native_spec(s) {
                continue; // Built-ins use tish_runtime, no package.json lookup
            }
            if !seen.insert(s.to_string()) {
                continue;
            }
            let m = resolve_native_module(s, &root_canon)?;
            modules.push(m);
        }
    }
    Ok(modules)
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

/// Extract Cargo feature names from native imports in a merged program.
/// Used to enable tish_runtime features based on `import { x } from 'tish:egui'` etc.
pub fn extract_native_import_features(program: &Program) -> Vec<String> {
    let mut features = std::collections::HashSet::new();
    for stmt in &program.statements {
        if let Statement::VarDecl {
            init: Some(Expr::NativeModuleLoad { spec, .. }),
            ..
        } = stmt
        {
            if let Some(f) = native_spec_to_feature(spec.as_ref()) {
                features.insert(f);
            }
        }
    }
    features.into_iter().collect()
}

/// Returns true if the merged program contains native imports (tish:*, @scope/pkg).
pub fn has_native_imports(program: &Program) -> bool {
    for stmt in &program.statements {
        if let Statement::VarDecl {
            init: Some(Expr::NativeModuleLoad { .. }),
            ..
        } = stmt
        {
            return true;
        }
    }
    false
}

/// Returns true if the merged program contains external native imports (not built-in tish:fs/http/process).
/// Cranelift/LLVM reject these; bytecode VM supports built-ins only.
pub fn has_external_native_imports(program: &Program) -> bool {
    for stmt in &program.statements {
        if let Statement::VarDecl {
            init: Some(Expr::NativeModuleLoad { spec, .. }),
            ..
        } = stmt
        {
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
    let program = tish_parser::parse(&source)
        .map_err(|e| format!("Parse error in {}: {}", canonical.display(), e))?;

    // Collect imports and load dependencies first (skip native imports)
    let dir = canonical.parent().unwrap_or(Path::new("."));
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from) {
                continue; // Native imports don't load files
            }
            let dep_path = resolve_import_path(from, dir, project_root)?;
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
/// - tish:egui, tish:polars, etc.
/// - @scope/package (npm-style)
pub fn is_native_import(spec: &str) -> bool {
    spec.starts_with("tish:") || spec.starts_with('@')
}

/// Map native spec to Cargo feature name for built-in tish:* modules.
pub fn native_spec_to_feature(spec: &str) -> Option<String> {
    if spec.starts_with("tish:") {
        Some(spec.strip_prefix("tish:").unwrap_or(spec).to_string())
    } else {
        None
    }
}

/// Resolve an import specifier (e.g. "./foo.tish", "../lib/utils") to an absolute path.
fn resolve_import_path(
    spec: &str,
    from_dir: &Path,
    _project_root: &Path,
) -> Result<PathBuf, String> {
    if is_native_import(spec) {
        return Err(format!(
            "resolve_import_path called for native import (use merge_modules native branch): {}",
            spec
        ));
    }
    if !spec.starts_with("./") && !spec.starts_with("../") {
        return Err(format!(
            "Only relative imports (./, ../) or native imports (tish:*, @scope/pkg) are supported. Got: {}",
            spec
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
            if is_native_import(from) {
                continue;
            }
            let dep_path = resolve_import_path(from, from_dir, Path::new("."))?;
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
                    if is_native_import(from) {
                        // Emit VarDecl with NativeModuleLoad for each specifier
                        for spec in specifiers {
                            match spec {
                                ImportSpecifier::Named { name, alias } => {
                                    let bind = alias.as_deref().unwrap_or(name.as_ref());
                                    let init = Expr::NativeModuleLoad {
                                        spec: from.clone(),
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
                    let dep_path = resolve_import_path(from, dir, Path::new("."))?;
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
                                    props.push(tish_ast::ObjectProp::KeyValue(
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
