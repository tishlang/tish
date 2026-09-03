//! Node-compatible `fs` surface for `tish:fs` and the async `tish:fs/promises` module
//! (issue #122). Each operation has a single `*_core` returning `Result<Value, Value>`
//! (Ok = result, Err = an error value); a macro derives the synchronous export (returns the
//! value, or the error object on failure — tish's sync convention) and the promise export
//! (a fulfilled/rejected Promise — Node's `fs/promises` convention).
//!
//! Node names are primary (`readFileSync`, `statSync`, …); the existing tish names
//! (`readFile`, `readDir`, `fileExists`, `isDir`) are kept as aliases in the backends.
#![cfg(feature = "fs")]

use crate::promise::{promise_reject, promise_resolve};
use std::time::{SystemTime, UNIX_EPOCH};
use tishlang_builtins::helpers::make_error_value;
use tishlang_core::{ObjectMap, Value, VmRef};

fn path_arg(args: &[Value], i: usize) -> String {
    args.get(i).map(|v| v.to_display_string()).unwrap_or_default()
}

/// Map an io::Error to the same error value `read_file` already produces.
fn io_err(e: std::io::Error) -> Value {
    make_error_value(e)
}

fn unwrap(r: Result<Value, Value>) -> Value {
    match r {
        Ok(v) => v,
        Err(e) => e,
    }
}

fn settle(r: Result<Value, Value>) -> Value {
    match r {
        Ok(v) => promise_resolve(&[v]),
        Err(e) => promise_reject(&[e]),
    }
}

/// Node callback form: the trailing arg is `(err, result) => …`. Run the op on the remaining
/// args, invoke the callback with `(null, value)` on success or `(err, null)` on failure, and
/// return null. The op is synchronous under the hood, so the callback fires synchronously.
fn run_callback(core: fn(&[Value]) -> Result<Value, Value>, args: &[Value]) -> Value {
    let cb = args.last().cloned().unwrap_or(Value::Null);
    let op_args: &[Value] = if args.len() > 1 { &args[..args.len() - 1] } else { &[] };
    let (err, data) = match core(op_args) {
        Ok(v) => (Value::Null, v),
        Err(e) => (e, Value::Null),
    };
    tishlang_core::value_call(&cb, &[err, data]);
    Value::Null
}

/// Generate the sync export (dual: a trailing function arg switches to the Node callback form)
/// and the promise export over a `*_core`.
macro_rules! fs_method {
    ($sync:ident, $promise:ident, $core:ident) => {
        pub fn $sync(args: &[Value]) -> Value {
            if let Some(Value::Function(_)) = args.last() {
                return run_callback($core, args);
            }
            unwrap($core(args))
        }
        pub fn $promise(args: &[Value]) -> Value {
            settle($core(args))
        }
    };
}

