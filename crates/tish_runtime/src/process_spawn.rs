//! Pipe-based child-process streaming for Tish (`tish:process` spawn/read/write/kill/wait),
//! behind the `process` feature.
//!
//! Unlike `tish:pty` (a real pseudoterminal — line editing, SIGWINCH, isatty) and unlike
//! `process.execCapture` (run-to-completion, fully buffered), this spawns a child with PLAIN
//! PIPES for stdin/stdout/stderr and streams them incrementally. That is the shape language
//! servers (LSP), debug adapters (DAP), MCP servers, and the node extension host need: their
//! stdio carries framed protocols (`Content-Length` headers, newline-delimited JSON) that a
//! pseudoterminal's line discipline would corrupt, and they are long-lived, so capture-only
//! `execCapture` cannot drive them.
//!
//!   import { spawn, readStdout, readStderr, writeStdin, closeStdin, wait, kill, pid } from 'tish:process'
//!
//!   - `spawn({ program, args?, cwd?, env? }) -> id | null`
//!   - `readStdout(id, timeoutMs?) -> string | null`   (`""` = live but no output yet; `null` = EOF/unknown)
//!   - `readStderr(id, timeoutMs?) -> string | null`
//!   - `writeStdin(id, data) -> bool`
//!   - `closeStdin(id) -> bool`     (drop the child's stdin → sends EOF; some servers need it)
//!   - `wait(id, timeoutMs?) -> number | null`   (exit code once exited; `null` while still running / unknown)
//!   - `kill(id) -> bool`
//!   - `pid(id) -> number | null`
//!
//! Two per-session reader threads (stdout, stderr) fill byte buffers that the read fns drain at
//! a UTF-8 boundary — an incomplete trailing multibyte sequence is held for the next read (no
//! mojibake). A global `OnceLock<Mutex<HashMap<id, ProcSession>>>` registry mirrors `pty.rs`;
//! errors surface as `null` / `false` rather than panicking.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tishlang_core::Value;

/// Output accumulated by a reader thread, drained by `readStdout` / `readStderr`.
struct StreamBuf {
    data: Vec<u8>,
    eof: bool,
}

type SharedBuf = Arc<(Mutex<StreamBuf>, Condvar)>;

struct ProcSession {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    out: SharedBuf,
    err: SharedBuf,
    pid: Option<u32>,
}

fn sessions() -> &'static Mutex<HashMap<u64, ProcSession>> {
    static S: OnceLock<Mutex<HashMap<u64, ProcSession>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// --- Value helpers (mirror pty.rs idioms) ---

fn arg_u64(args: &[Value], i: usize) -> Option<u64> {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

fn arg_timeout(args: &[Value], i: usize) -> u64 {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u64,
        _ => 0,
    }
}

fn obj_field(o: &Value, key: &str) -> Option<Value> {
    if let Value::Object(m) = o {
        return m.borrow().strings.get(key).cloned();
    }
    None
}

fn obj_str(o: &Value, key: &str) -> Option<String> {
    match obj_field(o, key) {
        Some(Value::String(s)) => Some(s.to_string()),
        Some(Value::Null) | None => None,
        Some(v) => Some(v.to_display_string()),
    }
}

fn new_buf() -> SharedBuf {
    Arc::new((
        Mutex::new(StreamBuf {
            data: Vec::new(),
            eof: false,
        }),
        Condvar::new(),
    ))
}

/// Reader thread: block on a pipe, append bytes, wake any waiting reader; mark EOF on 0/err.
fn spawn_reader<R: Read + Send + 'static>(mut r: R, buf: SharedBuf) {
    std::thread::spawn(move || {
        let mut tmp = [0u8; 8192];
        loop {
            match r.read(&mut tmp) {
                Ok(0) => {
                    let (lock, cv) = &*buf;
                    if let Ok(mut b) = lock.lock() {
                        b.eof = true;
                    }
                    cv.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, cv) = &*buf;
                    if let Ok(mut b) = lock.lock() {
                        b.data.extend_from_slice(&tmp[..n]);
                    }
                    cv.notify_all();
                }
                Err(_) => {
                    let (lock, cv) = &*buf;
                    if let Ok(mut b) = lock.lock() {
                        b.eof = true;
                    }
                    cv.notify_all();
                    break;
                }
            }
        }
    });
}

