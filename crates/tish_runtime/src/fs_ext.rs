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
fn stats_object(md: &std::fs::Metadata) -> Value {
    let mut m = ObjectMap::default();
    let is_file = md.is_file();
    let is_dir = md.is_dir();
    let is_symlink = md.file_type().is_symlink();
    m.insert("isFile".into(), Value::native(move |_| Value::Bool(is_file)));
    m.insert("isDirectory".into(), Value::native(move |_| Value::Bool(is_dir)));
    m.insert("isSymbolicLink".into(), Value::native(move |_| Value::Bool(is_symlink)));
    m.insert("isBlockDevice".into(), Value::native(|_| Value::Bool(false)));
    m.insert("isCharacterDevice".into(), Value::native(|_| Value::Bool(false)));
    m.insert("isFIFO".into(), Value::native(|_| Value::Bool(false)));
    m.insert("isSocket".into(), Value::native(|_| Value::Bool(false)));
    m.insert("size".into(), Value::Number(md.len() as f64));
    m.insert("mtimeMs".into(), Value::Number(ms_since_epoch(md.modified())));
    m.insert("atimeMs".into(), Value::Number(ms_since_epoch(md.accessed())));
    m.insert("birthtimeMs".into(), Value::Number(ms_since_epoch(md.created())));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.insert("mode".into(), Value::Number(md.mode() as f64));
        m.insert("uid".into(), Value::Number(md.uid() as f64));
        m.insert("gid".into(), Value::Number(md.gid() as f64));
        m.insert("ino".into(), Value::Number(md.ino() as f64));
    }
    Value::object(m)
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
        .map(|p| Value::String(p.to_string_lossy().into()))
        .map_err(io_err)
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
