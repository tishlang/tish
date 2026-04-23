//! Walk dependency `src/**/*.rs` and collect `pub fn` items by name.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{Item, ItemFn, Visibility};
use walkdir::WalkDir;

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Map export name (Rust ident) to the function AST (must be unique).
pub fn discover_public_functions(crate_root: &Path) -> Result<HashMap<String, ItemFn>, String> {
    let src = crate_root.join("src");
    if !src.is_dir() {
        return Err(format!("no src/ under {}", crate_root.display()));
    }

    let mut map: HashMap<String, (PathBuf, ItemFn)> = HashMap::new();

    for entry in WalkDir::new(&src)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false))
    {
        let path = entry.path();
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let file = syn::parse_file(&text).map_err(|e| format!("parse {}: {}", path.display(), e))?;

        for item in file.items {
            if let Item::Fn(f) = item {
                if !is_pub(&f.vis) {
                    continue;
                }
                let name = f.sig.ident.to_string();
                if let Some((prev_path, _)) = map.get(&name) {
                    return Err(format!(
                        "ambiguous public fn `{}`: found in {} and {}",
                        name,
                        prev_path.display(),
                        path.display()
                    ));
                }
                map.insert(name, (path.to_path_buf(), f));
            }
        }
    }

    Ok(map.into_iter().map(|(k, (_, v))| (k, v)).collect())
}

/// On-disk location of a top-level `pub fn {fn_name}` under `crate_root/src` (LSP line/column, 0-based).
///
/// Requires `proc-macro2` built with `span-locations` (this crate enables it) so spans from
/// `syn::parse_file` carry line/column.
pub fn rust_public_fn_location(
    crate_root: &Path,
    fn_name: &str,
) -> Result<(PathBuf, u32, u32), String> {
    let src = crate_root.join("src");
    if !src.is_dir() {
        return Err(format!("no src/ under {}", crate_root.display()));
    }

    for entry in WalkDir::new(&src)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false))
    {
        let path = entry.path();
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let file = syn::parse_file(&text).map_err(|e| format!("parse {}: {}", path.display(), e))?;

        for item in file.items {
            if let Item::Fn(f) = item {
                if !is_pub(&f.vis) {
                    continue;
                }
                if f.sig.ident != fn_name {
                    continue;
                }
                let lc = f.sig.ident.span().start();
                let line = u32::try_from(lc.line)
                    .map_err(|_| "span line out of range".to_string())?
                    .saturating_sub(1);
                let col = u32::try_from(lc.column).map_err(|_| "span column out of range".to_string())?;
                return Ok((path.to_path_buf(), line, col));
            }
        }
    }

    Err(format!(
        "no public fn `{}` found under {}/src",
        fn_name,
        crate_root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rust_public_fn_location_finds_fn() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("lib.rs"),
            "// comment\npub fn hello_tish_export() -> i32 { 0 }\n",
        )
        .unwrap();
        let (path, line, _col) = rust_public_fn_location(tmp.path(), "hello_tish_export").unwrap();
        assert!(path.ends_with("lib.rs"));
        assert_eq!(line, 1);
    }
}
