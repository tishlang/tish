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
//! Two per-session reader threads (stdout, stderr) fill bounded byte buffers that the read fns
//! drain at a UTF-8 boundary (see `stream_buf.rs` for the cap/backpressure and invalid-byte
//! semantics). A global `OnceLock<Mutex<HashMap<id, ProcSession>>>` registry mirrors `pty.rs`;
//! errors surface as `null` / `false` rather than panicking.
//!
//! Session lifecycle: an entry lives until `kill(id)` — or, since the documented
//! `spawn -> wait -> forget` flow never kills, until it is *reaped*: once the child has exited
//! and both stream buffers are fully drained, an opportunistic sweep on each `spawn` (plus a
//! check when a read returns `null` at EOF) removes the entry. The exit code and pid live on in
//! a small bounded tombstone map so a late `wait(id)` / `pid(id)` still answers, and `kill(id)`
//! on a reaped id still reports `true` once. The child's stdin write-end is dropped as soon as
//! the exit is observed so its fd frees immediately.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tishlang_core::Value;

use crate::stream_buf::{
    drain_stream, is_drained, new_buf, retire_buf, spawn_reader, spawn_side_reader, SharedBuf,
};

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

/// Exit record kept after a session is reaped, so late `wait`/`pid`/`kill` calls still answer.
struct Tombstone {
    code: i32,
    pid: Option<u32>,
}

/// Bounded: ids are monotonic, so evicting the smallest key evicts the oldest record.
const MAX_TOMBSTONES: usize = 1024;

fn tombstones() -> &'static Mutex<BTreeMap<u64, Tombstone>> {
    static T: OnceLock<Mutex<BTreeMap<u64, Tombstone>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn remember_tombstone(id: u64, t: Tombstone) {
    if let Ok(mut m) = tombstones().lock() {
        m.insert(id, t);
        while m.len() > MAX_TOMBSTONES {
            m.pop_first();
        }
    }
}

/// Reap every session whose child has exited AND whose streams are fully drained; drop the
/// stdin write-end of any exited child so its fd frees even when the buffers still hold data.
fn sweep_sessions() {
    let mut reaped: Vec<(u64, Tombstone)> = Vec::new();
    {
        let mut g = match sessions().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        g.retain(|id, s| {
            let status = match s.child.lock() {
                Ok(mut c) => c.try_wait().ok().flatten(),
                Err(_) => None,
            };
            let Some(st) = status else { return true };
            if let Ok(mut w) = s.stdin.lock() {
                *w = None;
            }
            if is_drained(&s.out) && is_drained(&s.err) {
                reaped.push((
                    *id,
                    Tombstone {
                        code: st.code().unwrap_or(-1),
                        pid: s.pid,
                    },
                ));
                false
            } else {
                true
            }
        });
    }
    for (id, t) in reaped {
        remember_tombstone(id, t);
    }
}