fn ms_since_epoch(t: std::io::Result<SystemTime>) -> f64 {
    t.ok()
        .and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Build a Node-like `Stats` object from metadata (predicate methods + size + times + mode).
///
/// FIXED key order via `object_from_pairs`: building through an `ObjectMap` (AHashMap) iterated
/// in random per-instance order made every call mint a fresh permanent chain in the global shape
/// registry (~2.3KB leaked per `stat()` call, unbounded — a file watcher stat-walking a workspace
/// leaked multi-GB/hour). Fixed-order construction dedupes to one cached shape chain, and skips
/// the intermediate hashmap entirely.
fn stats_object(md: &std::fs::Metadata) -> Value {
    let is_file = md.is_file();
    let is_dir = md.is_dir();
    let is_symlink = md.file_type().is_symlink();
    #[cfg(unix)]
    let (mode, uid, gid, ino) = {
        use std::os::unix::fs::MetadataExt;
        (md.mode() as f64, md.uid() as f64, md.gid() as f64, md.ino() as f64)
    };
    #[cfg(not(unix))]
    let (mode, uid, gid, ino) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    Value::object_from_pairs([
        ("isFile".into(), Value::native(move |_| Value::Bool(is_file))),
        ("isDirectory".into(), Value::native(move |_| Value::Bool(is_dir))),
        ("isSymbolicLink".into(), Value::native(move |_| Value::Bool(is_symlink))),
        ("isBlockDevice".into(), Value::native(|_| Value::Bool(false))),
        ("isCharacterDevice".into(), Value::native(|_| Value::Bool(false))),
        ("isFIFO".into(), Value::native(|_| Value::Bool(false))),
        ("isSocket".into(), Value::native(|_| Value::Bool(false))),
        ("size".into(), Value::Number(md.len() as f64)),
        ("mtimeMs".into(), Value::Number(ms_since_epoch(md.modified()))),
        ("atimeMs".into(), Value::Number(ms_since_epoch(md.accessed()))),
        ("birthtimeMs".into(), Value::Number(ms_since_epoch(md.created()))),
        ("mode".into(), Value::Number(mode)),
        ("uid".into(), Value::Number(uid)),
        ("gid".into(), Value::Number(gid)),
        ("ino".into(), Value::Number(ino)),
    ])
}

// ── cores ─────────────────────────────────────────────────────────────────────────────────

pub fn read_file_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::read_to_string(path_arg(args, 0))
        .map(|s| Value::String(s.into()))
        .map_err(io_err)
}

pub fn read_file_bytes_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::read(path_arg(args, 0))
        .map(|b| Value::Array(VmRef::new(b.into_iter().map(|x| Value::Number(x as f64)).collect())))
        .map_err(io_err)
}

/// Write a string, or a byte array (numbers 0–255), to a file.
pub fn write_file_core(args: &[Value]) -> Result<Value, Value> {
    let path = path_arg(args, 0);
    let res = match args.get(1) {
        Some(Value::Array(a)) => {
            let bytes: Vec<u8> = a
                .borrow()
                .iter()
                .map(|v| if let Value::Number(n) = v { *n as u8 } else { 0 })
                .collect();
            std::fs::write(&path, bytes)
        }
        Some(v) => std::fs::write(&path, v.to_display_string()),
        None => std::fs::write(&path, ""),
    };
    res.map(|_| Value::Null).map_err(io_err)
}

pub fn append_file_core(args: &[Value]) -> Result<Value, Value> {
    use std::io::Write;
    let path = path_arg(args, 0);
    let data = args.get(1).map(|v| v.to_display_string()).unwrap_or_default();
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(data.as_bytes()))
        .map(|_| Value::Null)
        .map_err(io_err)
}

pub fn stat_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::metadata(path_arg(args, 0)).map(|m| stats_object(&m)).map_err(io_err)
}
pub fn lstat_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::symlink_metadata(path_arg(args, 0)).map(|m| stats_object(&m)).map_err(io_err)
}

pub fn readdir_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::read_dir(path_arg(args, 0))
        .map(|entries| {
            let names: Vec<Value> = entries
                .filter_map(|e| e.ok())
                .map(|e| Value::String(e.file_name().to_string_lossy().into()))
                .collect();
            Value::Array(VmRef::new(names))
        })
        .map_err(io_err)
}

/// `mkdir(path[, { recursive }])` — recursive creates parents.
pub fn mkdir_core(args: &[Value]) -> Result<Value, Value> {
    let path = path_arg(args, 0);
    let recursive = opt_bool(args.get(1), "recursive");
    let res = if recursive {
        std::fs::create_dir_all(&path)
    } else {
        std::fs::create_dir(&path)
    };
    res.map(|_| Value::Null).map_err(io_err)
}

/// `rm(path[, { recursive }])` — file, or whole tree when recursive.
pub fn rm_core(args: &[Value]) -> Result<Value, Value> {
    let path = path_arg(args, 0);
    let recursive = opt_bool(args.get(1), "recursive");
    let md = std::fs::symlink_metadata(&path);
    let res = match md {
        Ok(m) if m.is_dir() => {
            if recursive {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_dir(&path)
            }
        }
        _ => std::fs::remove_file(&path),
    };
    res.map(|_| Value::Null).map_err(io_err)
}

