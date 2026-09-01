//! WebSocket module for Tish (tish:ws).
//!
//! Node.js `ws`-compatible API:
//! - **Server**: `Server({ port })` — has `clients` (array of LIVE conns; closed ones are pruned
//!   lazily), `on('connection', fn)`, `listen()`, `acceptTimeout(server, ms)`, `close()`
//! - **Connection**: `send(data)` (returns `false` when the conn is gone OR its bounded outbound
//!   queue is full — see `TISH_WS_SEND_BUF_BYTES`), `close()`, `readyState` (1=OPEN),
//!   `receive()` / `receiveTimeout(ms)`
//! - **Broadcast** (Node pattern): `server.clients.forEach(ws => ws.send(data))` or iterate room conns and `wsSend(ws, data)` (same as `ws.send(data)`)

use tishlang_core::VmRef;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use lazy_static::lazy_static;
use tishlang_core::{ObjectMap, Value};
use tokio::runtime::Runtime;
use tokio::sync::mpsc as tokio_mpsc;

thread_local! {
    /// Multi-thread runtime so `tokio::spawn` I/O tasks keep running after `block_on` returns.
    static WS_CLIENT_RT: std::cell::RefCell<Option<Runtime>> = const { std::cell::RefCell::new(None) };
}

fn with_ws_client_rt<F, R>(f: F) -> R
where
    F: FnOnce(&Runtime) -> R,
{
    WS_CLIENT_RT.with(|cell| {
        let mut b = cell.borrow_mut();
        if b.is_none() {
            *b = Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("ws client tokio runtime"),
            );
        }
        f(b.as_ref().expect("ws runtime"))
    })
}

static NEXT_CONN_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_SERVER_HANDLE: AtomicU32 = AtomicU32::new(1);

fn next_conn_id() -> u32 {
    NEXT_CONN_ID.fetch_add(1, Ordering::SeqCst)
}

fn next_server_handle() -> u32 {
    NEXT_SERVER_HANDLE.fetch_add(1, Ordering::SeqCst)
}

/// Install rustls' process-default `CryptoProvider` (once) so `connect_async`'s `wss://` path has a
/// provider to use. Both aws-lc-rs and ring are in the dep tree (reqwest + transitive), so rustls
/// 0.23 refuses to auto-select; without this, the first wss connect fails with "Could not determine
/// the process-level CryptoProvider". reqwest configures its own client explicitly, so it's
/// unaffected by (and doesn't set) the process default. Idempotent: a second install is a no-op.
fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// WS diagnostics are OFF by default — the client used to print on every connect/close, which is
/// production noise for a long-lived transport (remote pty / watch). Set `TISH_WS_DEBUG=1` to
/// restore the per-connection stderr logging.
fn ws_debug() -> bool {
    use std::sync::OnceLock;
    static D: OnceLock<bool> = OnceLock::new();
    *D.get_or_init(|| {
        std::env::var("TISH_WS_DEBUG")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

struct ConnState {
    send_tx: tokio_mpsc::Sender<String>,
    recv_rx: mpsc::Receiver<String>,
    /// Bytes accepted by `conn_send` and not yet written to the socket. Shared with the write task,
    /// which decrements as each write completes; `conn_send` refuses (returns false) once
    /// `ws_send_buf_bytes()` is exceeded so a peer that stops reading can't grow the queue forever.
    queued_bytes: Arc<AtomicUsize>,
    /// Abort handle for the read pump task. `unregister` aborts it so an explicit `close()` frees
    /// the task, socket fd and read buffer instead of leaving the task parked on `read.next()`
    /// until the REMOTE peer disconnects.
    read_abort: Option<tokio::task::AbortHandle>,
    #[allow(dead_code)]
    open: bool,
}

/// A listening `tish:ws` server: the queue of accepted connection ids plus the accept loop's
/// shutdown signal (see `server_close`).
struct ServerState {
    conn_rx: mpsc::Receiver<u32>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

lazy_static! {
    static ref CONNS: Mutex<HashMap<u32, ConnState>> = Mutex::new(HashMap::new());
    static ref SERVER_RECV: Mutex<HashMap<u32, ServerState>> = Mutex::new(HashMap::new());
}

fn register(
    send_tx: tokio_mpsc::Sender<String>,
    recv_rx: mpsc::Receiver<String>,
    queued_bytes: Arc<AtomicUsize>,
) -> u32 {
    let id = next_conn_id();
    CONNS.lock().unwrap().insert(
        id,
        ConnState {
            send_tx,
            recv_rx,
            queued_bytes,
            read_abort: None,
            open: true,
        },
    );
    id
}

/// Store the read pump's abort handle on an already-registered connection so `unregister` can
/// cancel it. If the connection has vanished in the meantime (instant disconnect), abort right away.
fn attach_read_task(id: u32, task: &tokio::task::JoinHandle<()>) {
    let handle = task.abort_handle();
    if let Ok(mut guard) = CONNS.lock() {
        match guard.get_mut(&id) {
            Some(state) => state.read_abort = Some(handle),
            None => handle.abort(),
        }
    }
}

fn unregister(id: u32) {
    let state = CONNS.lock().ok().and_then(|mut g| g.remove(&id));
    if let Some(state) = state {
        // Cancel the read pump. Without this an explicit close() left the task parked on
        // `read.next().await` forever, pinning the socket fd and the tungstenite read buffer until
        // the remote peer disconnected. Dropping `state` also drops `send_tx`, which ends the write
        // task — and the write task drives a Close frame + socket shutdown on its way out.
        if let Some(handle) = state.read_abort {
            handle.abort();
        }
    }
}

/// Bridge an already-handshaked WebSocket stream into the `CONNS` registry — spawning the read and
/// write pump tasks — and return its connection id. Shared by the `tish:ws` server-accept loop and
/// the `serve()` HTTP→WS upgrade path (see `http_hyper.rs`). Must be called inside a tokio runtime.
pub(crate) fn register_ws_stream<S>(ws_stream: tokio_tungstenite::WebSocketStream<S>) -> u32
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (send_tx, mut send_rx) = tokio_mpsc::channel::<String>(SEND_QUEUE_MAX_MSGS);
    let (recv_tx, recv_rx) = mpsc::sync_channel::<String>(64);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    let id = register(send_tx, recv_rx, Arc::clone(&queued_bytes));
    let recv_tx_task = Arc::new(recv_tx);
    let (mut write, mut read) = ws_stream.split();
    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = read.next().await {
            match msg {
                tokio_tungstenite::tungstenite::Message::Text(t) => {
                    // A dropped receiver means the connection was closed and nobody will ever read
                    // these frames — stop pumping instead of discarding them forever.
                    if recv_tx_task.send(t.to_string()).is_err() {
                        break;
                    }
                }
                // Binary frames are delivered too (utf8-lossy) — remote pty streams raw bytes and
                // some servers frame as Binary; dropping them silently loses output. Ping/Pong/Close
                // are handled by the stream (the loop ends when it yields None on close).
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    if recv_tx_task
                        .send(String::from_utf8_lossy(&b).into_owned())
                        .is_err()
                    {
                        break;
                    }
                }
                _ => {}
            }
        }
        unregister(id);
    });
    attach_read_task(id, &read_task);
    tokio::spawn(async move {
        while let Some(text) = send_rx.recv().await {
            let n = text.len();
            let _ = write
                .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                .await;
            queued_bytes.fetch_sub(n, Ordering::AcqRel);
        }
        // Channel closed: the connection was unregistered (explicit close() or remote disconnect).
        // Drive the closing handshake so the socket actually shuts down instead of leaking the fd.
        let _ = write.close().await;
    });
    id
}

