//! Pseudoterminal (PTY) module for Tish (`tish:pty`), behind the `pty` feature.
//!
//! Spawns a real OS pseudoterminal with a live shell/program attached — so an interactive
//! terminal emulator (xterm.js) on the other end of a socket behaves like a real TTY:
//! `isatty()` passes, so line-editing and curses apps (vim, top, ssh, tab-completion, and
//! `SIGWINCH` resize) all work. This is what `tish:process` (run-to-completion capture) and
//! `tish:tty` (this process's OWN controlling terminal) cannot do. Imported as
//! `import { spawn, read, write, resize, kill } from 'tish:pty'`.
//!
//! Polling model (transport-agnostic — pairs with a `tish:ws` pump or an HTTP long-poll):
//!   - `spawn({ program?, cwd?, cols?, rows?, env? }) -> id | null`
//!   - `read(id, timeoutMs?) -> string | null`   (`""` = live but no output yet; `null` = EOF/unknown)
//!   - `write(id, data) -> bool`
//!   - `resize(id, cols, rows) -> bool`
//!   - `kill(id) -> bool`
//!   - `pid(id) -> number | null`
//!
//! A per-session reader thread fills a bounded byte buffer that `read` drains at a UTF-8
//! boundary (see `stream_buf.rs` for the cap/backpressure and invalid-byte semantics). A global
//! `Mutex<HashMap<id, PtySession>>` registry mirrors `ws.rs`'s `CONNS`. Errors surface as
//! `null`/`false` rather than panicking.
//!
//! Session lifecycle mirrors `process_spawn.rs`: an entry lives until `kill(id)` or until it is
//! reaped — child exited AND the buffer fully drained — by an opportunistic sweep on each
//! `spawn` (plus a check when `read` returns `null` at EOF). The pid is tombstoned (bounded) so
//! a late `pid(id)` still answers, and `kill(id)` on a reaped id still reports `true` once.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use lazy_static::lazy_static;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tishlang_core::Value;

use crate::stream_buf::{drain_stream, is_drained, new_buf, retire_buf, spawn_reader, SharedBuf};

struct PtySession {
    child: Mutex<Box<dyn Child + Send + Sync>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    buf: SharedBuf,
    pid: Option<u32>,
}

lazy_static! {
    static ref SESSIONS: Mutex<HashMap<u64, PtySession>> = Mutex::new(HashMap::new());
    /// Pids of reaped sessions, so a late `pid(id)` still answers (bounded: ids are
    /// monotonic, so evicting the smallest key evicts the oldest record).
    static ref TOMBSTONES: Mutex<BTreeMap<u64, Option<u32>>> = Mutex::new(BTreeMap::new());
}

const MAX_TOMBSTONES: usize = 1024;

fn remember_tombstone(id: u64, pid: Option<u32>) {
    if let Ok(mut m) = TOMBSTONES.lock() {
        m.insert(id, pid);
        while m.len() > MAX_TOMBSTONES {
            m.pop_first();
        }
    }
}

/// Reap every session whose child has exited AND whose buffer is fully drained
/// (`try_wait` also reaps the OS zombie).
fn sweep_sessions() {
    let mut reaped: Vec<(u64, Option<u32>)> = Vec::new();
    {
        let mut g = match SESSIONS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        g.retain(|id, s| {
            let exited = match s.child.lock() {
                Ok(mut c) => c.try_wait().ok().flatten().is_some(),
                Err(_) => false,
            };
            if exited && is_drained(&s.buf) {
                reaped.push((*id, s.pid));
                false
            } else {
                true
            }
        });
    }
    for (id, pid) in reaped {
        remember_tombstone(id, pid);
    }
}

