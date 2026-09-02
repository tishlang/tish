//! Raw TCP sockets for Tish (`tish:net`), behind the `net` feature.
//!
//! A client (`connect`) and a server (`listen`/`accept`) over plain TCP — the layer beneath
//! `tish:http` (HTTP) and `tish:ws` (WebSocket). Needed for protocols that ride a bare socket: the
//! Debug Adapter Protocol's TCP transports (a spawned adapter that binds a port; js-debug's reverse
//! `startDebugging` connect-back), an OAuth loopback callback listener, and connect probes (is a
//! forwarded port up yet?).
//!
//!   import { connect, read, write, close, listen, accept, probe, sleep } from 'tish:net'
//!
//!   - `connect(host, port, timeoutMs?) -> id | null`
//!   - `read(id, timeoutMs?) -> string | null`   ("" = live but no data yet; null = EOF/unknown)
//!   - `write(id, data) -> bool`
//!   - `close(id) -> bool`
//!   - `listen(port, host?) -> { id, port } | null`   (host defaults 127.0.0.1; port 0 = OS-assigned,
//!                                                      returned so a loopback caller can learn it)
//!   - `accept(listenerId, timeoutMs?) -> id | null`  (a new connection id, or null on timeout)
//!   - `probe(host, port, timeoutMs?) -> bool`        (can we connect?)
//!   - `sleep(ms) -> null`                            (blocking backoff for the poll loops above)
//!
//! Each connection has a reader thread filling a bounded, UTF-8-boundary-drained buffer shared
//! with pty.rs / process_spawn.rs (see `stream_buf.rs` for the cap/backpressure and
//! invalid-byte semantics). Global OnceLock<Mutex<HashMap>> registries; errors surface as
//! null/false.
//!
//! Connection lifecycle: `close(id)` shuts the socket down (`Shutdown::Both`) so the reader
//! thread — which holds a dup of the fd — unblocks and exits, freeing thread + fd + buffer
//! together, and the peer sees a FIN. Entries whose peer disconnected and whose buffer was
//! read to EOF (`read` returned `null`) are swept when the next connection registers, so
//! read-to-EOF-and-forget cannot strand CLOSE_WAIT fds; a swept id keeps returning `null`
//! from `read`, exactly as before.

use std::collections::HashMap;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tishlang_core::{ObjectMap, Value};

use crate::stream_buf::{drain_stream, is_drained, new_buf, retire_buf, spawn_reader, SharedBuf};

struct Conn {
    writer: Mutex<TcpStream>,
    buf: SharedBuf,
}

fn conns() -> &'static Mutex<HashMap<u64, Conn>> {
    static C: OnceLock<Mutex<HashMap<u64, Conn>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}
fn listeners() -> &'static Mutex<HashMap<u64, TcpListener>> {
    static L: OnceLock<Mutex<HashMap<u64, TcpListener>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// --- Value helpers (mirror process_spawn.rs) ---

fn arg_u64(args: &[Value], i: usize) -> Option<u64> {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

fn arg_u16(args: &[Value], i: usize) -> Option<u16> {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 && *n <= 65535.0 => Some(*n as u16),
        _ => None,
    }
}

fn arg_str(args: &[Value], i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn arg_timeout(args: &[Value], i: usize) -> u64 {
    match args.get(i) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u64,
        _ => 0,
    }
}

// Register a connected stream (reader thread + writer clone) and return its id. Also sweeps
// out entries whose peer disconnected and whose buffer was fully read (`read` already
// returned `null` for them), so read-to-EOF-and-forget cannot strand CLOSE_WAIT fds — the
// sweep never touches a conn with undrained data or a live reader, so an id that has not yet
// returned `null` keeps its full read contract.
fn register_conn(stream: TcpStream) -> Option<u64> {
    let reader = stream.try_clone().ok()?;
    let _ = reader.set_nonblocking(false);
    let buf = new_buf();
    spawn_reader(reader, buf.clone());
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let mut g = conns().lock().ok()?;
    g.retain(|_, c| !is_drained(&c.buf));
    g.insert(
        id,
        Conn {
            writer: Mutex::new(stream),
            buf,
        },
    );
    Some(id)
}

