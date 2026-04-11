//! WebSocket module for Tish (tish:ws).
//!
//! Node.js `ws`-compatible API:
//! - **Server**: `Server({ port })` — has `clients` (array), `on('connection', fn)`, `listen()`, `acceptTimeout(server, ms)`
//! - **Connection**: `send(data)`, `close()`, `readyState` (1=OPEN), `receive()` / `receiveTimeout(ms)`
//! - **Broadcast** (Node pattern): `server.clients.forEach(ws => ws.send(data))` or iterate room conns and `wsSend(ws, data)` (same as `ws.send(data)`)

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
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

struct ConnState {
    send_tx: tokio_mpsc::UnboundedSender<String>,
    recv_rx: mpsc::Receiver<String>,
    #[allow(dead_code)]
    open: bool,
}

lazy_static! {
    static ref CONNS: Mutex<HashMap<u32, ConnState>> = Mutex::new(HashMap::new());
    static ref SERVER_RECV: Mutex<HashMap<u32, mpsc::Receiver<u32>>> = Mutex::new(HashMap::new());
}

fn register(send_tx: tokio_mpsc::UnboundedSender<String>, recv_rx: mpsc::Receiver<String>) -> u32 {
    let id = next_conn_id();
    CONNS.lock().unwrap().insert(
        id,
        ConnState {
            send_tx,
            recv_rx,
            open: true,
        },
    );
    id
}

fn unregister(id: u32) {
    CONNS.lock().unwrap().remove(&id);
}

fn conn_send(id: u32, data: String) -> bool {
    let guard = match CONNS.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let state = match guard.get(&id) {
        Some(s) if s.open => s,
        _ => return false,
    };
    state.send_tx.send(data).is_ok()
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
    let timeout_ms = timeout_ms.min(3600_000);
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
                std::thread::sleep(Duration::from_millis(50));
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
            if let Some(idv) = b.get(&Arc::from("_id")) {
                if let Value::Number(n) = idv {
                    if n.is_finite() && *n >= 0.0 {
                        return Some(*n as u32);
                    }
                }
            }
            // Wrapper: { ws: conn, ... }
            if let Some(ws) = b.get(&Arc::from("ws")) {
                return conn_id_from_value(ws);
            }
            None
        }
        _ => None,
    }
}