/// Reap one session if (and only if) its child has exited and its buffer is drained.
fn try_reap(id: u64) {
    let pid = {
        let mut g = match SESSIONS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(s) = g.get(&id) else { return };
        let exited = match s.child.lock() {
            Ok(mut c) => c.try_wait().ok().flatten().is_some(),
            Err(_) => false,
        };
        if !(exited && is_drained(&s.buf)) {
            return;
        }
        let pid = s.pid;
        g.remove(&id);
        pid
    };
    remember_tombstone(id, pid);
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// --- Value helpers (mirror the ws.rs idioms) ---

fn arg_u64(args: &[Value], i: usize) -> Option<u64> {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

fn arg_u16(args: &[Value], i: usize) -> Option<u16> {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Some(*n as u16),
        _ => None,
    }
}

/// Clone a field out of an options object (`{ key: value }`), or `None` if the arg isn't an object.
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

fn obj_num(o: &Value, key: &str) -> Option<f64> {
    match obj_field(o, key) {
        Some(Value::Number(n)) if n.is_finite() => Some(n),
        _ => None,
    }
}

/// `spawn({ program?, cwd?, cols?, rows?, env? })` → a stable id for read/write/resize/kill,
/// or `null` on failure.
pub fn pty_spawn(args: &[Value]) -> Value {
    // Opportunistic reap of exited-and-drained sessions (a shell the user `exit`ed and whose
    // output was fully read), so un-killed self-exiting ptys cannot accumulate without bound.
    sweep_sessions();

    let null = Value::Null;
    let opts = args.first().unwrap_or(&null);

    let cols = obj_num(opts, "cols").filter(|n| *n > 0.0).unwrap_or(80.0) as u16;
    let rows = obj_num(opts, "rows").filter(|n| *n > 0.0).unwrap_or(24.0) as u16;

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };

    let mut cmd = if let Some(prog) = obj_str(opts, "program") {
        CommandBuilder::new(prog)
    } else if cfg!(windows) {
        CommandBuilder::new("powershell.exe")
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        CommandBuilder::new(shell)
    };

    if let Some(cwd) = obj_str(opts, "cwd") {
        cmd.cwd(cwd);
    }

    // Caller-supplied env overrides inherit the parent environment (portable_pty default); we
    // only add. Ensure TERM is set so curses apps negotiate correctly unless overridden.
    let mut has_term = false;
    if let Some(Value::Object(em)) = obj_field(opts, "env") {
        for (k, v) in em.borrow().strings.iter() {
            if k.as_ref() == "TERM" {
                has_term = true;
            }
            cmd.env(k.as_ref(), v.to_display_string());
        }
    }
    if !has_term {
        cmd.env("TERM", "xterm-256color");
    }

    let child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(_) => return Value::Null,
    };
    let reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(_) => return Value::Null,
    };
    let writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(_) => return Value::Null,
    };
    let pid = child.process_id();

    // Reader thread: block on the master, append bytes (bounded, see stream_buf.rs), wake any
    // waiting `read`. On EOF/error, mark eof so `read` can return null once the buffer drains.
    let buf = new_buf();
    spawn_reader(reader, buf.clone());

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let session = PtySession {
        child: Mutex::new(child),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        buf,
        pid,
    };
    match SESSIONS.lock() {
        Ok(mut g) => {
            g.insert(id, session);
        }
        Err(_) => return Value::Null,
    }
    Value::Number(id as f64)
}

/// `read(id, timeoutMs?)` → available output as a string (possibly `""` if none arrived within the
/// timeout), or `null` at EOF / for an unknown id. Drains only complete UTF-8, holding an
/// incomplete trailing multibyte sequence for the next call (invalid bytes become U+FFFD).
pub fn pty_read(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let timeout_ms = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u64,
        _ => 0,
    };

    let buf = {
        let g = match SESSIONS.lock() {
            Ok(g) => g,
            Err(_) => return Value::Null,
        };
        match g.get(&id) {
            Some(s) => s.buf.clone(),
            None => return Value::Null,
        }
    };

    let out = drain_stream(&buf, timeout_ms);
    if matches!(out, Value::Null) {
        // EOF fully drained: if the child has also exited, the session is fully consumed —
        // reap it (a pid tombstone keeps `pid(id)` answering).
        try_reap(id);
    }
    out
}

/// `write(id, data)` → feed input bytes to the PTY. Returns whether the write succeeded.
pub fn pty_write(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let data = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => return Value::Bool(false),
    };
    let g = match SESSIONS.lock() {
        Ok(g) => g,
        Err(_) => return Value::Bool(false),
    };
    let s = match g.get(&id) {
        Some(s) => s,
        None => return Value::Bool(false),
    };
    let mut w = match s.writer.lock() {
        Ok(w) => w,
        Err(_) => return Value::Bool(false),
    };
    match w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
        Ok(_) => Value::Bool(true),
        Err(_) => Value::Bool(false),
    }
}

/// `resize(id, cols, rows)` → tell the PTY its new window size (fires `SIGWINCH` in the child).
pub fn pty_resize(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let cols = match arg_u16(args, 1) {
        Some(c) if c > 0 => c,
        _ => return Value::Bool(false),
    };
    let rows = match arg_u16(args, 2) {
        Some(r) if r > 0 => r,
        _ => return Value::Bool(false),
    };
    let g = match SESSIONS.lock() {
        Ok(g) => g,
        Err(_) => return Value::Bool(false),
    };
    let s = match g.get(&id) {
        Some(s) => s,
        None => return Value::Bool(false),
    };
    let m = match s.master.lock() {
        Ok(m) => m,
        Err(_) => return Value::Bool(false),
    };
    match m.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(_) => Value::Bool(true),
        Err(_) => Value::Bool(false),
    }
}

