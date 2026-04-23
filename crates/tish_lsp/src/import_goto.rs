//! Go to definition for import specifiers: relative paths, bare `node_modules` packages, and
//! native `tish:` / `@scope/pkg` / `cargo:` → Rust `pub fn` sites (via `syn` + optional `cargo metadata`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use regex::Regex;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

use tishlang_ast::{ImportSpecifier, Program, Statement};
use tishlang_resolve::member_access_chain_at_cursor;
use tishlang_compile::{
    export_name_to_rust_ident, is_builtin_native_spec, is_cargo_native_spec, is_native_import,
    normalize_builtin_spec, read_project_tish_config, resolve_bare_spec, resolve_native_modules,
};

/// Pick a workspace / project root for resolving `package.json` and `node_modules`.
fn infer_project_root(file_path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    if let Ok(can) = file_path.canonicalize() {
        for r in roots {
            if let Ok(rc) = r.canonicalize() {
                if can.starts_with(&rc) {
                    return Some(rc);
                }
            }
        }
    }
    let mut dir = file_path.parent()?.to_path_buf();
    loop {
        if dir.join("package.json").exists() || dir.join("Cargo.toml").exists() {
            return dir.canonicalize().ok().or(Some(dir));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

fn location_from_rust_path(path: &Path, line: u32, col: u32) -> Option<Location> {
    let uri = Url::from_file_path(path).ok()?;
    let line_str = std::fs::read_to_string(path).ok()?;
    let line_slice = line_str.lines().nth(line as usize).unwrap_or("");
    let end_char = line_slice.len() as u32;
    Some(Location {
        uri,
        range: Range {
            start: Position { line, character: col },
            end: Position {
                line,
                character: end_char.max(col.saturating_add(1)),
            },
        },
    })
}

fn resolve_cargo_dep_crate_root(
    project_root: &Path,
    dep_key: &str,
    raw: &serde_json::Value,
) -> Result<PathBuf, String> {
    match raw {
        serde_json::Value::String(ver) => {
            let r = tishlang_cargo_bindgen::resolve_registry_dependency(dep_key, ver)?;
            Ok(r.source_root())
        }
        serde_json::Value::Object(map) => {
            let path_str = map
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    "cargo:… rustDependencies entry must be a version string or an object with \"path\""
                        .to_string()
                })?;
            let glue = Path::new(path_str);
            let glue = if glue.is_absolute() {
                glue.to_path_buf()
            } else {
                project_root.join(path_str)
            };
            let glue = glue.canonicalize().unwrap_or(glue);
            let glue_cargo = glue.join("Cargo.toml");
            if glue_cargo.is_file() {
                match tishlang_cargo_bindgen::resolve_dependency_from_manifest(&glue_cargo, dep_key) {
                    Ok(r) => Ok(r.source_root()),
                    Err(_) if glue.join("src").is_dir() => Ok(glue),
                    Err(e) => Err(e),
                }
            } else if glue.join("src").is_dir() {
                Ok(glue)
            } else {
                Err(format!(
                    "path dependency for {} does not look like a Rust crate: {}",
                    dep_key,
                    glue.display()
                ))
            }
        }
        _ => Err(format!(
            "tish.rustDependencies.{} must be a string (semver) or object with path",
            dep_key
        )),
    }
}

fn cargo_crate_root_cached(
    project_root: &Path,
    spec: &str,
    dep_key: &str,
    raw: &serde_json::Value,
    cache: &RwLock<HashMap<(PathBuf, String), PathBuf>>,
) -> Result<PathBuf, String> {
    let key = (project_root.to_path_buf(), spec.to_string());
    if let Ok(g) = cache.read() {
        if let Some(p) = g.get(&key) {
            return Ok(p.clone());
        }
    }
    let root = resolve_cargo_dep_crate_root(project_root, dep_key, raw)?;
    if let Ok(mut g) = cache.write() {
        g.insert(key, root.clone());
    }
    Ok(root)
}

fn rust_def_for_crate_root(crate_root: &Path, tish_export: &str) -> Option<Location> {
    let snake = export_name_to_rust_ident(tish_export);
    let try_names: [&str; 2] = [&snake, tish_export];
    for name in try_names {
        if name.is_empty() {
            continue;
        }
        if let Ok((path, line, col)) = tishlang_cargo_bindgen::rust_public_fn_location(crate_root, name)
        {
            return location_from_rust_path(&path, line, col);
        }
    }
    None
}

fn rust_fn_location_exact(crate_root: &Path, rust_fn_ident: &str) -> Option<Location> {
    if rust_fn_ident.is_empty() {
        return None;
    }
    let (path, line, col) =
        tishlang_cargo_bindgen::rust_public_fn_location(crate_root, rust_fn_ident).ok()?;
    location_from_rust_path(&path, line, col)
}

fn is_ident_char_member(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// When the cursor is on `member` in `receiver.member` on a single line, returns `(receiver, member)`.
fn split_receiver_member(line: &str, col: usize) -> Option<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = col.min(chars.len().saturating_sub(1));
    if !is_ident_char_member(chars[col]) {
        return None;
    }
    let mut end = col;
    while end + 1 < chars.len() && is_ident_char_member(chars[end + 1]) {
        end += 1;
    }
    let mut start = col;
    while start > 0 && is_ident_char_member(chars[start - 1]) {
        start -= 1;
    }
    if start == 0 {
        return None;
    }
    if chars[start - 1] != '.' {
        return None;
    }
    let member: String = chars[start..=end].iter().collect();
    let mut k = start - 2;
    while k > 0 && is_ident_char_member(chars[k - 1]) {
        k -= 1;
    }
    let receiver: String = chars[k..start - 1].iter().collect();
    if receiver.is_empty() {
        return None;
    }
    Some((receiver, member))
}

fn try_rust_member_on_crate(
    crate_root: &Path,
    members: &[Arc<str>],
    imported_receiver: Option<&str>,
) -> Option<Location> {
    let last = members.last()?.as_ref();
    if let Some(loc) = rust_def_for_crate_root(crate_root, last) {
        return Some(loc);
    }
    if members.len() >= 2 {
        let joined = members
            .iter()
            .map(|m| export_name_to_rust_ident(m.as_ref()))
            .collect::<Vec<_>>()
            .join("_");
        if let Some(loc) = rust_fn_location_exact(crate_root, &joined) {
            return Some(loc);
        }
    }
    if let Some(im) = imported_receiver {
        let mut s = export_name_to_rust_ident(im);
        for m in members {
            s.push('_');
            s.push_str(&export_name_to_rust_ident(m.as_ref()));
        }
        if let Some(loc) = rust_fn_location_exact(crate_root, &s) {
            return Some(loc);
        }
    }
    None
}

/// Optional `| …` hover line after the 1-based line in a native package’s `lsp-pragmas.d.tish`.
fn parse_lsp_pragmas_native(src: &str) -> HashMap<String, (crate::builtin_goto::BuiltinDef, Option<String>)> {
    let re = Regex::new(
        r"(?m)^\s*//\s*@tish-source\s+(\S+)\s+(\S+)\s+(\d+)(?:\s*\|\s*(.*?))?\s*$",
    )
    .expect("native lsp pragma regex");
    let mut m = HashMap::new();
    for cap in re.captures_iter(src) {
        let sym = cap[1].to_string();
        let rel = cap[2].to_string();
        let line_1: u32 = cap[3].parse().unwrap_or(1);
        let doc = cap
            .get(4)
            .map(|g| g.as_str().trim().to_string())
            .filter(|s| !s.is_empty());
        m.insert(
            sym,
            (
                crate::builtin_goto::BuiltinDef {
                    rel_path: rel,
                    line: line_1.saturating_sub(1),
                    character: 0,
                },
                doc,
            ),
        );
    }
    m
}

fn pragma_key_for_native_member(
    imported_for_prefix: Option<&str>,
    members: &[Arc<str>],
) -> Option<String> {
    if members.is_empty() {
        return None;
    }
    match imported_for_prefix {
        Some(prefix) => {
            let tail: String = members.iter().map(|m| m.as_ref()).collect::<Vec<_>>().join(".");
            if tail.is_empty() {
                None
            } else {
                Some(format!("{prefix}.{tail}"))
            }
        }
        None => {
            if members.len() >= 2 {
                Some(
                    members
                        .iter()
                        .map(|m| m.as_ref())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        }
    }
}

fn lookup_lsp_pragma_in_crate_root(
    crate_root: &Path,
    key: &str,
) -> Option<(crate::builtin_goto::BuiltinDef, Option<String>)> {
    let path = crate_root.join("lsp-pragmas.d.tish");
    let src = std::fs::read_to_string(&path).ok()?;
    let map = parse_lsp_pragmas_native(&src);
    map.get(key).cloned()
}

#[derive(Debug, Clone)]
pub struct NativeMemberDefinition {
    pub location: Location,
    /// From `lsp-pragmas.d.tish` when Rust `pub fn` resolution misses.
    pub doc: Option<String>,
}

/// Static member chain `root.a.b` where `root` is an import: resolve the leaf to a Rust `pub fn`,
/// else to `lsp-pragmas.d.tish` in the native package (e.g. `tish-macos`).
pub fn native_member_definition(
    program: &Program,
    file_path: &Path,
    text: &str,
    roots: &[PathBuf],
    cargo_src_cache: &RwLock<HashMap<(PathBuf, String), PathBuf>>,
    lsp_line: u32,
    lsp_character: u32,
    word: &str,
) -> Option<NativeMemberDefinition> {
    let project_root = infer_project_root(file_path, roots)?;
    let from_dir = file_path.parent()?;

    let (receiver_local, members): (String, Vec<Arc<str>>) =
        if let Some(ch) = member_access_chain_at_cursor(program, text, lsp_line, lsp_character) {
            if ch.members.last()?.as_ref() != word {
                return None;
            }
            (ch.root_local.as_ref().to_string(), ch.members)
        } else {
            let line_str = text.lines().nth(lsp_line as usize)?;
            let col = lsp_character as usize;
            let (recv, member) = split_receiver_member(line_str, col)?;
            if member != word {
                return None;
            }
            (recv, vec![Arc::from(member.as_str())])
        };

    for stmt in &program.statements {
        let Statement::Import {
            specifiers, from, ..
        } = stmt
        else {
            continue;
        };
        let from_s = from.as_ref();
        for sp in specifiers {
            let (imported_for_prefix, local) = match sp {
                ImportSpecifier::Named { name, alias, .. } => (
                    Some(name.as_ref()),
                    alias.as_ref().map(|a| a.as_ref()).unwrap_or(name.as_ref()),
                ),
                ImportSpecifier::Default { name, .. } => (Some(name.as_ref()), name.as_ref()),
                ImportSpecifier::Namespace { name, .. } => (None, name.as_ref()),
            };
            if local != receiver_local.as_str() {
                continue;
            }

            if from_s.starts_with("./") || from_s.starts_with("../") {
                if members.len() == 1 {
                    let loc = resolve_relative_tish(from_dir, from_s, members[0].as_ref())?;
                    return Some(NativeMemberDefinition {
                        location: loc,
                        doc: None,
                    });
                }
                continue;
            }

            if !is_native_import(from_s) {
                continue;
            }

            let spec = normalize_builtin_spec(from_s).unwrap_or_else(|| from_s.to_string());
            if is_builtin_native_spec(&spec) {
                continue;
            }

            let crate_root = if is_cargo_native_spec(&spec) {
                let dep_key = spec.strip_prefix("cargo:")?;
                let tish = read_project_tish_config(&project_root);
                let raw = tish
                    .get("rustDependencies")
                    .and_then(|v| v.get(dep_key))
                    .cloned()?;
                cargo_crate_root_cached(&project_root, &spec, dep_key, &raw, cargo_src_cache).ok()?
            } else {
                let mods = resolve_native_modules(program, &project_root).ok()?;
                let m = mods.iter().find(|mm| mm.spec == spec)?;
                m.crate_path.clone()
            };

            if let Some(loc) = try_rust_member_on_crate(&crate_root, &members, imported_for_prefix) {
                return Some(NativeMemberDefinition {
                    location: loc,
                    doc: None,
                });
            }
            if let Some(key) = pragma_key_for_native_member(imported_for_prefix, &members) {
                if let Some((def, doc)) = lookup_lsp_pragma_in_crate_root(&crate_root, &key) {
                    if let Some(loc) = crate::builtin_goto::to_file_location(&crate_root, &def) {
                        return Some(NativeMemberDefinition { location: loc, doc });
                    }
                }
            }
            return None;
        }
    }
    None
}

/// Static member chain `root.a.b` where `root` is an import: resolve the leaf name to a Rust `pub fn`
/// (native / `cargo:`) or a single-level export in a relative `.tish` module.
pub fn definition_for_native_receiver_member(
    program: &Program,
    file_path: &Path,
    text: &str,
    roots: &[PathBuf],
    cargo_src_cache: &RwLock<HashMap<(PathBuf, String), PathBuf>>,
    lsp_line: u32,
    lsp_character: u32,
    word: &str,
) -> Option<Location> {
    native_member_definition(
        program,
        file_path,
        text,
        roots,
        cargo_src_cache,
        lsp_line,
        lsp_character,
        word,
    )
    .map(|d| d.location)
}

fn resolve_relative_tish(from_dir: &Path, from_s: &str, imported: &str) -> Option<Location> {
    let target = from_dir.join(from_s.trim_start_matches("./"));
    let target = if target.extension().is_none() {
        target.with_extension("tish")
    } else {
        target
    };
    let can = target.canonicalize().ok()?;
    let u = Url::from_file_path(&can).ok()?;
    let src = std::fs::read_to_string(&can).ok()?;
    let prog = tishlang_parser::parse(&src).ok()?;
    crate::find_export(&prog, imported, &u, &src)
}

/// After same-file [`tishlang_resolve::definition_span`] misses, resolve import sites (Tish files,
/// `node_modules` packages, and Rust `pub fn` for native / `cargo:` imports).
pub fn definition_for_import(
    program: &Program,
    file_path: &Path,
    word: &str,
    roots: &[PathBuf],
    cargo_src_cache: &RwLock<HashMap<(PathBuf, String), PathBuf>>,
) -> Option<Location> {
    if word.is_empty() {
        return None;
    }
    let project_root = infer_project_root(file_path, roots)?;
    let from_dir = file_path.parent()?;

    for stmt in &program.statements {
        let Statement::Import {
            specifiers, from, ..
        } = stmt
        else {
            continue;
        };
        let from_s = from.as_ref();
        for sp in specifiers {
            let (imported, local) = match sp {
                ImportSpecifier::Named { name, alias, .. } => (
                    name.as_ref(),
                    alias.as_ref().map(|a| a.as_ref()).unwrap_or(name.as_ref()),
                ),
                ImportSpecifier::Default { name, .. } => (name.as_ref(), name.as_ref()),
                ImportSpecifier::Namespace { .. } => continue,
            };
            if local != word {
                continue;
            }

            if from_s.starts_with("./") || from_s.starts_with("../") {
                return resolve_relative_tish(from_dir, from_s, imported);
            }

            if is_native_import(from_s) {
                let spec = normalize_builtin_spec(from_s).unwrap_or_else(|| from_s.to_string());
                if is_builtin_native_spec(&spec) {
                    return None;
                }
                if is_cargo_native_spec(&spec) {
                    let dep_key = spec.strip_prefix("cargo:")?;
                    let tish = read_project_tish_config(&project_root);
                    let raw = tish
                        .get("rustDependencies")
                        .and_then(|v| v.get(dep_key))
                        .cloned()?;
                    let crate_root =
                        cargo_crate_root_cached(&project_root, &spec, dep_key, &raw, cargo_src_cache)
                            .ok()?;
                    return rust_def_for_crate_root(&crate_root, imported);
                }

                let mods = resolve_native_modules(program, &project_root).ok()?;
                let m = mods.iter().find(|mm| mm.spec == spec)?;
                return rust_def_for_crate_root(&m.crate_path, imported);
            }

            let entry = resolve_bare_spec(from_s, from_dir, &project_root)?;
            let u = Url::from_file_path(&entry).ok()?;
            let src = std::fs::read_to_string(&entry).ok()?;
            let prog = tishlang_parser::parse(&src).ok()?;
            return crate::find_export(&prog, imported, &u, &src);
        }
    }
    None
}

#[cfg(test)]
mod receiver_member_tests {
    use std::sync::Arc;

    #[test]
    fn splits_window_set_title() {
        let line = "window.setTitle(\"Hi\")";
        let col = "window.".len(); // on 's'
        let (recv, mem) = super::split_receiver_member(line, col).expect("split");
        assert_eq!(recv, "window");
        assert_eq!(mem, "setTitle");
    }

    #[test]
    fn native_pragma_parse_optional_doc() {
        let src = r"// @tish-source window.innerHeight src/appkit/window_api.rs 289 | Height in points.
// @tish-source window.innerWidth src/appkit/window_api.rs 272
";
        let m = super::parse_lsp_pragmas_native(src);
        let (def, doc) = m.get("window.innerHeight").expect("innerHeight");
        assert_eq!(def.rel_path, "src/appkit/window_api.rs");
        assert_eq!(def.line, 288);
        assert_eq!(doc.as_deref(), Some("Height in points."));
        let (def2, doc2) = m.get("window.innerWidth").expect("innerWidth");
        assert_eq!(def2.line, 271);
        assert!(doc2.is_none());
    }

    #[test]
    fn pragma_key_named_import() {
        let k = super::pragma_key_for_native_member(Some("window"), &[Arc::from("innerHeight")]);
        assert_eq!(k.as_deref(), Some("window.innerHeight"));
    }
}