pub fn rmdir_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::remove_dir(path_arg(args, 0)).map(|_| Value::Null).map_err(io_err)
}
pub fn unlink_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::remove_file(path_arg(args, 0)).map(|_| Value::Null).map_err(io_err)
}
pub fn rename_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::rename(path_arg(args, 0), path_arg(args, 1)).map(|_| Value::Null).map_err(io_err)
}
pub fn copy_file_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::copy(path_arg(args, 0), path_arg(args, 1)).map(|_| Value::Null).map_err(io_err)
}
pub fn realpath_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::canonicalize(path_arg(args, 0))
        .map(|p| Value::String(strip_verbatim_prefix(&p.to_string_lossy()).into()))
        .map_err(io_err)
}

/// `std::fs::canonicalize` on Windows returns the verbatim form — `\\?\C:\Users\…` for a drive
/// path, `\\?\UNC\server\share\…` for a network one. Node's `fs.realpath` never does: it hands
/// back `C:\Users\…` / `\\server\share\…`, and programs treat the result as a display / URL /
/// comparison string, not just an OS handle (Dune showed `\\?\C:\…` as its workspace title, and a
/// `?` inside a URL path starts the query string). Match Node. No-op on POSIX and for paths
/// that carry no prefix.
pub(crate) fn strip_verbatim_prefix(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = p.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}
pub fn readlink_core(args: &[Value]) -> Result<Value, Value> {
    std::fs::read_link(path_arg(args, 0))
        .map(|p| Value::String(p.to_string_lossy().into()))
        .map_err(io_err)
}
pub fn truncate_core(args: &[Value]) -> Result<Value, Value> {
    let len = match args.get(1) {
        Some(Value::Number(n)) => *n as u64,
        _ => 0,
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(path_arg(args, 0))
        .and_then(|f| f.set_len(len))
        .map(|_| Value::Null)
        .map_err(io_err)
}

/// `chmod(path, mode)` — set the file mode (a number, e.g. `0o600` = 384). Unix only; a no-op that
/// succeeds on other platforms. Needed to write a secrets file 0600.
pub fn chmod_core(args: &[Value]) -> Result<Value, Value> {
    let mode = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u32,
        _ => {
            return Err(io_err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "chmod: mode must be a number",
            )))
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path_arg(args, 0), std::fs::Permissions::from_mode(mode))
            .map(|_| Value::Null)
            .map_err(io_err)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        Ok(Value::Null)
    }
}

/// `mkdtemp(prefix)` — create a uniquely-named temp dir and return its path. Node appends 6 RANDOM
/// characters; a timestamp suffix is both predictable (a temp-dir security smell) and collision-prone
/// under rapid calls. Use a random suffix and rely on `create_dir`'s exclusive semantics, retrying on
/// the astronomically-rare collision.
pub fn mkdtemp_core(args: &[Value]) -> Result<Value, Value> {
    let prefix = path_arg(args, 0);
    for _ in 0..16 {
        // 10 base-36 chars derived from a random u64 (~51 bits of entropy).
        let mut n: u64 = rand::random();
        let mut suffix = String::with_capacity(10);
        for _ in 0..10 {
            suffix.push(char::from_digit((n % 36) as u32, 36).unwrap());
            n /= 36;
        }
        let path = format!("{}{}", prefix, suffix);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(Value::String(path.into())),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err(e)),
        }
    }
    Err(io_err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "mkdtemp: could not create a unique temporary directory",
    )))
}

/// `cp(src, dest[, { recursive }])` — copy a file, or a directory tree when recursive.
pub fn cp_core(args: &[Value]) -> Result<Value, Value> {
    let src = path_arg(args, 0);
    let dest = path_arg(args, 1);
    let recursive = opt_bool(args.get(2), "recursive");
    copy_recursive(std::path::Path::new(&src), std::path::Path::new(&dest), recursive)
        .map(|_| Value::Null)
        .map_err(io_err)
}

