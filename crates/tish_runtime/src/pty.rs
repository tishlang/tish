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
//! A per-session reader thread fills a byte buffer that `read` drains at a UTF-8 boundary
//! (incomplete trailing multibyte sequences are held for the next read — no mojibake). A global
//! `Mutex<HashMap<id, PtySession>>` registry mirrors `ws.rs`'s `CONNS`. Errors surface as
//! `null`/`false` rather than panicking.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use lazy_static::lazy_static;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tishlang_core::Value;

/// Output accumulated by the reader thread, drained by `read`.
struct PtyBuf {
    data: Vec<u8>,
    eof: bool,
}

struct PtySession {
    child: Mutex<Box<dyn Child + Send + Sync>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    buf: Arc<(Mutex<PtyBuf>, Condvar)>,
    pid: Option<u32>,
}

lazy_static! {
    static ref SESSIONS: Mutex<HashMap<u64, PtySession>> = Mutex::new(HashMap::new());
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

    let buf = Arc::new((
        Mutex::new(PtyBuf {
            data: Vec::new(),
            eof: false,
        }),
        Condvar::new(),
    ));

    // Reader thread: block on the master, append bytes, wake any waiting `read`. On EOF/error,
    // mark eof so `read` can return null once the buffer drains.
    {
        let buf = buf.clone();
        let mut reader = reader;
        std::thread::spawn(move || {
            let mut tmp = [0u8; 8192];
            loop {
                match reader.read(&mut tmp) {
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
/// timeout), or `null` at EOF / for an unknown id. Drains only a valid UTF-8 prefix, holding any
/// incomplete trailing multibyte bytes for the next call.
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

    let (lock, cv) = &*buf;
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

    // Drain up to the last complete UTF-8 code point; keep the incomplete tail for next time.
    let valid = match std::str::from_utf8(&b.data) {
        Ok(_) => b.data.len(),
        Err(e) => e.valid_up_to(),
    };
    if valid == 0 {
        // A leading incomplete multibyte sequence. If the stream is done, flush it lossily so
        // nothing is stranded; otherwise wait for the rest.
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

/// `kill(id)` → terminate the child and drop the session. Returns whether the id was live.
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
            Value::Bool(true)
        }
        None => Value::Bool(false),
    }
}

/// `pid(id)` → the child process id, or `null` for an unknown id / no pid.
pub fn pty_pid(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let g = match SESSIONS.lock() {
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