lazy_static! {
    /// Connections upgraded from HTTP by `serve()` (http_hyper), waiting for the program to pick them
    /// up via `wsAccept`. Per-process: an upgraded socket stays in whatever SO_REUSEPORT worker
    /// accepted it, so each worker drains its own queue and there's no cross-process routing problem.
    static ref UPGRADE_QUEUE: (Mutex<std::collections::VecDeque<u32>>, std::sync::Condvar) =
        (Mutex::new(std::collections::VecDeque::new()), std::sync::Condvar::new());
}

/// Enqueue a `serve()`-upgraded connection id for `wsAccept` to hand to the program.
pub(crate) fn enqueue_upgraded_conn(id: u32) {
    let (m, cv) = &*UPGRADE_QUEUE;
    if let Ok(mut q) = m.lock() {
        q.push_back(id);
    }
    cv.notify_all();
}

/// `wsAccept(timeoutMs?)` — return the next connection upgraded on the HTTP server's port (see the
/// `serve()` HTTP→WS upgrade path) as a normal `tish:ws` connection object, or `Null` if none arrives
/// within the timeout (default 0 = non-blocking).
pub fn ws_serve_accept(args: &[Value]) -> Value {
    let timeout_ms = match args.first() {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => (*n as u64).min(3_600_000),
        _ => 0,
    };
    let (m, cv) = &*UPGRADE_QUEUE;
    let mut q = match m.lock() {
        Ok(q) => q,
        Err(_) => return Value::Null,
    };
    if q.is_empty() && timeout_ms > 0 {
        let res = cv.wait_timeout_while(
            q,
            std::time::Duration::from_millis(timeout_ms),
            |q| q.is_empty(),
        );
        q = match res {
            Ok((g, _)) => g,
            Err(e) => e.into_inner().0,
        };
    }
    match q.pop_front() {
        Some(id) => conn_object(id),
        None => Value::Null,
    }
}

/// Max simultaneous WebSocket connections. Each accepted conn registers in `CONNS` and
/// spawns tasks + channels, freed only on close — unbounded accepts exhaust tasks/memory.
/// Override with `TISH_WS_MAX_CONNS`; default 10000.
fn max_ws_connections() -> usize {
    use std::sync::OnceLock;
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("TISH_WS_MAX_CONNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000)
    })
}

/// Can another connection register under `max_ws_connections()`? Shared accounting point for the
/// `tish:ws` listener AND the `serve()` HTTP→WS upgrade path (http_hyper), which previously
/// registered pre-auth connections with no cap at all.
pub(crate) fn has_ws_capacity() -> bool {
    CONNS
        .lock()
        .map(|c| c.len() < max_ws_connections())
        .unwrap_or(false)
}

/// Slot count of the bounded per-connection send queue. The byte budget (`ws_send_buf_bytes`) is
/// the primary limit; this bounds per-message channel overhead when frames are tiny.
const SEND_QUEUE_MAX_MSGS: usize = 1024;

/// Per-connection outbound byte budget. `conn_send` refuses (returns false) once this many bytes
/// sit queued waiting on the socket, so a peer that stops reading gets backpressure instead of
/// unbounded queue growth. A single message is always admitted into an EMPTY queue even when it
/// exceeds the budget (so a legitimately large frame can still be sent); the queue is therefore
/// bounded by `budget + one message`. Override with `TISH_WS_SEND_BUF_BYTES`; default 4 MiB.
fn ws_send_buf_bytes() -> usize {
    use std::sync::OnceLock;
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("TISH_WS_SEND_BUF_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4 * 1024 * 1024)
    })
}

/// Shared tungstenite config for every connection (server-accept, `serve()` upgrade, client):
/// 16 KiB read/write buffers instead of the 128 KiB defaults — the read buffer is allocated eagerly
/// per connection, so the defaults authorize ~1.3 GB across the 10k-conn cap before any traffic —
/// and a finite `max_write_buffer_size` so a failing socket can't grow the write buffer without
/// bound.
pub(crate) fn ws_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(16 * 1024)
        .max_write_buffer_size(ws_send_buf_bytes().saturating_mul(2).max(1 << 20))
}

