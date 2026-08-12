//! Snapshot matchers — `__snapshots__/<testfile>.<name>.snap` relative to the test file.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use tishlang_core::{set_pending_throw, Value};

use crate::assert::assertion_error;

thread_local! {
    static CURRENT_FILE: RefCell<String> = const { RefCell::new(String::new()) };
    /// Full name of the running test. Snapshots are keyed by it so two unnamed
    /// `toMatchSnapshot()` calls in different tests cannot collide on one file.
    static CURRENT_TEST: RefCell<String> = const { RefCell::new(String::new()) };
    static UPDATE: RefCell<bool> = const { RefCell::new(false) };
    static CI: RefCell<bool> = const { RefCell::new(false) };
    static COUNTERS: RefCell<std::collections::HashMap<String, usize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Set the active test file (used to resolve `__snapshots__/`).
pub fn set_current_file(path: &str) {
    CURRENT_FILE.with(|f| *f.borrow_mut() = path.to_string());
    COUNTERS.with(|c| c.borrow_mut().clear());
}

/// Set the running test's full name, and reset its per-test snapshot counter.
pub fn set_current_test(full_name: &str) {
    CURRENT_TEST.with(|t| *t.borrow_mut() = full_name.to_string());
    COUNTERS.with(|c| {
        c.borrow_mut().remove(full_name);
    });
}

pub fn set_update_snapshots(update: bool) {
    UPDATE.with(|u| *u.borrow_mut() = update);
}

/// In CI a *missing* snapshot is a failure, not something to write and pass. Otherwise a
/// deleted `.snap` file makes the suite go green on the very run that should catch it.
pub fn set_ci_mode(ci: bool) {
    CI.with(|c| *c.borrow_mut() = ci);
}

fn ci_mode() -> bool {
    CI.with(|c| *c.borrow())
        || std::env::var("CI")
            .ok()
            .is_some_and(|v| v != "0" && v != "false")
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

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Keep file names bounded; a long describe > it chain would otherwise blow past NAME_MAX.
    if s.len() > 120 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        format!("{}_{hash:x}", &s[..120])
    } else {
        s
    }
}

fn snap_path(test_file: &str, name: &str) -> PathBuf {
    let stem = Path::new(test_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test");
    snapshot_dir_for(test_file).join(format!("{stem}.{}.snap", sanitize(name)))
}

/// Default snapshot key: `<test full name> <n>`, mirroring Jest. Keyed by test rather than by
/// file so unnamed snapshots in sibling tests get distinct files.
fn next_anon_name() -> String {
    let test = CURRENT_TEST.with(|t| t.borrow().clone());
    let key = if test.is_empty() {
        CURRENT_FILE.with(|f| f.borrow().clone())
    } else {
        test
    };
    COUNTERS.with(|c| {
        let mut map = c.borrow_mut();
        let n = map.entry(key.clone()).or_insert(0);
        *n += 1;
        format!("{key} {}", *n)
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
    let name = hint.map(|s| s.to_string()).unwrap_or_else(next_anon_name);
    let path = snap_path(&file, &name);
    let actual = serialize(received);
    let update = update_snapshots();
    let exists = path.exists();

    if !exists && ci_mode() && !update {
        set_pending_throw(assertion_error(
            format!(
                "expect(received).toMatchSnapshot()\n\nSnapshot missing (CI mode): {}",
                path.display()
            ),
            Some(received.clone()),
            None,
            "toMatchSnapshot",
            true,
        ));
        return Value::Null;
    }

    if update || !exists {
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
