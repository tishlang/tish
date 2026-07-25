//! Raw TCP sockets for Tish (`tish:net`), behind the `net` feature.
//!
//! A client (`connect`) and a server (`listen`/`accept`) over plain TCP — the layer beneath
//! `tish:http` (HTTP) and `tish:ws` (WebSocket). Needed for protocols that ride a bare socket: the
//! Debug Adapter Protocol's TCP transports (a spawned adapter that binds a port; js-debug's reverse
//! `startDebugging` connect-back), an OAuth loopback callback listener, and connect probes (is a
//! forwarded port up yet?).
//!
//!   import { connect, read, write, close, listen, accept, probe } from 'tish:net'
//!
//!   - `connect(host, port, timeoutMs?) -> id | null`
//!   - `read(id, timeoutMs?) -> string | null`   ("" = live but no data yet; null = EOF/unknown)
//!   - `write(id, data) -> bool`
//!   - `close(id) -> bool`
//!   - `listen(port, host?) -> { id, port } | null`   (host defaults 127.0.0.1; port 0 = OS-assigned,
//!                                                      returned so a loopback caller can learn it)
//!   - `accept(listenerId, timeoutMs?) -> id | null`  (a new connection id, or null on timeout)
//!   - `probe(host, port, timeoutMs?) -> bool`        (can we connect?)
//!
//! Each connection has a reader thread filling a UTF-8-boundary-drained buffer, exactly like
//! pty.rs / process_spawn.rs. Global OnceLock<Mutex<HashMap>> registries; errors surface as
//! null/false.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tishlang_core::{ObjectMap, Value};

struct StreamBuf {
    data: Vec<u8>,
    eof: bool,
}
type SharedBuf = Arc<(Mutex<StreamBuf>, Condvar)>;

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

fn new_buf() -> SharedBuf {
    Arc::new((
        Mutex::new(StreamBuf {
            data: Vec::new(),
            eof: false,
        }),
        Condvar::new(),
    ))
}

// Reader thread: block on the socket, append bytes, wake waiters; mark EOF on 0/err.
fn spawn_reader(mut stream: TcpStream, buf: SharedBuf) {
    std::thread::spawn(move || {
        let mut tmp = [0u8; 8192];
        loop {
            match stream.read(&mut tmp) {
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
    Value::String(String::from_utf8(out).unwrap_or_default().into())
}

// Register a connected stream (reader thread + writer clone) and return its id.
fn register_conn(stream: TcpStream) -> Option<u64> {
    let reader = stream.try_clone().ok()?;
    let _ = reader.set_nonblocking(false);
    let buf = new_buf();
    spawn_reader(reader, buf.clone());
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    conns().lock().ok()?.insert(
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

/// `close(id)` → drop the connection (the reader thread ends on EOF). Returns whether it was live.
pub fn net_close(args: &[Value]) -> Value {
    let id = match arg_u64(args, 0) {
        Some(x) => x,
        None => return Value::Bool(false),
    };
    let removed = match conns().lock() {
        Ok(mut g) => g.remove(&id),
        Err(_) => return Value::Bool(false),
    };
    Value::Bool(removed.is_some())
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