/// Queue `data` for the connection's write task. Returns `false` when the connection is gone OR
/// when the outbound queue is full (byte budget `ws_send_buf_bytes()` / `SEND_QUEUE_MAX_MSGS`
/// slots) — callers already treat `false` as "not delivered". Never blocks the caller.
fn conn_send(id: u32, data: String) -> bool {
    let guard = match CONNS.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let state = match guard.get(&id) {
        Some(s) if s.open => s,
        _ => return false,
    };
    let len = data.len();
    let queued = state.queued_bytes.fetch_add(len, Ordering::AcqRel);
    // Refuse over-budget sends, but always admit one message into an empty queue (see
    // `ws_send_buf_bytes`) — mirrors tungstenite's own "buffer + one message" convention.
    if queued > 0 && queued.saturating_add(len) > ws_send_buf_bytes() {
        state.queued_bytes.fetch_sub(len, Ordering::AcqRel);
        return false;
    }
    match state.send_tx.try_send(data) {
        Ok(()) => true,
        Err(_) => {
            state.queued_bytes.fetch_sub(len, Ordering::AcqRel);
            false
        }
    }
}

/// Default timeout for receive() so the main thread blocks and keeps the process/runtime alive.
const RECV_DEFAULT_TIMEOUT_MS: u64 = 2000;

fn conn_receive(id: u32) -> Option<String> {
    conn_receive_timeout(id, RECV_DEFAULT_TIMEOUT_MS)
}