/// `connect(host, port, timeoutMs?)` → a connection id, or `null`.
pub fn net_connect(args: &[Value]) -> Value {
    let host = match arg_str(args, 0) {
        Some(h) => h,
        None => return Value::Null,
    };
    let port = match arg_u16(args, 1) {
        Some(p) => p,
        None => return Value::Null,
    };
    let timeout = arg_timeout(args, 2);
    let mut addrs = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a,
        Err(_) => return Value::Null,
    };
    let addr = match addrs.next() {
        Some(a) => a,
        None => return Value::Null,
    };
    let stream = if timeout > 0 {
        TcpStream::connect_timeout(&addr, Duration::from_millis(timeout))
    } else {
        TcpStream::connect(addr)
    };
    match stream {
        Ok(s) => match register_conn(s) {
            Some(id) => Value::Number(id as f64),
            None => Value::Null,
        },
        Err(_) => Value::Null,
    }
}

/// `read(id, timeoutMs?)` → available bytes as a string, `""`, or `null` at EOF/unknown.
pub fn net_read(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let timeout_ms = arg_timeout(args, 1);
    let buf = {
        let g = match conns().lock() {
            Ok(g) => g,
            Err(_) => return Value::Null,
        };
        match g.get(&id) {
            Some(c) => c.buf.clone(),
            None => return Value::Null,
        }
    };
    drain_stream(&buf, timeout_ms)
}

/// `write(id, data)` → returns whether the write succeeded.
pub fn net_write(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let data = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => return Value::Bool(false),
    };
    let g = match conns().lock() {
        Ok(g) => g,
        Err(_) => return Value::Bool(false),
    };
    let c = match g.get(&id) {
        Some(c) => c,
        None => return Value::Bool(false),
    };
    let mut w = match c.writer.lock() {
        Ok(w) => w,
        Err(_) => return Value::Bool(false),
    };
    match w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
        Ok(_) => Value::Bool(true),
        Err(_) => Value::Bool(false),
    }
}

/// `close(id)` → close a connection or a listener. For a connection this actually shuts the
/// socket down (`Shutdown::Both`, the `http.rs` idiom): the reader thread — which holds a dup
/// of the fd — unblocks out of its `read()`, exits, and frees thread + fd + buffer together,
/// and the peer sees a FIN instead of a silently half-alive socket. Conn and listener ids
/// share one counter, so an id names at most one of the two; closing a listener frees its
/// port — the reserve-ephemeral-then-release pattern a TCP DAP `spawn` uses so the child can
/// bind it. Returns whether anything live was removed.
pub fn net_close(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let removed_conn = match conns().lock() {
        Ok(mut g) => g.remove(&id),
        Err(_) => return Value::Bool(false),
    };
    if let Some(c) = removed_conn {
        if let Ok(w) = c.writer.lock() {
            let _ = w.shutdown(std::net::Shutdown::Both);
        }
        // Wake a reader parked at the buffer cap (shutdown only unblocks a read-blocked one).
        retire_buf(&c.buf);
        return Value::Bool(true);
    }
    let removed_listener = match listeners().lock() {
        Ok(mut g) => g.remove(&id).is_some(),
        Err(_) => return Value::Bool(false),
    };
    Value::Bool(removed_listener)
}

/// `listen(port, host?)` → `{ id, port }` (the actual bound port, so port-0 loopback callers can
/// learn the OS-assigned one), or `null`.
pub fn net_listen(args: &[Value]) -> Value {
    let port = match arg_u16(args, 0) {
        Some(p) => p,
        None => return Value::Null,
    };
    let host = arg_str(args, 1).unwrap_or_else(|| "127.0.0.1".to_string());
    let listener = match TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(_) => return Value::Null,
    };
    let bound = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(_) => port,
    };
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    match listeners().lock() {
        Ok(mut g) => {
            g.insert(id, listener);
        }
        Err(_) => return Value::Null,
    }
    let mut m = ObjectMap::default();
    m.insert(std::sync::Arc::from("id"), Value::Number(id as f64));
    m.insert(std::sync::Arc::from("port"), Value::Number(bound as f64));
    Value::object(m)
}

