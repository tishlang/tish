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