/// Reap one session if (and only if) its child has exited and both streams are drained.
fn try_reap(id: u64) {
    let tomb = {
        let mut g = match sessions().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(s) = g.get(&id) else { return };
        let status = match s.child.lock() {
            Ok(mut c) => c.try_wait().ok().flatten(),
            Err(_) => None,
        };
        let Some(st) = status else { return };
        if let Ok(mut w) = s.stdin.lock() {
            *w = None;
        }
        if !(is_drained(&s.out) && is_drained(&s.err)) {
            return;
        }
        let tomb = Tombstone {
            code: st.code().unwrap_or(-1),
            pid: s.pid,
        };
        g.remove(&id);
        tomb
    };
    remember_tombstone(id, tomb);
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

/// `spawn({ program, args?, cwd?, env? })` → a stable id for read/write/wait/kill, or `null`
/// on failure. `program` is required; `args` is an array of strings; `env` entries are ADDED to
/// the inherited environment.
pub fn process_spawn(args: &[Value]) -> Value {
    // Opportunistic reap of exited-and-drained sessions, so the documented
    // spawn -> wait -> forget lifecycle cannot grow the table (or leak fds) without bound.
    sweep_sessions();

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
    // stderr is a SIDE channel programs may never read: DropOldest, not Park — parking would
    // block the child's stderr write(2) at the cap and freeze its stdout with it.
    spawn_side_reader(stderr, err.clone());

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
    let out = drain_stream(&buf, timeout_ms);
    if matches!(out, Value::Null) {
        // This stream hit EOF fully drained; if the sibling stream is done too and the child
        // has exited, the session is fully consumed — reap it (tombstone keeps wait/pid live).
        try_reap(id);
    }
    out
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
/// Still answers after the session was reaped (the exit code is tombstoned).
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
                None => {
                    // Reaped after a natural exit: the tombstoned exit code still answers.
                    return match tombstones().lock() {
                        Ok(t) => match t.get(&id) {
                            Some(tomb) => Value::Number(tomb.code as f64),
                            None => Value::Null,
                        },
                        Err(_) => Value::Null,
                    };
                }
            };
            let mut c = match s.child.lock() {
                Ok(c) => c,
                Err(_) => return Value::Null,
            };
            let status = c.try_wait();
            if let Ok(Some(_)) = status {
                // Child gone: release its stdin write-end fd immediately (the session itself
                // stays until its buffers are drained).
                if let Ok(mut w) = s.stdin.lock() {
                    *w = None;
                }
            }
            status
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

/// `kill(id)` → terminate the child and drop the session. Returns whether the id was live
/// (including a reaped id, whose child already exited — reported `true` once, like before
/// reaping existed).
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
            // Wake reader threads parked at the buffer cap so they exit (a read-blocked
            // reader exits on its own once the killed child's pipe closes).
            retire_buf(&s.out);
            retire_buf(&s.err);
            Value::Bool(true)
        }
        None => {
            // A reaped session was live from the caller's perspective; consume its tombstone.
            match tombstones().lock() {
                Ok(mut t) => Value::Bool(t.remove(&id).is_some()),
                Err(_) => Value::Bool(false),
            }
        }
    }
}