/// Drain a stream buffer as a string (possibly `""` within the timeout), or `null` at EOF.
/// Drains only a valid UTF-8 prefix, holding any incomplete trailing multibyte for next time
/// (identical semantics to `pty::pty_read`).
fn drain_stream(buf: &SharedBuf, timeout_ms: u64) -> Value {
    let (lock, cv) = &**buf;
    let mut b = match lock.lock() {
        Ok(b) => b,
        Err(_) => return Value::Null,
    };
    if b.data.is_empty() && !b.eof && timeout_ms > 0 {
        let res = cv.wait_timeout_while(b, Duration::from_millis(timeout_ms), |b| {
            b.data.is_empty() && !b.eof
        });
        b = match res {
            Ok((g, _)) => g,
            Err(e) => e.into_inner().0,
        };
    }
    if b.data.is_empty() {
        if b.eof {
            return Value::Null;
        }
        return Value::String("".into());
    }
    let valid = match std::str::from_utf8(&b.data) {
        Ok(_) => b.data.len(),
        Err(e) => e.valid_up_to(),
    };
    if valid == 0 {
        if b.eof {
            let s = String::from_utf8_lossy(&b.data).into_owned();
            b.data.clear();
            return Value::String(s.into());
        }
        return Value::String("".into());
    }
    let out: Vec<u8> = b.data.drain(..valid).collect();
    let s = String::from_utf8(out).unwrap_or_default();
    Value::String(s.into())
}

/// `spawn({ program, args?, cwd?, env? })` → a stable id for read/write/wait/kill, or `null`
/// on failure. `program` is required; `args` is an array of strings; `env` entries are ADDED to
/// the inherited environment.
pub fn process_spawn(args: &[Value]) -> Value {
    let null = Value::Null;
    let opts = args.first().unwrap_or(&null);

    let program = match obj_str(opts, "program") {
        Some(p) if !p.is_empty() => p,
        _ => return Value::Null,
    };

    let mut cmd = Command::new(program);
    if let Some(Value::Array(a)) = obj_field(opts, "args") {
        for v in a.borrow().iter() {
            cmd.arg(v.to_display_string());
        }
    }
    if let Some(cwd) = obj_str(opts, "cwd") {
        cmd.current_dir(cwd);
    }
    if let Some(Value::Object(em)) = obj_field(opts, "env") {
        for (k, v) in em.borrow().strings.iter() {
            cmd.env(k.as_ref(), v.to_display_string());
        }
    }

    // `detached: true` — fire-and-forget launch (a GUI app, an ssh -L tunnel, the `dune` CLI's own
    // window op): null stdio + a fresh process group so the child outlives this process and isn't hit
    // by signals to the parent's group. Returns the child PID (a number) with NO session registered —
    // there's nothing to read/write/wait/kill.
    if matches!(obj_field(opts, "detached"), Some(Value::Bool(true))) {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        return match cmd.spawn() {
            Ok(child) => Value::Number(child.id() as f64),
            Err(_) => Value::Null,
        };
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Value::Null,
    };

    let stdin = child.stdin.take();
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return Value::Null,
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => return Value::Null,
    };
    let pid = Some(child.id());

    let out = new_buf();
    let err = new_buf();
    spawn_reader(stdout, out.clone());
    spawn_reader(stderr, err.clone());

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let session = ProcSession {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        out,
        err,
        pid,
    };
    match sessions().lock() {
        Ok(mut g) => {
            g.insert(id, session);
        }
        Err(_) => return Value::Null,
    }
    Value::Number(id as f64)
}

fn read_from(args: &[Value], stderr: bool) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let timeout_ms = arg_timeout(args, 1);
    let buf = {
        let g = match sessions().lock() {
            Ok(g) => g,
            Err(_) => return Value::Null,
        };
        match g.get(&id) {
            Some(s) => {
                if stderr {
                    s.err.clone()
                } else {
                    s.out.clone()
                }
            }
            None => return Value::Null,
        }
    };
    drain_stream(&buf, timeout_ms)
}

/// `readStdout(id, timeoutMs?)` → available stdout, `""` (live, none yet), or `null` at EOF/unknown.
pub fn process_read_stdout(args: &[Value]) -> Value {
    read_from(args, false)
}

/// `readStderr(id, timeoutMs?)` → available stderr, `""` (live, none yet), or `null` at EOF/unknown.
pub fn process_read_stderr(args: &[Value]) -> Value {
    read_from(args, true)
}

/// `writeStdin(id, data)` → feed bytes to the child's stdin. Returns whether the write succeeded.
pub fn process_write_stdin(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let data = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => return Value::Bool(false),
    };
    let g = match sessions().lock() {
        Ok(g) => g,
        Err(_) => return Value::Bool(false),
    };
    let s = match g.get(&id) {
        Some(s) => s,
        None => return Value::Bool(false),
    };
    let mut w = match s.stdin.lock() {
        Ok(w) => w,
        Err(_) => return Value::Bool(false),
    };
    match w.as_mut() {
        Some(stdin) => match stdin.write_all(data.as_bytes()).and_then(|_| stdin.flush()) {
            Ok(_) => Value::Bool(true),
            Err(_) => Value::Bool(false),
        },
        None => Value::Bool(false),
    }
}