/// Block for up to timeout_ms; returns Some(msg) or None on timeout/disconnect.
/// Uses try_recv in a loop to avoid holding CONNS lock while blocking (prevents deadlock
/// when connection closes and tokio task needs to unregister).
fn conn_receive_timeout(id: u32, timeout_ms: u64) -> Option<String> {
    let timeout_ms = timeout_ms.min(3_600_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let poll_interval = Duration::from_millis(50);
    loop {
        let result = {
            let guard = match CONNS.lock() {
                Ok(g) => g,
                Err(_) => return None,
            };
            if !guard.contains_key(&id) {
                drop(guard);
                // Dead/unknown id: return promptly, but pace tight caller loops that poll a closed
                // conn — sleeping never past the caller's own deadline. `receiveTimeout(0)` must
                // cost 0ms, not a flat 50ms.
                let remaining = deadline.saturating_duration_since(Instant::now());
                if !remaining.is_zero() {
                    std::thread::sleep(remaining.min(poll_interval));
                }
                return None;
            }
            guard.get(&id).unwrap().recv_rx.try_recv()
        };
        match result {
            Ok(s) => return Some(s),
            Err(mpsc::TryRecvError::Disconnected) => return None,
            Err(mpsc::TryRecvError::Empty) => {
                if Instant::now() >= deadline {
                    return None;
                }
                crate::timers::sleep_with_drain(poll_interval.as_millis() as u64);
            }
        }
    }
}

/// Native send: avoids method-call path. Takes conn object (with _id) and string data.
pub fn ws_send_native(conn: &Value, data: &str) -> bool {
    let id = conn_id_from_value(conn);
    match id {
        Some(id) => conn_send(id, data.to_string()),
        None => false,
    }
}

/// Extract connection id from conn object { _id, send, ... } or wrapper { ws: conn, ... }.
fn conn_id_from_value(v: &Value) -> Option<u32> {
    match v {
        Value::Object(o) => {
            let b = o.borrow();
            // Direct conn: { _id, send, ... }
            if let Some(Value::Number(n)) = b.strings.get("_id") {
                if n.is_finite() && *n >= 0.0 {
                    return Some(*n as u32);
                }
            }
            // Wrapper: { ws: conn, ... }
            if let Some(ws) = b.strings.get("ws") {
                return conn_id_from_value(ws);
            }
            None
        }
        _ => None,
    }
}

/// Drop conn-array entries whose connection is no longer registered (closed or disconnected).
/// Called lazily from the tish thread — at the `server.clients` push sites and before broadcasts —
/// and NEVER from the tokio read task: `VmRef` is `Rc` in single-threaded builds, so cross-thread
/// pruning would be unsound. Keeps `server.clients` O(live) instead of O(ever-accepted) and matches
/// Node `ws` semantics, where closed sockets leave `clients` (#698).
fn prune_closed_conns(list: &mut Vec<Value>) {
    let Ok(guard) = CONNS.lock() else { return };
    list.retain(|c| {
        conn_id_from_value(c)
            .map(|id| guard.contains_key(&id))
            .unwrap_or(false)
    });
}

/// Native broadcast: send data to all conns in array except `except`. Avoids Tish-side method calls.
pub fn ws_broadcast_native(args: &[Value]) -> Value {
    let conns = match args.first() {
        Some(Value::Array(a)) => {
            // Prune first so broadcast cost stays proportional to LIVE connections, not to every
            // connection ever accepted (#698). Sequential borrows — the mutable one ends before
            // the clone's shared borrow starts.
            prune_closed_conns(&mut a.borrow_mut());
            a.borrow().clone()
        }
        _ => return Value::Null,
    };
    let except = args.get(1).cloned().unwrap_or(Value::Null);
    let data = args
        .get(2)
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let mut n = 0u32;
    for c in conns {
        if c.strict_eq(&except) {
            continue;
        }
        if let Some(id) = conn_id_from_value(&c) {
            if conn_send(id, data.clone()) {
                n += 1;
            }
        }
    }
    Value::Number(n as f64)
}

/// Is a connection still live (present in the registry)? The read task calls `unregister` when the
/// stream ends, so this flips to false on close/disconnect. `receiveTimeout` returns Null for BOTH
/// an idle timeout and a closed socket, so a pump loop needs this to tell them apart (e.g. to emit
/// EOF exactly once when a remote pty / watch socket drops).
fn conn_is_open(id: u32) -> bool {
    CONNS.lock().map(|g| g.contains_key(&id)).unwrap_or(false)
}

/// Build connection object: { _id, send, close, readyState, receive, receiveTimeout, isOpen }. JS-like.
fn conn_object(id: u32) -> Value {
    let mut obj: ObjectMap = ObjectMap::default();
    obj.insert(Arc::from("_id"), Value::Number(id as f64));
    obj.insert(Arc::from("readyState"), Value::Number(1.0)); // OPEN
    obj.insert(
        Arc::from("send"),
        Value::native(move |args: &[Value]| {
            let data = args
                .first()
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            Value::Bool(conn_send(id, data))
        }),
    );
    obj.insert(
        Arc::from("isOpen"),
        Value::native(move |_args: &[Value]| Value::Bool(conn_is_open(id))),
    );
    obj.insert(
        Arc::from("close"),
        Value::native(move |_args: &[Value]| {
            unregister(id);
            Value::Null
        }),
    );
    obj.insert(
        Arc::from("receive"),
        Value::native(move |_args: &[Value]| match conn_receive(id) {
            Some(s) => {
                let mut ev: ObjectMap = ObjectMap::default();
                ev.insert(Arc::from("data"), Value::String(s.into()));
                Value::object(ev)
            }
            None => Value::Null,
        }),
    );
    let id_timeout = id;
    obj.insert(
        Arc::from("receiveTimeout"),
        Value::native(move |args: &[Value]| {
            let timeout_ms = args
                .first()
                .and_then(|v| match v {
                    Value::Number(n) if n.is_finite() && *n >= 0.0 => {
                        Some((*n as u64).min(3_600_000))
                    }
                    _ => None,
                })
                .unwrap_or(1000);
            match conn_receive_timeout(id_timeout, timeout_ms) {
                Some(s) => {
                    let mut ev: ObjectMap = ObjectMap::default();
                    ev.insert(Arc::from("data"), Value::String(s.into()));
                    Value::object(ev)
                }
                None => Value::Null,
            }
        }),
    );
    Value::object(obj)
}

fn parse_port(args: &[Value]) -> Option<u16> {
    args.first().and_then(|v| match v {
        Value::Object(o) => o.borrow().strings.get("port").and_then(|v| match v {
            Value::Number(n) if n.is_finite() && *n >= 0.0 => Some(*n as u16),
            _ => None,
        }),
        _ => None,
    })
}

/// WebSocket(url, connectTimeoutMs?) — JS-like client. Returns object with send, close, readyState,
/// receive; `Null` if the connection fails or (with a positive timeout) doesn't complete in time.
/// `wss://` is supported via rustls (the `rustls-tls-native-roots` feature on tokio-tungstenite).
pub fn web_socket_client(args: &[Value]) -> Value {
    let mut url = match args.first().map(|v| v.to_display_string()) {
        Some(u) if !u.is_empty() => u,
        _ => return Value::Null,
    };
    // Bounded connect so a dead/unreachable host can't hang the caller forever. Default 15s; pass 0
    // to wait indefinitely (matches the old behavior).
    let connect_timeout_ms = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => (*n as u64).min(3_600_000),
        _ => 15_000,
    };
    // Ensure URL has a path so the client sends "GET / ..." (avoids server responding with 200 instead of 101)
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    if !url[after_scheme..].contains('/') {
        url.push('/');
    }
    // wss:// needs rustls' process-default crypto provider installed first.
    ensure_crypto_provider();
    let (send_tx, mut send_rx) = tokio_mpsc::channel::<String>(SEND_QUEUE_MAX_MSGS);
    let (recv_tx, recv_rx) = mpsc::sync_channel::<String>(64);
    let recv_tx = Arc::new(recv_tx);
    let queued_bytes = Arc::new(AtomicUsize::new(0));

    let id = with_ws_client_rt(|rt| {
        rt.block_on(async move {
            let connect =
                tokio_tungstenite::connect_async_with_config(&url, Some(ws_config()), false);
            let connected = if connect_timeout_ms > 0 {
                match tokio::time::timeout(Duration::from_millis(connect_timeout_ms), connect).await {
                    Ok(r) => r,
                    Err(_) => {
                        if ws_debug() {
                            eprintln!("[tish ws] connect timed out after {}ms: {}", connect_timeout_ms, url);
                        }
                        return None;
                    }
                }
            } else {
                connect.await
            };
            let (ws_stream, _) = match connected {
                Ok(x) => {
                    if ws_debug() {
                        eprintln!("[tish ws] client connected (handshake OK): {}", url);
                    }
                    x
                }
                Err(e) => {
                    if ws_debug() {
                        let hint = if e.to_string().contains("200 OK") {
                            " Another process may be using the port (not the WebSocket gateway). With gateway running, run: lsof -i :<port>"
                        } else {
                            ""
                        };
                        eprintln!("[tish ws] connect_async failed: {} (url: {}){}", e, url, hint);
                    }
                    return None;
                }
            };
            let id = register(send_tx, recv_rx, Arc::clone(&queued_bytes));
            let (mut write, mut read) = ws_stream.split();
            let recv_tx = Arc::clone(&recv_tx);
            let url_closed = url.clone();
            let read_task = tokio::spawn(async move {
                while let Some(Ok(msg)) = read.next().await {
                    match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                            if recv_tx.send(t.to_string()).is_err() {
                                break;
                            }
                        }
                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                            if recv_tx.send(String::from_utf8_lossy(&b).into_owned()).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if ws_debug() {
                    eprintln!("[tish ws] client connection closed (stream ended): {}", url_closed);
                }
                unregister(id);
            });
            attach_read_task(id, &read_task);
            tokio::spawn(async move {
                while let Some(text) = send_rx.recv().await {
                    let n = text.len();
                    let _ = write
                        .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                        .await;
                    queued_bytes.fetch_sub(n, Ordering::AcqRel);
                }
                // See `register_ws_stream`: drive the closing handshake so close() frees the fd.
                let _ = write.close().await;
            });
            Some(id)
        })
    });

    let Some(id) = id else {
        return Value::Null;
    };
    conn_object(id)
}

