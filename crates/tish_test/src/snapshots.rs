//! Snapshot matchers — `__snapshots__/<testfile>.<name>.snap` relative to the test file.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use tishlang_core::{set_pending_throw, Value};

use crate::assert::assertion_error;

thread_local! {
    static CURRENT_FILE: RefCell<String> = RefCell::new(String::new());
    static UPDATE: RefCell<bool> = RefCell::new(false);
    static COUNTERS: RefCell<std::collections::HashMap<String, usize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Set the active test file (used to resolve `__snapshots__/`).
pub fn set_current_file(path: &str) {
    CURRENT_FILE.with(|f| *f.borrow_mut() = path.to_string());
    COUNTERS.with(|c| c.borrow_mut().clear());
}

pub fn set_update_snapshots(update: bool) {
    UPDATE.with(|u| *u.borrow_mut() = update);
}

pub fn update_snapshots() -> bool {
    UPDATE.with(|u| *u.borrow())
        || std::env::var("TISH_UPDATE_SNAPSHOTS").ok().as_deref() == Some("1")
        || std::env::var("UPDATE_SNAPSHOTS").ok().as_deref() == Some("1")
}

fn snapshot_dir_for(test_file: &str) -> PathBuf {
    let p = Path::new(test_file);
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    parent.join("__snapshots__")
}

fn snap_path(test_file: &str, name: &str) -> PathBuf {
    let stem = Path::new(test_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test");
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    snapshot_dir_for(test_file).join(format!("{stem}.{safe}.snap"))
}

fn next_anon_name(test_file: &str) -> String {
    COUNTERS.with(|c| {
        let mut map = c.borrow_mut();
        let n = map.entry(test_file.to_string()).or_insert(0);
        *n += 1;
        format!("snapshot_{}", *n)
    })
}

fn serialize(v: &Value) -> String {
    tishlang_core::json_stringify(v)
}

/// Compare `received` against the on-disk snapshot (or write/update it).
pub fn match_snapshot(received: &Value, hint: Option<&str>) -> Value {
    let file = CURRENT_FILE.with(|f| f.borrow().clone());
    if file.is_empty() {
        set_pending_throw(assertion_error(
            "toMatchSnapshot: no current test file (runner must call set_current_file)",
            Some(received.clone()),
            None,
            "toMatchSnapshot",
            true,
        ));
        return Value::Null;
    }
    let name = hint
        .map(|s| s.to_string())
        .unwrap_or_else(|| next_anon_name(&file));
    let path = snap_path(&file, &name);
    let actual = serialize(received);
    let update = update_snapshots();

    if update || !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&path, format!("{}\n", actual)) {
            set_pending_throw(assertion_error(
                format!("toMatchSnapshot: failed to write {}: {}", path.display(), e),
                Some(received.clone()),
                None,
                "toMatchSnapshot",
                true,
            ));
            return Value::Null;
        }
        return Value::Null;
    }

    let expected = match fs::read_to_string(&path) {
        Ok(s) => s.trim_end_matches('\n').to_string(),
        Err(e) => {
            set_pending_throw(assertion_error(
                format!("toMatchSnapshot: failed to read {}: {}", path.display(), e),
                Some(received.clone()),
                None,
                "toMatchSnapshot",
                true,
            ));
            return Value::Null;
        }
    };

    if actual != expected {
        set_pending_throw(assertion_error(
            format!(
                "expect(received).toMatchSnapshot()\n\nSnapshot: {}\n\n- Expected\n+ Received\n\n- {}\n+ {}",
                path.display(),
                expected,
                actual
            ),
            Some(received.clone()),
            Some(Value::String(expected.as_str().into())),
            "toMatchSnapshot",
            true,
        ));
    }
    Value::Null
}
