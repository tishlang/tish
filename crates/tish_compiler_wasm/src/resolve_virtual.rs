//! Virtual module resolution for browser/playground. No filesystem.
//! Resolves imports from an in-memory file map, merges modules into a single Program.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tishlang_ast::{ExportDeclaration, Expr, ImportSpecifier, Program, Statement};

/// A resolved module: virtual path and its parsed program.
#[derive(Debug, Clone)]
pub struct VirtualModule {
    pub path: String,
    pub program: Program,
}

/// Node-compatible aliases for built-in modules (fs -> tish:fs, etc.).
const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("fs", "tish:fs"),
    ("http", "tish:http"),
    ("process", "tish:process"),
    ("ws", "tish:ws"),
];

fn normalize_builtin_spec(spec: &str) -> Option<String> {
    if spec.starts_with("tish:") {
        return Some(spec.to_string());
    }
    BUILTIN_ALIASES
        .iter()
        .find(|(alias, _)| *alias == spec)
        .map(|(_, canonical)| (*canonical).to_string())
}

fn is_native_import(spec: &str) -> bool {
    spec.starts_with("tish:")
        || spec.starts_with("cargo:")
        || spec.starts_with('@')
        || matches!(spec, "fs" | "http" | "process" | "ws")
}

/// Normalize a virtual path: resolve . and .. components.
/// e.g. "sub/../lib.tish" -> "lib.tish", "./foo.tish" -> "foo.tish"
fn normalize_virtual_path(from_dir: &str, spec: &str) -> Result<String, String> {
    if !spec.starts_with("./") && !spec.starts_with("../") {
        return Err(format!(
            "Only relative imports (./, ../) or native imports (tish:*, @scope/pkg) are supported. Got: {}",
            spec
        ));
    }
    let combined = if from_dir.is_empty() {
        spec.to_string()
    } else {
        format!("{}/{}", from_dir, spec)
    };
    let parts: Vec<&str> = combined.split('/').collect();
    let mut stack: Vec<&str> = Vec::new();
    for p in parts {
        match p {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            _ => stack.push(p),
        }
    }
    Ok(stack.join("/"))
}

/// Parent directory of a virtual path. "main.tish" -> "", "sub/foo.tish" -> "sub"
fn parent_dir(path: &str) -> &str {
    if let Some(slash) = path.rfind('/') {
        &path[..slash]
    } else {
        ""
    }
}

/// Resolve import spec to a key for the files map. Tries .tish extension if missing.
fn resolve_import_to_key(
    spec: &str,
    from_dir: &str,
    files: &HashMap<String, String>,
) -> Result<String, String> {
    let normalized = normalize_virtual_path(from_dir, spec)?;
    if files.contains_key(&normalized) {
        return Ok(normalized);
    }
    if !normalized.ends_with(".tish") && !normalized.contains('.') {
        let with_ext = format!("{}.tish", normalized);
        if files.contains_key(&with_ext) {
            return Ok(with_ext);
        }
    }
    Err(format!(
        "Cannot resolve import '{}' from {}: file not in virtual file map",
        spec, from_dir
    ))
}

/// Resolve all modules starting from the entry file. Returns modules in dependency order.
pub fn resolve_virtual(
    entry_path: &str,
    files: &HashMap<String, String>,
) -> Result<Vec<VirtualModule>, String> {
    let entry_key = if files.contains_key(entry_path) {
        entry_path.to_string()
    } else if !entry_path.ends_with(".tish") {
        let with_ext = format!("{}.tish", entry_path);
        if files.contains_key(&with_ext) {
            with_ext
        } else {
            return Err(format!("Entry file '{}' not in virtual file map", entry_path));
        }
    } else {
        return Err(format!("Entry file '{}' not in virtual file map", entry_path));
    };

    let mut visited = HashSet::new();
    let mut path_to_module: HashMap<String, Program> = HashMap::new();
    let mut load_order: Vec<String> = Vec::new();

    load_module_recursive(
        &entry_key,
        files,
        &mut visited,
        &mut path_to_module,
        &mut load_order,
    )?;

    Ok(load_order
        .into_iter()
        .map(|p| {
            let program = path_to_module.remove(&p).unwrap();
            VirtualModule { path: p, program }
        })
        .collect())
}