/// Start listening; returns `Value::Number(handle)` or `Value::Null` on bind failure.
/// A background thread accepts connections and pushes connection ids on a channel.
pub fn web_socket_server_listen(args: &[Value]) -> Value {
    let port = match parse_port(args) {
        Some(p) => p,
        _ => return Value::Null,
    };

    let (bind_tx, bind_rx) = mpsc::sync_channel::<bool>(1);
    let (conn_tx, conn_rx) = mpsc::channel::<u32>();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = next_server_handle();

    {
        let mut map = SERVER_RECV.lock().unwrap();
        map.insert(
            handle,
            ServerState {
                conn_rx,
                shutdown_tx: Some(shutdown_tx),
            },
        );
    }

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(_) => {
                let _ = bind_tx.send(false);
                return;
            }
        };
        rt.block_on(async {
            let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await {
                Ok(l) => l,
                Err(_) => {
                    let _ = bind_tx.send(false);
                    return;
                }
            };
            let _ = bind_tx.send(true);
            println!("WebSocket server listening on ws://0.0.0.0:{}", port);

            loop {
                // `server.close()` fires (or drops) the shutdown signal — exit the loop so the
                // listener, this runtime and its thread are actually freed (`TcpListener::accept`
                // is cancel-safe).
                let accepted = tokio::select! {
                    _ = &mut shutdown_rx => break,
                    r = listener.accept() => r,
                };
                let (stream, _) = match accepted {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // Cap total connections — unbounded accepts would exhaust tasks/memory.
                if !has_ws_capacity() {
                    drop(stream);
                    continue;
                }
                let ws_stream =
                    match tokio_tungstenite::accept_async_with_config(stream, Some(ws_config()))
                        .await
                    {
                        Ok(ws) => {
                            if ws_debug() {
                                eprintln!(
                                    "[tish ws] server accepted connection (handshake OK): port {}",
                                    port
                                );
                            }
                            ws
                        }
                        Err(e) => {
                            if ws_debug() {
                                eprintln!(
                                    "[tish ws] server accept_async failed: {} (port {})",
                                    e, port
                                );
                            }
                            continue;
                        }
                    };
                let id = register_ws_stream(ws_stream);
                if conn_tx.send(id).is_err() {
                    break;
                }
            }
        });
    });

    match bind_rx.recv() {
        Ok(true) => Value::Number(handle as f64),
        _ => {
            SERVER_RECV.lock().unwrap().remove(&handle);
            Value::Null
        }
    }
}

/// Block until the next connection for this server handle; returns connection object or `Null`.
pub fn web_socket_server_accept(args: &[Value]) -> Value {
    let handle = match args.first() {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u32,
        _ => return Value::Null,
    };
    let mut map = match SERVER_RECV.lock() {
        Ok(g) => g,
        Err(_) => return Value::Null,
    };
    let st = match map.get_mut(&handle) {
        Some(s) => s,
        None => return Value::Null,
    };
    match st.conn_rx.recv() {
        Ok(id) => conn_object(id),
        Err(_) => Value::Null,
    }
}

/// Like accept but with timeout (ms). Returns connection object or `Null` if no connection in time.
pub fn web_socket_server_accept_timeout(args: &[Value]) -> Value {
    let handle = match args.first() {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as u32,
        _ => return Value::Null,
    };
    let timeout_ms = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => (*n as u64).min(3_600_000),
        _ => 100,
    };
    let mut map = match SERVER_RECV.lock() {
        Ok(g) => g,
        Err(_) => return Value::Null,
    };
    let st = match map.get_mut(&handle) {
        Some(s) => s,
        None => return Value::Null,
    };
    match st
        .conn_rx
        .recv_timeout(std::time::Duration::from_millis(timeout_ms))
    {
        Ok(id) => conn_object(id),
        Err(_) => Value::Null,
    }
}

/// Tear down a listening server: stop the accept loop (freeing the `TcpListener`, its dedicated
/// thread and tokio runtime — previously they lived for the whole process, #698), and unregister
/// any accepted-but-unclaimed connections still sitting in the queue.
fn server_close(handle: u32) {
    let state = SERVER_RECV.lock().ok().and_then(|mut m| m.remove(&handle));
    if let Some(mut st) = state {
        if let Some(tx) = st.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Connections the accept loop registered but the program never picked up would otherwise
        // stay in CONNS (holding their tasks + fd) until the remote peer disconnects.
        while let Ok(id) = st.conn_rx.try_recv() {
            unregister(id);
        }
    }
}