/// `accept(listenerId, timeoutMs?)` → a new connection id, or `null` on timeout/unknown.
pub fn net_accept(args: &[Value]) -> Value {
    let lid = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Null,
    };
    let timeout = arg_timeout(args, 1);
    // Clone the listener so we don't hold the registry lock across the poll.
    let listener = {
        let g = match listeners().lock() {
            Ok(g) => g,
            Err(_) => return Value::Null,
        };
        match g.get(&lid).and_then(|l| l.try_clone().ok()) {
            Some(l) => l,
            None => return Value::Null,
        }
    };
    if listener.set_nonblocking(true).is_err() {
        return Value::Null;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout.max(1));
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                return match register_conn(stream) {
                    Some(id) => Value::Number(id as f64),
                    None => Value::Null,
                };
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Value::Null;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return Value::Null,
        }
    }
}

/// `probe(host, port, timeoutMs?)` → whether a TCP connection can be established (then dropped).
pub fn net_probe(args: &[Value]) -> Value {
    let host = match arg_str(args, 0) {
        Some(h) => h,
        None => return Value::Bool(false),
    };
    let port = match arg_u16(args, 1) {
        Some(p) => p,
        None => return Value::Bool(false),
    };
    let timeout = arg_timeout(args, 2);
    let mut addrs = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a,
        Err(_) => return Value::Bool(false),
    };
    let addr = match addrs.next() {
        Some(a) => a,
        None => return Value::Bool(false),
    };
    let d = if timeout > 0 { timeout } else { 2000 };
    match TcpStream::connect_timeout(&addr, Duration::from_millis(d)) {
        Ok(_) => Value::Bool(true),
        Err(_) => Value::Bool(false),
    }
}