fn load_module_recursive(
    module_path: &str,
    files: &HashMap<String, String>,
    visited: &mut HashSet<String>,
    path_to_module: &mut HashMap<String, Program>,
    load_order: &mut Vec<String>,
) -> Result<(), String> {
    if visited.contains(module_path) {
        return Ok(());
    }
    visited.insert(module_path.to_string());

    let source = files.get(module_path).ok_or_else(|| {
        format!("Module '{}' not in virtual file map", module_path)
    })?;
    let program = tishlang_parser::parse(source.trim())
        .map_err(|e| format!("Parse error in {}: {}", module_path, e))?;

    let from_dir = parent_dir(module_path);
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from) {
                continue;
            }
            let dep_key = resolve_import_to_key(from, from_dir, files)?;
            if !path_to_module.contains_key(&dep_key) {
                load_module_recursive(
                    &dep_key,
                    files,
                    visited,
                    path_to_module,
                    load_order,
                )?;
            }
        }
    }

    path_to_module.insert(module_path.to_string(), program);
    load_order.push(module_path.to_string());
    Ok(())
}

/// Check for cyclic imports.
pub fn detect_cycles_virtual(modules: &[VirtualModule]) -> Result<(), String> {
    let path_to_idx: HashMap<_, _> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.path.clone(), i))
        .collect();

    for (idx, module) in modules.iter().enumerate() {
        let dir = parent_dir(&module.path);
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
                .map(|&i| modules[i].path.clone())
                .collect();
            return Err(format!("Circular import detected: {}", path_names.join(" -> ")));
        }
    }
    Ok(())
}

fn has_cycle_from(
    from_dir: &str,
    program: &Program,
    path_to_idx: &HashMap<String, usize>,
    modules: &[VirtualModule],
    stack: &mut Vec<usize>,
    visiting: &mut HashSet<usize>,
) -> Result<bool, String> {
    for stmt in &program.statements {
        if let Statement::Import { from, .. } = stmt {
            if is_native_import(from) {
                continue;
            }
            let dep_key = resolve_import_to_key_for_cycle(from, from_dir, path_to_idx)?;
            if let Some(&dep_idx) = path_to_idx.get(&dep_key) {
                if stack.contains(&dep_idx) {
                    stack.push(dep_idx);
                    return Ok(true);
                }
                if !visiting.contains(&dep_idx) {
                    visiting.insert(dep_idx);
                    stack.push(dep_idx);
                    let dep = &modules[dep_idx];
                    let dep_dir = parent_dir(&dep.path);
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

fn resolve_import_to_key_for_cycle(
    spec: &str,
    from_dir: &str,
    path_to_idx: &HashMap<String, usize>,
) -> Result<String, String> {
    let normalized = normalize_virtual_path(from_dir, spec)?;
    if path_to_idx.contains_key(&normalized) {
        return Ok(normalized);
    }
    if !normalized.ends_with(".tish") && !normalized.contains('.') {
        let with_ext = format!("{}.tish", normalized);
        if path_to_idx.contains_key(&with_ext) {
            return Ok(with_ext);
        }
    }
    Err(format!(
        "Cannot resolve import '{}' from {}: module not in resolved set",
        spec, from_dir
    ))
}

/// Merge all resolved modules into a single program. Dependencies are emitted first.
pub fn merge_modules_virtual(modules: Vec<VirtualModule>) -> Result<Program, String> {
    let path_to_idx: HashMap<String, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.path.clone(), i))
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
        let dir = parent_dir(&module.path);
        for stmt in &module.program.statements {
            match stmt {
                Statement::Import { specifiers, from, span } => {
                    if is_native_import(from) {
                        let canonical_spec =
                            normalize_builtin_spec(from).unwrap_or_else(|| from.to_string());
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
                                        "Default import '{}' not supported for native module '{}'. Use named import.",
                                        bind.as_ref(),
                                        from.as_ref()
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    let dep_key = resolve_import_to_key_for_cycle(from, dir, &path_to_idx)?;
                    let dep_idx = *path_to_idx
                        .get(&dep_key)
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
mod tests {
    use super::*;

    #[test]
    fn test_resolve_virtual_simple_import() {
        let mut files = HashMap::new();
        files.insert(
            "lib.tish".to_string(),
            "export fn add(a, b) { return a + b }".to_string(),
        );
        files.insert(
            "main.tish".to_string(),
            "import { add } from \"./lib.tish\"\nconsole.log(add(1, 2))".to_string(),
        );
        let modules = resolve_virtual("main.tish", &files).unwrap();
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].path, "lib.tish");
        assert_eq!(modules[1].path, "main.tish");
        detect_cycles_virtual(&modules).unwrap();
        let program = merge_modules_virtual(modules).unwrap();
        assert!(!program.statements.is_empty());
    }
}