/// `Server(options)` — object with `_handle`, `_onConnection`, `on`, `listen`, `clients` (Node.js-compatible).
pub fn web_socket_server_construct(args: &[Value]) -> Value {
    let handle_val = web_socket_server_listen(args);
    if matches!(handle_val, Value::Null) {
        return Value::Null;
    }

    // Node.js-compatible: server.clients is array of connected WebSocket instances
    let clients: VmRef<Vec<Value>> = VmRef::new(Vec::new());

    let on_fn = Value::native(|args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        let event = args
            .get(1)
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let cb = args.get(2).cloned().unwrap_or(Value::Null);
        if event == "connection" {
            so.borrow_mut()
                .strings
                .insert(Arc::from("_onConnection"), cb);
        }
        Value::Null
    });

    let clients_listen = clients.clone();
    let listen_fn = Value::native(move |args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        loop {
            let handle_n = {
                let b = so.borrow();
                match b
                    .strings
                    .get("_handle")
                    .cloned()
                    .unwrap_or(Value::Null)
                {
                    Value::Number(n) if n.is_finite() && n >= 0.0 => n,
                    _ => break,
                }
            };
            let cb = so
                .borrow()
                .strings
                .get("_onConnection")
                .cloned()
                .unwrap_or(Value::Null);
            let ws = web_socket_server_accept(&[Value::Number(handle_n)]);
            if matches!(ws, Value::Null) {
                break;
            }
            {
                // Lazy prune on the tish thread: closed conns leave `clients` before each push, so
                // the array tracks LIVE connections (Node `ws` semantics) instead of growing with
                // every connection ever accepted (#698).
                let mut list = clients_listen.borrow_mut();
                prune_closed_conns(&mut list);
                list.push(ws.clone());
            }
            if let Value::Function(f) = cb {
                let _ = f.call(&[ws]);
            }
        }
        Value::Null
    });

    let clients_accept = clients.clone();
    let accept_timeout_fn = Value::native(move |args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        let handle_n = so
            .borrow()
            .strings
            .get("_handle")
            .cloned()
            .unwrap_or(Value::Null);
        let timeout_ms = args.get(1).cloned().unwrap_or(Value::Number(100.0));
        let ws = web_socket_server_accept_timeout(&[handle_n, timeout_ms]);
        if !matches!(ws, Value::Null) {
            // Same lazy prune as the listen() push site (#698).
            let mut list = clients_accept.borrow_mut();
            prune_closed_conns(&mut list);
            list.push(ws.clone());
        }
        ws
    });

    let handle_u32 = match &handle_val {
        Value::Number(n) if n.is_finite() && *n >= 0.0 => *n as u32,
        _ => 0,
    };
    let clients_close = clients.clone();
    let close_fn = Value::native(move |args: &[Value]| {
        server_close(handle_u32);
        // Null the handle so a listen() loop exits on its next iteration, and clear `clients`
        // (the accept loop is stopped, so nothing repopulates it).
        if let Some(Value::Object(so)) = args.first() {
            so.borrow_mut()
                .strings
                .insert(Arc::from("_handle"), Value::Null);
        }
        clients_close.borrow_mut().clear();
        Value::Null
    });

    let mut m: ObjectMap = ObjectMap::default();
    m.insert(Arc::from("_handle"), handle_val);
    m.insert(Arc::from("_onConnection"), Value::Null);
    m.insert(Arc::from("clients"), Value::Array(clients));
    m.insert(Arc::from("on"), on_fn);
    m.insert(Arc::from("listen"), listen_fn);
    m.insert(Arc::from("acceptTimeout"), accept_timeout_fn);
    m.insert(Arc::from("close"), close_fn);
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    /// The serve() HTTP→WS upgrade queue: `wsAccept` hands out enqueued conns FIFO and returns Null
    /// when empty. (End-to-end upgrade over a live hyper port is covered by the native smoke test.)
    #[test]
    fn upgrade_queue_drains_then_empty() {
        assert!(matches!(ws_serve_accept(&[Value::Number(0.0)]), Value::Null));
        enqueue_upgraded_conn(4242);
        match ws_serve_accept(&[Value::Number(0.0)]) {
            Value::Object(o) => {
                let b = o.borrow();
                assert!(matches!(b.strings.get("_id"), Some(Value::Number(n)) if *n == 4242.0));
            }
            other => panic!("expected a conn object, got {:?}", other),
        }
        assert!(matches!(ws_serve_accept(&[Value::Number(0.0)]), Value::Null));
    }

    /// The client must surface Binary frames too (remote pty streams raw bytes; some servers frame
    /// output as Binary). A raw tungstenite server sends one Binary frame; the tish client receives
    /// it (utf8-lossy) via receiveTimeout.
    #[test]
    fn ws_client_receives_binary() {
        let port: u16 = 18_744;
        let server = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                    .await
                    .unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let _ = ws
                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                        b"binhello".to_vec().into(),
                    ))
                    .await;
                // Hold the connection open briefly so the client can drain the frame.
                tokio::time::sleep(Duration::from_millis(300)).await;
            });
        });

        thread::sleep(Duration::from_millis(100));
        let url = format!("ws://127.0.0.1:{}/", port);
        let client = web_socket_client(&[Value::String(url.into())]);
        let Value::Object(co) = client else {
            panic!("client connect failed");
        };
        let Some(Value::Function(recv_f)) = co.borrow().strings.get("receiveTimeout").cloned() else {
            panic!("no receiveTimeout");
        };
        let got = recv_f.call(&[Value::Number(2000.0)]);
        let Value::Object(ev) = got else {
            panic!("expected a binary message, got {:?}", got);
        };
        let data = ev
            .borrow()
            .strings
            .get("data")
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        assert_eq!(data, "binhello");
        let _ = server.join();
    }

    /// Proves `wss://` is wired to a TLS connector (rustls). Before a TLS feature was enabled,
    /// connect_async rejected wss at the scheme layer (TlsFeatureNotEnabled) WITHOUT dialing. Now it
    /// dials TCP then attempts a TLS handshake. Against a plain-TCP listener (no TLS server) the
    /// handshake can't complete, so the client returns Null — but the listener MUST observe an
    /// inbound connection, which is only possible if the client got past scheme rejection into a real
    /// dial. (A live wss happy-path smoke needs network egress / a trusted cert — Part 5 interactive.)
    #[test]
    fn ws_client_wss_attempts_tls_handshake() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<bool>();
        let server = thread::spawn(move || {
            if listener.accept().is_ok() {
                let _ = tx.send(true);
            }
        });
        let client = web_socket_client(&[
            Value::String(format!("wss://127.0.0.1:{}/", port).into()),
            Value::Number(3000.0),
        ]);
        assert!(
            matches!(client, Value::Null),
            "a plain-TCP server can't complete a TLS handshake → expected Null"
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)),
            Ok(true),
            "client must dial TCP for a wss URL (proves the TLS connector is wired)"
        );
        let _ = server.join();
    }

    /// isOpen() must reflect the registry: true while registered, false once the read task
    /// unregisters on close (a pump loop relies on this to tell an idle receiveTimeout from a closed
    /// socket and emit EOF exactly once).
    #[test]
    fn conn_is_open_reflects_registry() {
        let (tx, _rx) = tokio_mpsc::channel::<String>(4);
        let (_stx, srx) = mpsc::sync_channel::<String>(1);
        let id = register(tx, srx, Arc::new(AtomicUsize::new(0)));
        assert!(conn_is_open(id), "just-registered conn should be open");
        unregister(id);
        assert!(!conn_is_open(id), "unregistered conn should be closed");
    }

    /// The bounded send queue must refuse (return false) once the byte budget is reached instead of
    /// queueing forever — the backpressure contract callers rely on (`sent === false` → drop the
    /// peer). Nothing drains the channel here, simulating a peer that stopped reading.
    #[test]
    fn conn_send_refuses_when_queue_full() {
        let (tx, _rx) = tokio_mpsc::channel::<String>(SEND_QUEUE_MAX_MSGS);
        let (_stx, srx) = mpsc::sync_channel::<String>(1);
        let id = register(tx, srx, Arc::new(AtomicUsize::new(0)));

        let chunk = "x".repeat(64 * 1024);
        let budget = ws_send_buf_bytes();
        let mut accepted = 0usize;
        while conn_send(id, chunk.clone()) {
            accepted += 1;
            assert!(
                accepted * chunk.len() <= budget + chunk.len(),
                "queue admitted {} bytes — budget {} never enforced",
                accepted * chunk.len(),
                budget
            );
        }
        assert!(
            accepted > 0,
            "an empty queue must admit at least one message"
        );
        assert!(
            (accepted + 1) * chunk.len() > budget,
            "refused after only {} bytes (budget {})",
            accepted * chunk.len(),
            budget
        );
        // Once full, even a tiny send is refused (only an EMPTY queue admits unconditionally).
        assert!(
            !conn_send(id, "y".into()),
            "full queue must refuse further sends"
        );
        unregister(id);
        assert!(!conn_send(id, "z".into()), "closed conn must refuse sends");
    }

    /// `server.clients` housekeeping: `prune_closed_conns` drops entries whose connection is no
    /// longer registered (and non-conn junk), keeping only live conns — Node `ws` semantics (#698).
    #[test]
    fn prune_closed_conns_drops_dead_entries() {
        let (tx, _rx) = tokio_mpsc::channel::<String>(4);
        let (_stx, srx) = mpsc::sync_channel::<String>(1);
        let live = register(tx, srx, Arc::new(AtomicUsize::new(0)));
        let (tx2, _rx2) = tokio_mpsc::channel::<String>(4);
        let (_stx2, srx2) = mpsc::sync_channel::<String>(1);
        let dead = register(tx2, srx2, Arc::new(AtomicUsize::new(0)));
        unregister(dead);

        let mut list = vec![conn_object(live), conn_object(dead), Value::Null];
        prune_closed_conns(&mut list);
        assert_eq!(list.len(), 1, "only the live conn should remain");
        assert_eq!(conn_id_from_value(&list[0]), Some(live));
        unregister(live);
    }

    /// `receiveTimeout` on a dead/unknown id must honor the caller's timeout — it used to sleep a
    /// flat 50ms, so a pump loop polling N zombie conns paid N x 50ms per tick.
    #[test]
    fn receive_timeout_dead_id_honors_caller_timeout() {
        let t0 = Instant::now();
        assert!(conn_receive_timeout(u32::MAX, 0).is_none());
        assert!(
            t0.elapsed() < Duration::from_millis(40),
            "receiveTimeout(0) on a dead id must return immediately, took {:?}",
            t0.elapsed()
        );
    }

    /// close() must actually close the socket: the SERVER side must observe the stream ending and
    /// unregister its conn. Before the fix, close() only removed the local registry entry — the
    /// read task stayed parked on the open socket forever (fd + task + buffers leaked per close).
    #[test]
    fn client_close_shuts_down_the_socket() {
        let port: u16 = 18_745;
        let opts = {
            let mut m: ObjectMap = ObjectMap::default();
            m.insert(Arc::from("port"), Value::Number(port as f64));
            Value::object(m)
        };
        let handle = match web_socket_server_listen(std::slice::from_ref(&opts)) {
            Value::Number(h) => h as u32,
            _ => panic!("listen failed"),
        };
        let (id_tx, id_rx) = mpsc::channel::<u32>();
        let server = thread::spawn(move || {
            let ws = web_socket_server_accept(&[Value::Number(handle as f64)]);
            let _ = id_tx.send(conn_id_from_value(&ws).expect("server conn id"));
        });

        thread::sleep(Duration::from_millis(100));
        let url = format!("ws://127.0.0.1:{}/", port);
        let client = web_socket_client(&[Value::String(url.into())]);
        let Value::Object(co) = client else {
            panic!("client connect failed");
        };
        let server_id = id_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server never accepted");
        assert!(conn_is_open(server_id), "server conn should start open");

        let Some(Value::Function(close_f)) = co.borrow().strings.get("close").cloned() else {
            panic!("no close");
        };
        let _ = close_f.call(&[]);

        let deadline = Instant::now() + Duration::from_secs(5);
        while conn_is_open(server_id) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !conn_is_open(server_id),
            "server still sees the conn open after the client's close() — the socket was not shut down"
        );
        let _ = server.join();
    }

    /// server.close() must stop the accept loop and free the TcpListener (previously a bound
    /// server's listener, thread and runtime lived for the whole process — #698).
    #[test]
    fn server_close_frees_the_listener() {
        let port: u16 = 18_746;
        let opts = {
            let mut m: ObjectMap = ObjectMap::default();
            m.insert(Arc::from("port"), Value::Number(port as f64));
            Value::object(m)
        };
        let srv = web_socket_server_construct(std::slice::from_ref(&opts));
        let Value::Object(so) = &srv else {
            panic!("server construct failed");
        };
        let Some(Value::Function(close_f)) = so.borrow().strings.get("close").cloned() else {
            panic!("no close method on server object");
        };
        let _ = close_f.call(std::slice::from_ref(&srv));

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut rebound = false;
        while Instant::now() < deadline {
            if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
                rebound = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(rebound, "port still held after server.close()");
        // And the handle is gone: a subsequent accept returns Null instead of blocking.
        assert!(matches!(
            web_socket_server_accept_timeout(&[Value::Number(0.0), Value::Number(0.0)]),
            Value::Null
        ));
    }

    #[test]
    fn ws_echo_roundtrip() {
        let port: u16 = 18_742;
        let opts = {
            let mut m: ObjectMap = ObjectMap::default();
            m.insert(Arc::from("port"), Value::Number(port as f64));
            Value::object(m)
        };

        let handle = match web_socket_server_listen(std::slice::from_ref(&opts)) {
            Value::Number(h) => h as u32,
            _ => panic!("listen failed"),
        };

        let server = thread::spawn(move || {
            let ws = web_socket_server_accept(&[Value::Number(handle as f64)]);
            let Value::Object(wso) = ws else {
                panic!("accept failed");
            };
            // Echo one message
            for _ in 0..50 {
                let recv_fn = wso.borrow().strings.get("receive").cloned();
                if let Some(Value::Function(rf)) = recv_fn {
                    let msg = rf.call(&[]);
                    if !matches!(msg, Value::Null) {
                        let data = match msg {
                            Value::Object(ev) => ev
                                .borrow()
                                .strings
                                .get("data")
                                .map(|v| v.to_display_string())
                                .unwrap_or_default(),
                            _ => String::new(),
                        };
                        if let Some(Value::Function(sf)) =
                            wso.borrow().strings.get("send").cloned()
                        {
                            let _ = sf.call(&[Value::String(data.into())]);
                        }
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        });

        thread::sleep(Duration::from_millis(100));
        let url = format!("ws://127.0.0.1:{}", port);
        let client = web_socket_client(&[Value::String(url.into())]);
        assert!(!matches!(client, Value::Null), "client connect failed");

        let Value::Object(co) = client else {
            panic!("client not object");
        };
        let send = co.borrow().strings.get("send").cloned().unwrap();
        let Value::Function(send_f) = send else {
            panic!("no send");
        };
        let _ = send_f.call(&[Value::String("hello".into())]);

        let recv = co.borrow().strings.get("receive").cloned().unwrap();
        let Value::Function(recv_f) = recv else {
            panic!("no receive");
        };
        let mut got = Value::Null;
        for _ in 0..100 {
            got = recv_f.call(&[]);
            if !matches!(got, Value::Null) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let Value::Object(ev) = got else {
            panic!("expected message object");
        };
        let data = ev
            .borrow()
            .strings
            .get("data")
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        assert_eq!(data, "hello");

        let _ = server.join();
    }

    /// Gateway→agent flow: server receives "join", sends "joined" + "presence"; client must receive both via receiveTimeout.
    #[test]
    fn ws_gateway_agent_flow() {
        let port: u16 = 18_743;
        let opts = {
            let mut m: ObjectMap = ObjectMap::default();
            m.insert(Arc::from("port"), Value::Number(port as f64));
            Value::object(m)
        };

        let handle = match web_socket_server_listen(std::slice::from_ref(&opts)) {
            Value::Number(h) => h as u32,
            _ => panic!("listen failed"),
        };

        let server = thread::spawn(move || {
            let ws = web_socket_server_accept(&[Value::Number(handle as f64)]);
            let Value::Object(wso) = ws else {
                panic!("accept failed");
            };
            let recv_fn = wso.borrow().strings.get("receive").cloned();
            let Value::Function(rf) = recv_fn.unwrap() else {
                panic!("no receive");
            };
            // Poll until we get join
            for _ in 0..200 {
                let msg = rf.call(&[]);
                if !matches!(msg, Value::Null) {
                    let data = match &msg {
                        Value::Object(ev) => ev
                            .borrow()
                            .strings
                            .get("data")
                            .map(|v| v.to_display_string())
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    if data.contains("\"type\":\"join\"") || data.contains("\"type\": \"join\"") {
                        let joined = r#"{"type":"joined","sessionId":"default"}"#;
                        let presence = r#"{"type":"presence","agentLanes":["ai-a"]}"#;
                        ws_send_native(&Value::Object(wso.clone()), joined);
                        ws_send_native(&Value::Object(wso.clone()), presence);
                        return;
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("server never got join");
        });

        thread::sleep(Duration::from_millis(100));
        let url = format!("ws://127.0.0.1:{}/", port);
        let client = web_socket_client(&[Value::String(url.into())]);
        assert!(!matches!(client, Value::Null), "client connect failed");

        let Value::Object(co) = client else {
            panic!("client not object");
        };
        let send = co.borrow().strings.get("send").cloned().unwrap();
        let Value::Function(send_f) = send else {
            panic!("no send");
        };
        let join_msg = r#"{"type":"join","sessionId":"default","role":"agent","laneId":"ai-a"}"#;
        let _ = send_f.call(&[Value::String(join_msg.into())]);

        // Client uses receiveTimeout like the agent
        let recv_timeout = co
            .borrow()
            .strings
            .get("receiveTimeout")
            .cloned()
            .unwrap();
        let Value::Function(recv_timeout_f) = recv_timeout else {
            panic!("no receiveTimeout");
        };
        let timeout_arg = Value::Number(2000.0);

        let got1 = recv_timeout_f.call(&[timeout_arg.clone()]);
        let Value::Object(ev1) = got1 else {
            panic!("first recv: expected object, got {:?}", got1);
        };
        let data1 = ev1
            .borrow()
            .strings
            .get("data")
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        assert!(
            data1.contains("\"type\":\"joined\""),
            "expected joined, got {}",
            data1
        );

        let got2 = recv_timeout_f.call(&[timeout_arg]);
        let Value::Object(ev2) = got2 else {
            panic!("second recv: expected object, got {:?}", got2);
        };
        let data2 = ev2
            .borrow()
            .strings
            .get("data")
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        assert!(
            data2.contains("\"type\":\"presence\""),
            "expected presence, got {}",
            data2
        );

        let _ = server.join();
    }
}