/// `sleep(ms)` → block the current OS thread for `ms` milliseconds (draining due timers first),
/// then return null. A blocking backoff for the socket poll loops that live here — a TCP DAP
/// adapter needs a moment to bind before `connect` succeeds, and `accept` pollers pace between
/// tries. Colocated with the socket ops because that's what needs it; unlike `setTimeout` (which
/// only fires when a blocking op happens to yield), this actually sleeps, so it is usable from a
/// `Promise.spawn` pump thread. Capped at 60s so a bad argument can't wedge the thread.
pub fn net_sleep(args: &[Value]) -> Value {
    let ms = arg_timeout(args, 0).min(60_000);
    if ms > 0 {
        crate::timers::sleep_with_drain(ms);
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_connect_roundtrip() {
        // Bind an ephemeral loopback listener, connect a client, exchange bytes both ways.
        let l = net_listen(&[Value::Number(0.0)]);
        let port = match &l {
            Value::Object(m) => match m.borrow().strings.get("port") {
                Some(Value::Number(p)) => *p,
                other => panic!("no port: {:?}", other),
            },
            other => panic!("listen failed: {:?}", other),
        };
        let lid = match &l {
            Value::Object(m) => match m.borrow().strings.get("id") {
                Some(Value::Number(i)) => *i,
                _ => panic!("no id"),
            },
            _ => panic!("no obj"),
        };

        let client = match net_connect(&[Value::String("127.0.0.1".into()), Value::Number(port)]) {
            Value::Number(n) => n,
            other => panic!("connect failed: {:?}", other),
        };
        let server = match net_accept(&[Value::Number(lid), Value::Number(1000.0)]) {
            Value::Number(n) => n,
            other => panic!("accept failed/timed out: {:?}", other),
        };

        // client -> server
        assert!(matches!(
            net_write(&[Value::Number(client), Value::String("ping\n".into())]),
            Value::Bool(true)
        ));
        let mut got = String::new();
        for _ in 0..100 {
            match net_read(&[Value::Number(server), Value::Number(50.0)]) {
                Value::String(s) => got.push_str(&s),
                Value::Null => break,
                _ => {}
            }
            if got.contains("ping") {
                break;
            }
        }
        assert!(got.contains("ping"), "server did not receive: {:?}", got);

        // server -> client
        assert!(matches!(
            net_write(&[Value::Number(server), Value::String("pong\n".into())]),
            Value::Bool(true)
        ));
        let mut back = String::new();
        for _ in 0..100 {
            match net_read(&[Value::Number(client), Value::Number(50.0)]) {
                Value::String(s) => back.push_str(&s),
                Value::Null => break,
                _ => {}
            }
            if back.contains("pong") {
                break;
            }
        }
        assert!(back.contains("pong"), "client did not receive: {:?}", back);

        assert!(matches!(net_close(&[Value::Number(client)]), Value::Bool(true)));
        assert!(matches!(net_close(&[Value::Number(server)]), Value::Bool(true)));
    }

    fn listen_pair() -> (f64, f64, f64) {
        let l = net_listen(&[Value::Number(0.0)]);
        let (lid, port) = match &l {
            Value::Object(m) => {
                let b = m.borrow();
                let id = match b.strings.get("id") {
                    Some(Value::Number(i)) => *i,
                    _ => panic!("no id"),
                };
                let port = match b.strings.get("port") {
                    Some(Value::Number(p)) => *p,
                    _ => panic!("no port"),
                };
                (id, port)
            }
            other => panic!("listen failed: {:?}", other),
        };
        let client = match net_connect(&[Value::String("127.0.0.1".into()), Value::Number(port)]) {
            Value::Number(n) => n,
            other => panic!("connect failed: {:?}", other),
        };
        let server = match net_accept(&[Value::Number(lid), Value::Number(1000.0)]) {
            Value::Number(n) => n,
            other => panic!("accept failed: {:?}", other),
        };
        (lid, client, server)
    }

    #[test]
    fn close_shuts_down_the_socket_so_the_peer_sees_eof() {
        // Pre-fix, net_close only dropped the writer fd: the reader thread's dup kept the
        // socket alive, no FIN was ever sent, and the peer's reads blocked forever.
        let (lid, client, server) = listen_pair();
        assert!(matches!(
            net_close(&[Value::Number(server)]),
            Value::Bool(true)
        ));
        let mut saw_eof = false;
        for _ in 0..100 {
            if let Value::Null = net_read(&[Value::Number(client), Value::Number(100.0)]) {
                saw_eof = true;
                break;
            }
        }
        assert!(saw_eof, "peer never saw EOF after close — no FIN was sent");
        let _ = net_close(&[Value::Number(client)]);
        let _ = net_close(&[Value::Number(lid)]);
    }

    #[test]
    fn conn_read_to_eof_is_swept_on_next_register() {
        // The natural "read until null, forget the id" pattern: pre-fix the Conn entry (and
        // its CLOSE_WAIT fd) survived forever.
        let (lid, client, server) = listen_pair();
        assert!(matches!(
            net_write(&[Value::Number(server), Value::String("bye".into())]),
            Value::Bool(true)
        ));
        assert!(matches!(
            net_close(&[Value::Number(server)]),
            Value::Bool(true)
        ));
        // Drain the client to EOF (null) — and never close it.
        let mut got = String::new();
        let mut saw_eof = false;
        for _ in 0..100 {
            match net_read(&[Value::Number(client), Value::Number(100.0)]) {
                Value::String(s) => got.push_str(&s),
                Value::Null => {
                    saw_eof = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_eof, "client never reached EOF");
        assert!(got.contains("bye"), "payload lost before EOF: {:?}", got);
        // Registering the next connection sweeps the drained entry.
        let (lid2, client2, server2) = listen_pair();
        assert!(
            !conns().lock().unwrap().contains_key(&(client as u64)),
            "drained-at-EOF conn was not swept"
        );
        // The read-to-EOF-then-read-again contract: the swept id still reads as null.
        assert!(matches!(net_read(&[Value::Number(client)]), Value::Null));
        let _ = net_close(&[Value::Number(client2)]);
        let _ = net_close(&[Value::Number(server2)]);
        let _ = net_close(&[Value::Number(lid)]);
        let _ = net_close(&[Value::Number(lid2)]);
    }

    #[test]
    fn probe_and_unknown() {
        // Nothing listening on this port -> probe false.
        assert!(matches!(
            net_probe(&[Value::String("127.0.0.1".into()), Value::Number(1.0), Value::Number(200.0)]),
            Value::Bool(false)
        ));
        assert!(matches!(net_read(&[Value::Number(9_999_999.0)]), Value::Null));
        assert!(matches!(net_write(&[Value::Number(9_999_999.0), Value::String("x".into())]), Value::Bool(false)));
        assert!(matches!(net_close(&[Value::Number(9_999_999.0)]), Value::Bool(false)));
    }
}