/// `closeStdin(id)` → drop the child's stdin, sending EOF. Returns whether stdin was open.
pub fn process_close_stdin(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let g = match sessions().lock() {
        Ok(g) => g,
        Err(_) => return Value::Bool(false),
    };
    let s = match g.get(&id) {
        Some(s) => s,
        None => return Value::Bool(false),
    };
    let mut w = match s.stdin.lock() {
        Ok(w) => w,
        Err(_) => return Value::Bool(false),
    };
    // Dropping the ChildStdin closes the pipe.
    Value::Bool(w.take().is_some())
}

/// `wait(id, timeoutMs?)` → the child's exit code once it has exited, or `null` while it is still
/// running (or for an unknown id). Polls up to `timeoutMs` (0 = a single non-blocking check).
pub fn process_wait(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let timeout_ms = arg_timeout(args, 1);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        // Compute try_wait() into an owned result inside a tight scope so the sessions guard and
        // the child guard both drop before the loop's sleep (avoids holding locks across the wait).
        let status = {
            let g = match sessions().lock() {
                Ok(g) => g,
                Err(_) => return Value::Null,
            };
            let s = match g.get(&id) {
                Some(s) => s,
                None => return Value::Null,
            };
            let mut c = match s.child.lock() {
                Ok(c) => c,
                Err(_) => return Value::Null,
            };
            c.try_wait()
        };
        match status {
            Ok(Some(st)) => return Value::Number(st.code().unwrap_or(-1) as f64),
            Ok(None) => {}
            Err(_) => return Value::Null,
        }
        if Instant::now() >= deadline {
            return Value::Null;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `kill(id)` → terminate the child and drop the session. Returns whether the id was live.
pub fn process_kill(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let sess = {
        let mut g = match sessions().lock() {
            Ok(g) => g,
            Err(_) => return Value::Bool(false),
        };
        g.remove(&id)
    };
    match sess {
        Some(s) => {
            if let Ok(mut c) = s.child.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
            Value::Bool(true)
        }
        None => Value::Bool(false),
    }
}

/// `pid(id)` → the child process id, or `null` for an unknown id.
pub fn process_pid(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let g = match sessions().lock() {
        Ok(g) => g,
        Err(_) => return Value::Null,
    };
    match g.get(&id).and_then(|s| s.pid) {
        Some(p) => Value::Number(p as f64),
        None => Value::Null,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn spawn_cat() -> f64 {
        // `cat` echoes stdin → stdout, a clean full-duplex pipe streaming target.
        let mut m = tishlang_core::ObjectMap::default();
        m.insert(std::sync::Arc::from("program"), Value::String("cat".into()));
        match process_spawn(&[Value::object(m)]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        }
    }

    #[test]
    fn spawn_write_read_roundtrip() {
        let id = spawn_cat();
        assert!(matches!(
            process_write_stdin(&[Value::Number(id), Value::String("hello_pipe_42\n".into())]),
            Value::Bool(true)
        ));
        let mut acc = String::new();
        for _ in 0..100 {
            match process_read_stdout(&[Value::Number(id), Value::Number(50.0)]) {
                Value::String(s) => acc.push_str(&s),
                Value::Null => break,
                _ => {}
            }
            if acc.contains("hello_pipe_42") {
                break;
            }
        }
        assert!(matches!(process_pid(&[Value::Number(id)]), Value::Number(_)));
        assert!(matches!(process_kill(&[Value::Number(id)]), Value::Bool(true)));
        assert!(acc.contains("hello_pipe_42"), "pipe output missing: {:?}", acc);
    }

    #[test]
    fn wait_reports_exit_code() {
        let mut m = tishlang_core::ObjectMap::default();
        m.insert(std::sync::Arc::from("program"), Value::String("sh".into()));
        let mut args = Vec::new();
        args.push(Value::String("-c".into()));
        args.push(Value::String("exit 7".into()));
        m.insert(
            std::sync::Arc::from("args"),
            Value::Array(tishlang_core::VmRef::new(args)),
        );
        let id = match process_spawn(&[Value::object(m)]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        };
        let mut code = Value::Null;
        for _ in 0..100 {
            code = process_wait(&[Value::Number(id), Value::Number(50.0)]);
            if !matches!(code, Value::Null) {
                break;
            }
        }
        assert!(matches!(code, Value::Number(n) if n == 7.0), "exit code: {:?}", code);
        let _ = process_kill(&[Value::Number(id)]);
    }

    #[test]
    fn unknown_id_surfaces_null_or_false() {
        let bad = 9_999_999.0;
        assert!(matches!(process_read_stdout(&[Value::Number(bad)]), Value::Null));
        assert!(matches!(
            process_write_stdin(&[Value::Number(bad), Value::String("x".into())]),
            Value::Bool(false)
        ));
        assert!(matches!(process_wait(&[Value::Number(bad)]), Value::Null));
        assert!(matches!(process_kill(&[Value::Number(bad)]), Value::Bool(false)));
        assert!(matches!(process_pid(&[Value::Number(bad)]), Value::Null));
    }
}