/// Native broadcast: send data to all conns in array except `except`. Avoids Tish-side method calls.
pub fn ws_broadcast_native(args: &[Value]) -> Value {
    let conns = match args.get(0) {
        Some(Value::Array(a)) => a.borrow().clone(),
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

/// Build connection object: { _id, send, close, readyState, receive }. JS-like.
fn conn_object(id: u32) -> Value {
    let mut obj: ObjectMap = ObjectMap::default();
    obj.insert(Arc::from("_id"), Value::Number(id as f64));
    obj.insert(Arc::from("readyState"), Value::Number(1.0)); // OPEN
    obj.insert(
        Arc::from("send"),
        Value::Function(Rc::new(move |args: &[Value]| {
            let data = args
                .first()
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            Value::Bool(conn_send(id, data))
        })),
    );
    obj.insert(
        Arc::from("close"),
        Value::Function(Rc::new(move |_args: &[Value]| {
            unregister(id);
            Value::Null
        })),
    );
    obj.insert(
        Arc::from("receive"),
        Value::Function(Rc::new(move |_args: &[Value]| match conn_receive(id) {
            Some(s) => {
                let mut ev: ObjectMap = ObjectMap::default();
                ev.insert(Arc::from("data"), Value::String(s.into()));
                Value::Object(Rc::new(RefCell::new(ev)))
            }
            None => Value::Null,
        })),
    );
    let id_timeout = id;
    obj.insert(
        Arc::from("receiveTimeout"),
        Value::Function(Rc::new(move |args: &[Value]| {
            let timeout_ms = args
                .first()
                .and_then(|v| match v {
                    Value::Number(n) if n.is_finite() && *n >= 0.0 => {
                        Some((*n as u64).min(3600_000))
                    }
                    _ => None,
                })
                .unwrap_or(1000);
            match conn_receive_timeout(id_timeout, timeout_ms) {
                Some(s) => {
                    let mut ev: ObjectMap = ObjectMap::default();
                    ev.insert(Arc::from("data"), Value::String(s.into()));
                    Value::Object(Rc::new(RefCell::new(ev)))
                }
                None => Value::Null,
            }
        })),
    );
    Value::Object(Rc::new(RefCell::new(obj)))
}

fn parse_port(args: &[Value]) -> Option<u16> {
    args.first().and_then(|v| match v {
        Value::Object(o) => o.borrow().get(&Arc::from("port")).and_then(|v| match v {
            Value::Number(n) if n.is_finite() && *n >= 0.0 => Some(*n as u16),
            _ => None,
        }),
        _ => None,
    })
}

/// WebSocket(url) — JS-like client. Returns object with send, close, readyState, receive.
pub fn web_socket_client(args: &[Value]) -> Value {
    let mut url = match args.first().map(|v| v.to_display_string()) {
        Some(u) if !u.is_empty() => u,
        _ => return Value::Null,
    };
    // Ensure URL has a path so the client sends "GET / ..." (avoids server responding with 200 instead of 101)
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    if !url[after_scheme..].contains('/') {
        url.push('/');
    }
    let (send_tx, mut send_rx) = tokio_mpsc::unbounded_channel::<String>();
    let (recv_tx, recv_rx) = mpsc::sync_channel::<String>(64);
    let recv_tx = Arc::new(recv_tx);

    let id = with_ws_client_rt(|rt| {
        rt.block_on(async move {
            let (ws_stream, _) = match tokio_tungstenite::connect_async(&url).await {
                Ok(x) => {
                    eprintln!("[tish ws] client connected (handshake OK): {}", url);
                    x
                }
                Err(e) => {
                    let hint = if e.to_string().contains("200 OK") {
                        " Another process may be using the port (not the WebSocket gateway). With gateway running, run: lsof -i :<port>"
                    } else {
                        ""
                    };
                    eprintln!("[tish ws] connect_async failed: {} (url: {}){}", e, url, hint);
                    return None;
                }
            };
            let id = register(send_tx, recv_rx);
            let (mut write, mut read) = ws_stream.split();
            let recv_tx = Arc::clone(&recv_tx);
            let url_closed = url.clone();
            tokio::spawn(async move {
                while let Some(Ok(msg)) = read.next().await {
                    if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
                        let _ = recv_tx.send(t.to_string());
                    }
                }
                eprintln!("[tish ws] client connection closed (stream ended): {}", url_closed);
                unregister(id);
            });
            tokio::spawn(async move {
                while let Some(text) = send_rx.recv().await {
                    let _ = write
                        .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                        .await;
                }
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
    let handle = next_server_handle();

    {
        let mut map = SERVER_RECV.lock().unwrap();
        map.insert(handle, conn_rx);
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
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let ws_stream = match tokio_tungstenite::accept_async(stream).await {
                    Ok(ws) => {
                        eprintln!(
                            "[tish ws] server accepted connection (handshake OK): port {}",
                            port
                        );
                        ws
                    }
                    Err(e) => {
                        eprintln!(
                            "[tish ws] server accept_async failed: {} (port {})",
                            e, port
                        );
                        continue;
                    }
                };
                let (send_tx, mut send_rx) = tokio_mpsc::unbounded_channel::<String>();
                let (recv_tx, recv_rx) = mpsc::sync_channel::<String>(64);
                let id = register(send_tx, recv_rx);
                let recv_tx = Arc::new(recv_tx);
                let recv_tx_task = Arc::clone(&recv_tx);
                let (mut write, mut read) = ws_stream.split();
                tokio::spawn(async move {
                    while let Some(Ok(msg)) = read.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
                            let _ = recv_tx_task.send(t.to_string());
                        }
                    }
                    unregister(id);
                });
                tokio::spawn(async move {
                    while let Some(text) = send_rx.recv().await {
                        let _ = write
                            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                            .await;
                    }
                });
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
    let rx = match map.get_mut(&handle) {
        Some(r) => r,
        None => return Value::Null,
    };
    match rx.recv() {
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
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => (*n as u64).min(3600_000),
        _ => 100,
    };
    let mut map = match SERVER_RECV.lock() {
        Ok(g) => g,
        Err(_) => return Value::Null,
    };
    let rx = match map.get_mut(&handle) {
        Some(r) => r,
        None => return Value::Null,
    };
    match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
        Ok(id) => conn_object(id),
        Err(_) => Value::Null,
    }
}

/// `Server(options)` — object with `_handle`, `_onConnection`, `on`, `listen`, `clients` (Node.js-compatible).
pub fn web_socket_server_construct(args: &[Value]) -> Value {
    let handle_val = web_socket_server_listen(args);
    if matches!(handle_val, Value::Null) {
        return Value::Null;
    }

    // Node.js-compatible: server.clients is array of connected WebSocket instances
    let clients: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));

    let on_fn = Rc::new(|args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        let event = args
            .get(1)
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let cb = args.get(2).cloned().unwrap_or(Value::Null);
        if event == "connection" {
            so.borrow_mut().insert(Arc::from("_onConnection"), cb);
        }
        Value::Null
    });

    let clients_listen = Rc::clone(&clients);
    let listen_fn = Rc::new(move |args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        loop {
            let handle_n = {
                let b = so.borrow();
                match b.get(&Arc::from("_handle")).cloned().unwrap_or(Value::Null) {
                    Value::Number(n) if n.is_finite() && n >= 0.0 => n,
                    _ => break,
                }
            };
            let cb = so
                .borrow()
                .get(&Arc::from("_onConnection"))
                .cloned()
                .unwrap_or(Value::Null);
            let ws = web_socket_server_accept(&[Value::Number(handle_n)]);
            if matches!(ws, Value::Null) {
                break;
            }
            clients_listen.borrow_mut().push(ws.clone());
            if let Value::Function(f) = cb {
                let _ = f(&[ws]);
            }
        }
        Value::Null
    });

    let clients_accept = Rc::clone(&clients);
    let accept_timeout_fn = Rc::new(move |args: &[Value]| {
        let Some(Value::Object(so)) = args.first() else {
            return Value::Null;
        };
        let handle_n = so
            .borrow()
            .get(&Arc::from("_handle"))
            .cloned()
            .unwrap_or(Value::Null);
        let timeout_ms = args.get(1).cloned().unwrap_or(Value::Number(100.0));
        let ws = web_socket_server_accept_timeout(&[handle_n, timeout_ms]);
        if !matches!(ws, Value::Null) {
            clients_accept.borrow_mut().push(ws.clone());
        }
        ws
    });

    let mut m: ObjectMap = ObjectMap::default();
    m.insert(Arc::from("_handle"), handle_val);
    m.insert(Arc::from("_onConnection"), Value::Null);
    m.insert(Arc::from("clients"), Value::Array(clients));
    m.insert(Arc::from("on"), Value::Function(on_fn));
    m.insert(Arc::from("listen"), Value::Function(listen_fn));
    m.insert(
        Arc::from("acceptTimeout"),
        Value::Function(accept_timeout_fn),
    );
    Value::Object(Rc::new(RefCell::new(m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn ws_echo_roundtrip() {
        let port: u16 = 18_742;
        let opts = {
            let mut m: ObjectMap = ObjectMap::default();
            m.insert(Arc::from("port"), Value::Number(port as f64));
            Value::Object(Rc::new(RefCell::new(m)))
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
                let recv_fn = wso.borrow().get(&Arc::from("receive")).cloned();
                if let Some(Value::Function(rf)) = recv_fn {
                    let msg = rf(&[]);
                    if !matches!(msg, Value::Null) {
                        let data = match msg {
                            Value::Object(ev) => ev
                                .borrow()
                                .get(&Arc::from("data"))
                                .map(|v| v.to_display_string())
                                .unwrap_or_default(),
                            _ => String::new(),
                        };
                        if let Some(Value::Function(sf)) =
                            wso.borrow().get(&Arc::from("send")).cloned()
                        {
                            let _ = sf(&[Value::String(data.into())]);
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
        let send = co.borrow().get(&Arc::from("send")).cloned().unwrap();
        let Value::Function(send_f) = send else {
            panic!("no send");
        };
        let _ = send_f(&[Value::String("hello".into())]);

        let recv = co.borrow().get(&Arc::from("receive")).cloned().unwrap();
        let Value::Function(recv_f) = recv else {
            panic!("no receive");
        };
        let mut got = Value::Null;
        for _ in 0..100 {
            got = recv_f(&[]);
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
            .get(&Arc::from("data"))
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
            Value::Object(Rc::new(RefCell::new(m)))
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
            let recv_fn = wso.borrow().get(&Arc::from("receive")).cloned();
            let Value::Function(rf) = recv_fn.unwrap() else {
                panic!("no receive");
            };
            // Poll until we get join
            for _ in 0..200 {
                let msg = rf(&[]);
                if !matches!(msg, Value::Null) {
                    let data = match &msg {
                        Value::Object(ev) => ev
                            .borrow()
                            .get(&Arc::from("data"))
                            .map(|v| v.to_display_string())
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    if data.contains("\"type\":\"join\"") || data.contains("\"type\": \"join\"") {
                        let joined = r#"{"type":"joined","sessionId":"default"}"#;
                        let presence = r#"{"type":"presence","agentLanes":["ai-a"]}"#;
                        ws_send_native(&Value::Object(Rc::clone(&wso)), joined);
                        ws_send_native(&Value::Object(Rc::clone(&wso)), presence);
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
        let send = co.borrow().get(&Arc::from("send")).cloned().unwrap();
        let Value::Function(send_f) = send else {
            panic!("no send");
        };
        let join_msg = r#"{"type":"join","sessionId":"default","role":"agent","laneId":"ai-a"}"#;
        let _ = send_f(&[Value::String(join_msg.into())]);

        // Client uses receiveTimeout like the agent
        let recv_timeout = co
            .borrow()
            .get(&Arc::from("receiveTimeout"))
            .cloned()
            .unwrap();
        let Value::Function(recv_timeout_f) = recv_timeout else {
            panic!("no receiveTimeout");
        };
        let timeout_arg = Value::Number(2000.0);

        let got1 = recv_timeout_f(&[timeout_arg.clone()]);
        let Value::Object(ev1) = got1 else {
            panic!("first recv: expected object, got {:?}", got1);
        };
        let data1 = ev1
            .borrow()
            .get(&Arc::from("data"))
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        assert!(
            data1.contains("\"type\":\"joined\""),
            "expected joined, got {}",
            data1
        );

        let got2 = recv_timeout_f(&[timeout_arg]);
        let Value::Object(ev2) = got2 else {
            panic!("second recv: expected object, got {:?}", got2);
        };
        let data2 = ev2
            .borrow()
            .get(&Arc::from("data"))
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