/// `kill(id)` → terminate the child and drop the session. Returns whether the id was live
/// (including a reaped id, whose child already exited — reported `true` once, like before
/// reaping existed).
pub fn pty_kill(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let sess = {
        let mut g = match SESSIONS.lock() {
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
            // Wake a reader parked at the buffer cap so its thread exits (a read-blocked
            // reader exits on its own once the killed child's pty closes).
            retire_buf(&s.buf);
            Value::Bool(true)
        }
        None => {
            // A reaped session was live from the caller's perspective; consume its tombstone.
            match TOMBSTONES.lock() {
                Ok(mut t) => Value::Bool(t.remove(&id).is_some()),
                Err(_) => Value::Bool(false),
            }
        }
    }
}

/// `pid(id)` → the child process id, or `null` for an unknown id / no pid. Still answers after
/// the session was reaped (the pid is tombstoned).
pub fn pty_pid(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    {
        let g = match SESSIONS.lock() {
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
    match TOMBSTONES.lock() {
        Ok(t) => match t.get(&id).copied().flatten() {
            Some(p) => Value::Number(p as f64),
            None => Value::Null,
        },
        Err(_) => Value::Null,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Proves a LIVE interactive PTY (persistent shell on a pseudoterminal), not run-to-completion:
    /// spawn a shell, write a command, and read its executed output back.
    #[test]
    fn spawn_write_read_roundtrip() {
        let id = match pty_spawn(&[]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        };
        // Drain the shell's startup banner/prompt.
        let _ = pty_read(&[Value::Number(id), Value::Number(300.0)]);
        assert!(matches!(
            pty_write(&[Value::Number(id), Value::String("echo pty_ok_123\n".into())]),
            Value::Bool(true)
        ));
        let mut acc = String::new();
        for _ in 0..100 {
            match pty_read(&[Value::Number(id), Value::Number(50.0)]) {
                Value::String(s) => acc.push_str(&s),
                Value::Null => break,
                _ => {}
            }
            if acc.contains("pty_ok_123") {
                break;
            }
        }
        let _ = pty_kill(&[Value::Number(id)]);
        assert!(acc.contains("pty_ok_123"), "pty output missing echo: {:?}", acc);
    }

    #[test]
    fn resize_live_session_ok() {
        let id = match pty_spawn(&[]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        };
        assert!(matches!(
            pty_resize(&[Value::Number(id), Value::Number(120.0), Value::Number(40.0)]),
            Value::Bool(true)
        ));
        assert!(matches!(pty_pid(&[Value::Number(id)]), Value::Number(_)));
        assert!(matches!(pty_kill(&[Value::Number(id)]), Value::Bool(true)));
    }

    #[test]
    fn self_exited_session_is_reaped_after_drain_with_pid_tombstoned() {
        // `true` exits immediately on its own — the un-killed lifecycle that used to strand
        // the session (2 fds + an unreaped child) forever.
        let mut m = tishlang_core::ObjectMap::default();
        m.insert(
            std::sync::Arc::from("program"),
            Value::String("true".into()),
        );
        let id = match pty_spawn(&[Value::object(m)]) {
            Value::Number(n) => n,
            other => panic!("spawn failed: {:?}", other),
        };
        // Drain to EOF; the null read reaps the exited session.
        let mut saw_eof = false;
        for _ in 0..200 {
            if matches!(
                pty_read(&[Value::Number(id), Value::Number(50.0)]),
                Value::Null
            ) {
                saw_eof = true;
                break;
            }
        }
        assert!(saw_eof, "pty never reached EOF");
        assert!(
            !SESSIONS.lock().unwrap().contains_key(&(id as u64)),
            "session not reaped after exit + full drain"
        );
        // The pid still answers from the tombstone; kill reports the id as (once) live;
        // reads keep returning null (the read-to-EOF-then-read-again contract).
        assert!(matches!(pty_read(&[Value::Number(id)]), Value::Null));
        assert!(matches!(pty_pid(&[Value::Number(id)]), Value::Number(_)));
        assert!(matches!(pty_kill(&[Value::Number(id)]), Value::Bool(true)));
        assert!(matches!(pty_kill(&[Value::Number(id)]), Value::Bool(false)));
    }

    #[test]
    fn unknown_id_surfaces_null_or_false() {
        let bad = 9_999_999.0;
        assert!(matches!(pty_read(&[Value::Number(bad)]), Value::Null));
        assert!(matches!(
            pty_write(&[Value::Number(bad), Value::String("x".into())]),
            Value::Bool(false)
        ));
        assert!(matches!(
            pty_resize(&[Value::Number(bad), Value::Number(80.0), Value::Number(24.0)]),
            Value::Bool(false)
        ));
        assert!(matches!(pty_kill(&[Value::Number(bad)]), Value::Bool(false)));
        assert!(matches!(pty_pid(&[Value::Number(bad)]), Value::Null));
    }
}
