//! Discover `*.test.tish` / `*_test.tish` / `*.spec.tish` / `*_spec.tish` files.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Return true if `name` matches a recognized test file pattern.
pub fn is_test_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".tish") {
        return false;
    }
    lower.ends_with(".test.tish")
        || lower.ends_with("_test.tish")
        || lower.ends_with(".spec.tish")
        || lower.ends_with("_spec.tish")
}

/// Walk `roots` for test files. Optional `filters` keep paths whose display string
/// contains every substring (case-sensitive, Jest-style path filter).
pub fn discover_tests(roots: &[PathBuf], filters: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        if root.is_file() {
            if let Some(name) = root.file_name().and_then(|n| n.to_str()) {
                if is_test_file_name(name) || root.extension().map(|e| e == "tish") == Some(true) {
                    if path_matches(root, filters) {
                        out.push(root.clone());
                    }
                }
            }
            continue;
        }
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_test_file_name(name) {
                continue;
            }
            if path_matches(path, filters) {
                out.push(path.to_path_buf());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn path_matches(path: &Path, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let s = path.to_string_lossy();
    filters.iter().all(|f| s.contains(f.as_str()))
}