fn copy_recursive(src: &std::path::Path, dest: &std::path::Path, recursive: bool) -> std::io::Result<()> {
    if src.is_dir() {
        if !recursive {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cp on a directory requires { recursive: true }",
            ));
        }
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()), true)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest).map(|_| ())
    }
}

fn opt_bool(v: Option<&Value>, key: &str) -> bool {
    match v {
        Some(Value::Object(o)) => o
            .borrow()
            .strings
            .get(key)
            .map(|b| b.is_truthy())
            .unwrap_or(false),
        _ => false,
    }
}

// ── sync + promise exports ──────────────────────────────────────────────────────────────────

fs_method!(read_file, read_file_promise, read_file_core);
fs_method!(read_file_bytes, read_file_bytes_promise, read_file_bytes_core);
fs_method!(write_file, write_file_promise, write_file_core);
fs_method!(append_file, append_file_promise, append_file_core);
fs_method!(stat, stat_promise, stat_core);
fs_method!(lstat, lstat_promise, lstat_core);
fs_method!(readdir, readdir_promise, readdir_core);
fs_method!(mkdir, mkdir_promise, mkdir_core);
fs_method!(rm, rm_promise, rm_core);
fs_method!(rmdir, rmdir_promise, rmdir_core);
fs_method!(unlink, unlink_promise, unlink_core);
fs_method!(rename, rename_promise, rename_core);
fs_method!(copy_file, copy_file_promise, copy_file_core);
fs_method!(realpath, realpath_promise, realpath_core);
fs_method!(readlink, readlink_promise, readlink_core);
fs_method!(truncate, truncate_promise, truncate_core);
fs_method!(chmod, chmod_promise, chmod_core);
fs_method!(mkdtemp, mkdtemp_promise, mkdtemp_core);
fs_method!(cp, cp_promise, cp_core);

// `exists` / `access` never error — they answer a boolean.
pub fn exists(args: &[Value]) -> Value {
    Value::Bool(std::path::Path::new(&path_arg(args, 0)).exists())
}
pub fn exists_promise(args: &[Value]) -> Value {
    promise_resolve(&[exists(args)])
}
/// `accessSync(path)` → true if the path exists (tish-friendly); the promise form resolves
/// `true` / rejects with an error, matching `fs/promises.access`.
pub fn access(args: &[Value]) -> Value {
    exists(args)
}
pub fn access_promise(args: &[Value]) -> Value {
    let path = path_arg(args, 0);
    if std::path::Path::new(&path).exists() {
        promise_resolve(&[Value::Null])
    } else {
        promise_reject(&[Value::String(format!("ENOENT: no such file '{}'", path).into())])
    }
}

/// `isDir(path)` — tish convenience kept for back-compat (≈ `statSync().isDirectory()`).
pub fn is_dir(args: &[Value]) -> Value {
    Value::Bool(std::path::Path::new(&path_arg(args, 0)).is_dir())
}

/// `fs.constants` — the access-mode flags.
pub fn constants() -> Value {
    let mut m = ObjectMap::default();
    m.insert("F_OK".into(), Value::Number(0.0));
    m.insert("R_OK".into(), Value::Number(4.0));
    m.insert("W_OK".into(), Value::Number(2.0));
    m.insert("X_OK".into(), Value::Number(1.0));
    Value::object(m)
}

#[cfg(test)]
mod verbatim_prefix_tests {
    use super::strip_verbatim_prefix;

    #[test]
    fn strips_windows_verbatim_prefixes_like_node() {
        assert_eq!(strip_verbatim_prefix("\\\\?\\C:\\Users\\x\\ui"), "C:\\Users\\x\\ui");
        assert_eq!(strip_verbatim_prefix("\\\\?\\UNC\\srv\\share\\d"), "\\\\srv\\share\\d");
        assert_eq!(strip_verbatim_prefix("C:\\Users\\x"), "C:\\Users\\x");
        assert_eq!(strip_verbatim_prefix("/Users/x/ui"), "/Users/x/ui");
    }
}
