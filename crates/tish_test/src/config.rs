//! Load `package.json` → `tish.test` config for the runner.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

#[derive(Clone, Debug, Default)]
pub struct TishTestConfig {
    /// Discovery roots (relative to package root).
    pub root: Vec<PathBuf>,
    /// Extra preload modules run before each test file (paths relative to package root).
    pub preload: Vec<PathBuf>,
    /// Default timeout ms.
    pub timeout_ms: Option<u64>,
    /// Project globs keyed by name (unit / integration / …).
    pub projects: Vec<ProjectConfig>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectConfig {
    pub name: String,
    pub root: PathBuf,
}

/// Read `package.json` beside `cwd` (or ancestors) and parse `tish.test`.
pub fn load_tish_test_config(start: &Path) -> TishTestConfig {
    let Some(pkg) = find_package_json(start) else {
        return TishTestConfig::default();
    };
    let Ok(text) = fs::read_to_string(&pkg) else {
        return TishTestConfig::default();
    };
    let Ok(json) = serde_json::from_str::<Json>(&text) else {
        return TishTestConfig::default();
    };
    let Some(test) = json.get("tish").and_then(|t| t.get("test")).or_else(|| json.get("tish.test")) else {
        // Also accept top-level "tish.test" object via nested tish.test already tried;
        // try `tishTest` camelCase as a convenience.
        if let Some(t) = json.get("tishTest") {
            return parse_config(t, pkg.parent().unwrap_or(start));
        }
        return TishTestConfig::default();
    };
    parse_config(test, pkg.parent().unwrap_or(start))
}

fn find_package_json(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        let candidate = cur.join("package.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn parse_config(test: &Json, base: &Path) -> TishTestConfig {
    let mut cfg = TishTestConfig::default();
    if let Some(root) = test.get("root") {
        if let Some(s) = root.as_str() {
            cfg.root.push(base.join(s));
        } else if let Some(arr) = root.as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    cfg.root.push(base.join(s));
                }
            }
        }
    }
    if let Some(arr) = test.get("preload").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                cfg.preload.push(base.join(s));
            }
        }
    }
    if let Some(n) = test.get("timeout").and_then(|v| v.as_u64()) {
        cfg.timeout_ms = Some(n);
    }
    if let Some(projects) = test.get("projects").and_then(|v| v.as_array()) {
        for p in projects {
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let root = p
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            cfg.projects.push(ProjectConfig {
                name,
                root: base.join(root),
            });
        }
    }
    cfg
}