/// `pid(id)` → the child process id, or `null` for an unknown id. Still answers after the
/// session was reaped (the pid is tombstoned).
pub fn process_pid(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    {
        let g = match sessions().lock() {
            Ok(g) => g,
            Err(_) => return Value::Null,
        };
        if let Some(s) = g.get(&id) {
            return match s.pid {
                Some(p) => Value::Number(p as f64),
                None => Value::Null,
            };
        }
    }
    match tombstones().lock() {
        Ok(t) => match t.get(&id).and_then(|tomb| tomb.pid) {
            Some(p) => Value::Number(p as f64),
            None => Value::Null,
        },
        Err(_) => Value::Null,
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

    fn spawn_sh(cmd: &str) -> f64 {
        let mut m = tishlang_core::ObjectMap::default();
        m.insert(std::sync::Arc::from("program"), Value::String("sh".into()));
        let args = vec![Value::String("-c".into()), Value::String(cmd.into())];
        m.insert(
            std::sync::Arc::from("args"),
            Value::Array(tishlang_core::VmRef::new(args)),
        );
        match process_spawn(&[Value::object(m)]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        }
    }

    fn wait_code(id: f64) -> f64 {
        for _ in 0..200 {
            if let Value::Number(n) = process_wait(&[Value::Number(id), Value::Number(50.0)]) {
                return n;
            }
        }
        panic!("child did not exit in time");
    }

    fn drain_to_eof(id: f64, stderr: bool) {
        for _ in 0..200 {
            let v = if stderr {
                process_read_stderr(&[Value::Number(id), Value::Number(50.0)])
            } else {
                process_read_stdout(&[Value::Number(id), Value::Number(50.0)])
            };
            if matches!(v, Value::Null) {
                return;
            }
        }
        panic!("stream did not reach EOF in time");
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

    #[test]
    fn invalid_utf8_byte_does_not_wedge_stdout() {
        // One invalid byte, then valid text (the issue-#709 wedge shape). Pre-fix, every read
        // after the 0xFF returned "" forever while the buffer grew unbounded.
        let id = spawn_sh("printf '\\377'; echo wedge_ok_9");
        let mut acc = String::new();
        for _ in 0..200 {
            match process_read_stdout(&[Value::Number(id), Value::Number(50.0)]) {
                Value::String(s) => acc.push_str(&s),
                Value::Null => break,
                _ => {}
            }
            if acc.contains("wedge_ok_9") {
                break;
            }
        }
        let _ = process_kill(&[Value::Number(id)]);
        assert!(
            acc.contains('\u{fffd}'),
            "invalid byte not replaced: {:?}",
            acc
        );
        assert!(
            acc.contains("wedge_ok_9"),
            "stream wedged after invalid byte: {:?}",
            acc
        );
    }

    #[test]
    fn exited_and_drained_session_is_reaped_with_exit_code_tombstoned() {
        let id = spawn_sh("exit 7");
        assert_eq!(wait_code(id), 7.0);
        // Observing the exit must have released the child's stdin write-end fd already,
        // even while the session itself is still registered.
        {
            let g = sessions().lock().unwrap();
            if let Some(s) = g.get(&(id as u64)) {
                assert!(
                    s.stdin.lock().unwrap().is_none(),
                    "stdin fd not released on exit"
                );
            }
        }
        drain_to_eof(id, false);
        drain_to_eof(id, true);
        // Reading both streams to EOF reaped the session...
        assert!(
            !sessions().lock().unwrap().contains_key(&(id as u64)),
            "session not reaped after exit + full drain"
        );
        // ...but wait/pid still answer from the tombstone, and kill still reports the id as
        // having been live (once).
        assert!(matches!(process_wait(&[Value::Number(id)]), Value::Number(n) if n == 7.0));
        assert!(matches!(
            process_pid(&[Value::Number(id)]),
            Value::Number(_)
        ));
        assert!(matches!(
            process_kill(&[Value::Number(id)]),
            Value::Bool(true)
        ));
        assert!(matches!(
            process_kill(&[Value::Number(id)]),
            Value::Bool(false)
        ));
    }

    #[test]
    fn spawn_sweeps_prior_exited_sessions() {
        let id = spawn_sh("exit 3");
        assert_eq!(wait_code(id), 3.0);
        // No reads issued — wait until the reader threads observe EOF so the sweep (the only
        // eviction path exercised here) can fire on the next spawn.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            {
                let g = sessions().lock().unwrap();
                match g.get(&(id as u64)) {
                    Some(s) if is_drained(&s.out) && is_drained(&s.err) => break,
                    Some(_) => {}
                    None => break, // a concurrent test's spawn already swept it
                }
            }
            assert!(Instant::now() < deadline, "reader threads never hit EOF");
            std::thread::sleep(Duration::from_millis(5));
        }
        let id2 = spawn_cat();
        assert!(
            !sessions().lock().unwrap().contains_key(&(id as u64)),
            "sweep on spawn did not reap the exited session"
        );
        assert!(matches!(process_wait(&[Value::Number(id)]), Value::Number(n) if n == 3.0));
        let _ = process_kill(&[Value::Number(id2)]);
    }

    #[test]
    fn spawn_wait_drain_loop_does_not_accumulate_sessions() {
        // The G4 harness shape (spawn + wait, no kill) measured exactly +1 pipe fd and +1
        // table entry per iteration pre-fix. Post-fix every fully-consumed session is reaped.
        let mut ids = Vec::new();
        for _ in 0..25 {
            let id = spawn_sh("exit 0");
            assert_eq!(wait_code(id), 0.0);
            drain_to_eof(id, false);
            drain_to_eof(id, true);
            ids.push(id);
        }
        let g = sessions().lock().unwrap();
        for id in ids {
            assert!(!g.contains_key(&(id as u64)), "session {} leaked", id);
        }
    }
}
