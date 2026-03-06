//! Module resolver: resolves relative imports, builds dependency graph, detects cycles.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tish_ast::{ExportDeclaration, Expr, ImportSpecifier, Program, Statement};

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

    // Collect imports and load dependencies first
    let dir = canonical.parent().unwrap_or(Path::new("."));
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
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

/// Resolve an import specifier (e.g. "./foo.tish", "../lib/utils") to an absolute path.
fn resolve_import_path(
    spec: &str,
    from_dir: &Path,
    _project_root: &Path,
) -> Result<PathBuf, String> {
    if !spec.starts_with("./") && !spec.starts_with("../") {
        return Err(format!(
            "Only relative imports are supported (./ or ../). Got: {}",
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
            idx,
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
    from_idx: usize,
    from_dir: &Path,
    program: &Program,
    path_to_idx: &HashMap<PathBuf, usize>,
    modules: &[ResolvedModule],
    stack: &mut Vec<usize>,
    visiting: &mut HashSet<usize>,
) -> Result<bool, String> {
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
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
                        dep_idx,
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
                    let dep_path = resolve_import_path(from, dir, Path::new("."))?;
                    let dep_path = dep_path
                        .canonicalize()
                        .unwrap_or_else(|_| dep_path);
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
                        ExportDeclaration::Named(s) => statements.push((*s.clone())),
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
