//! `hyper`-based HTTP backend for `tish:http`.
//!
//! Selectable at runtime via `TISH_HTTP_BACKEND=hyper` when the
//! `http-hyper` feature is compiled in. When unset, `serve` falls back to
//! the `tiny_http` path in [`crate::http`].
//!
//! ## Architecture
//!
//! ```text
//!   N OS threads (one per CPU, pinned via core_affinity)
//!    ├─ single-threaded tokio runtime per thread
//!    ├─ SO_REUSEPORT-bound TcpListener per thread
//!    └─ hyper HTTP/1.1 + HTTP/2 server
//!          │
//!          ├─ async per-connection state machine (no OS thread)
//!          ├─ static-route fast path (lock-free ArcSwap<HashMap> from
//!          │    [`crate::http::register_static_route`])
//!          └─ VM-dispatch slow path: crosses mpsc to VM thread, awaits
//!               oneshot reply, writes response
//! ```
//!
//! ## Why this is broadly useful (beyond the bench)
//!
//! * Removes tiny_http's thread-per-connection model (the bottleneck on
//!   every macOS/Linux Tish HTTP server at >50k concurrent connections).
//! * Gives HTTP/2 + TLS-ready surface for free (hyper handles the state
//!   machine; we only have to convert our `RequestPrimitive` to/from
//!   `http::Request` / `http::Response`).
//! * Shared async context with `reqwest`, `tokio-postgres`, `tokio` in
//!   general — the tokio runtime is reused for client fetch, db, and
//!   server accept.
//!
//! ## Integration with the Tish VM
//!
//! The Tish handler returns a synchronous `Value`. We call it from inside
//! a tokio task via `tokio::task::spawn_blocking`, which:
//!   * detaches onto tokio's blocking thread pool,
//!   * lets `tish-pg`'s `block_on` detect no ambient runtime and block
//!     directly (no extra thread spawn per query),
//!   * unblocks hyper's reactor to serve other connections while the VM
//!     runs.
//!
//! Multi-VM (one Tish VM per worker thread) is the **next** step layered
//! on top of this file — see [`WorkerHandler`] below and the `onWorker`
//! semantics in [`crate::http::serve`]. For now the VM stays single-threaded
//! and we share it via the mpsc-dispatch pattern; adding per-core VMs is
//! a drop-in replacement of `dispatch_to_vm` with a per-worker handler
//! closure.

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::thread;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

use crate::http::{
    bind_listeners_reuseport, build_error_response_pub, cached_date_header_arc,
    lookup_static_route_pub, num_workers_pub as num_workers, RequestPrimitive, ResponseBody,
    ResponsePrimitive, StaticRouteSnapshot,
};

/// Bridge from hyper-thread → VM thread and back. Same `Job` shape as the
/// tiny_http path uses so the VM dispatcher code is identical.
pub(crate) type Job = (
    RequestPrimitive,
    std_mpsc::SyncSender<ResponsePrimitive>,
);

/// Drop-in replacement for [`crate::http::serve`]. Same arg layout, same
/// return value. Selected at runtime when `TISH_HTTP_BACKEND=hyper`.
pub fn serve<F>(args: &[tishlang_core::Value], handler: F) -> tishlang_core::Value
where
    F: Fn(&[tishlang_core::Value]) -> tishlang_core::Value,
{
    use tishlang_core::Value;

    let port = match args.first() {
        Some(Value::Number(n)) => *n as u16,
        _ => return build_error_response_pub("serve requires a port number"),
    };

    let max_requests: Option<usize> = args.get(2).and_then(|v| match v {
        Value::Number(n) if *n >= 1.0 => Some(*n as usize),
        _ => None,
    });

    // Kick the Date background thread so the first response has a value.
    let _ = cached_date_header_arc();

    // Prefork: same semantics as tiny_http path — spawn N-1 subprocesses,
    // each runs its own single-threaded tokio runtime + Tish VM.
    let role = crate::http_prefork::role_from_env();
    let prefork_n = crate::http::num_prefork_workers_pub();
    let mut in_prefork_group = false;
    match role {
        crate::http_prefork::PreforkRole::Parent if prefork_n > 1 => {
            match crate::http_prefork::spawn_children(prefork_n) {
                Ok(c) => {
                    let _ = crate::http_prefork::install_parent_signal_handler(c);
                    in_prefork_group = true;
                }
                Err(e) => {
                    eprintln!(
                        "[tish http/hyper] prefork spawn failed, running single process: {}",
                        e
                    );
                }
            }
        }
        _ => {}
    }

    let workers = if in_prefork_group { 1 } else { num_workers() };
    let listeners = match bind_listeners_reuseport(port, workers) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[tish http/hyper] failed to bind: {}", e);
            return build_error_response_pub(&e);
        }
    };

    let worker_id = match role {
        crate::http_prefork::PreforkRole::Child(id) => id,
        _ => 0,
    };
    if matches!(role, crate::http_prefork::PreforkRole::Parent) && in_prefork_group {
        println!(
            "tish http/hyper: listening on http://0.0.0.0:{} ({} process{} x {} accept thread{})",
            port,
            prefork_n,
            if prefork_n == 1 { "" } else { "es" },
            workers,
            if workers == 1 { "" } else { "s" }
        );
    } else if worker_id == 0 {
        println!(
            "tish http/hyper: listening on http://0.0.0.0:{} ({} worker{})",
            port,
            workers,
            if workers == 1 { "" } else { "s" }
        );
    }

    // Shared job queue (same shape as tiny_http path).
    let (tx, rx) = std_mpsc::sync_channel::<Job>(workers * 512);
    let stop = Arc::new(AtomicBool::new(false));

    // Spawn the HTTP worker threads. Each owns a single-threaded tokio
    // runtime and a SO_REUSEPORT-bound listener. Optionally pins to a
    // physical core via core_affinity (opt-in via env).
    let core_ids: Vec<Option<core_affinity::CoreId>> = {
        let pin = std::env::var("TISH_HTTP_PIN_CORES")
            .map(|v| v != "0" && v != "false")
            .unwrap_or(false);
        if pin {
            core_affinity::get_core_ids()
                .unwrap_or_default()
                .into_iter()
                .map(Some)
                .collect()
        } else {
            (0..workers).map(|_| None).collect()
        }
    };

    let mut worker_handles = Vec::with_capacity(workers);
    for (idx, listener) in listeners.into_iter().enumerate() {
        let tx = tx.clone();
        let stop = Arc::clone(&stop);
        let core = core_ids.get(idx).copied().flatten();
        let handle = thread::Builder::new()
            .name(format!("tish-http-h{}", idx))
            .spawn(move || worker_thread(listener, tx, stop, core))
            .expect("spawn tish-http-hyper worker");
        worker_handles.push(handle);
    }
    drop(tx);

    // VM-thread dispatcher loop: identical to the tiny_http path.
    let mut count = 0usize;
    while let Ok((req_prim, resp_tx)) = rx.recv() {
        let req_value = req_prim.into_value_pub();
        let response_value = handler(&[req_value]);
        let resp_prim = ResponsePrimitive::from_value_pub(&response_value);
        let _ = resp_tx.send(resp_prim);

        count += 1;
        if max_requests.map(|m| count >= m).unwrap_or(false) {
            stop.store(true, Ordering::Relaxed);
            break;
        }
    }

    stop.store(true, Ordering::Relaxed);
    for h in worker_handles {
        let _ = h.join();
    }

    Value::Null
}

/// Per-OS-thread entry: build a single-threaded tokio runtime, adopt the
/// pre-bound SO_REUSEPORT listener, serve connections with hyper.
fn worker_thread(
    listener: std::net::TcpListener,
    dispatch: std_mpsc::SyncSender<Job>,
    stop: Arc<AtomicBool>,
    core: Option<core_affinity::CoreId>,
) {
    if let Some(id) = core {
        let _ = core_affinity::set_for_current(id);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tish-http-hyper: build current-thread runtime");

    rt.block_on(async move {
        listener
            .set_nonblocking(true)
            .expect("tish-http-hyper: set_nonblocking");
        let tokio_listener = tokio::net::TcpListener::from_std(listener)
            .expect("tish-http-hyper: adopt tokio listener");

        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // Accept with a short timeout so we notice the stop flag.
            let accept_fut = tokio_listener.accept();
            let (stream, _peer) = match tokio::time::timeout(
                std::time::Duration::from_millis(250),
                accept_fut,
            )
            .await
            {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    eprintln!("[tish http/hyper] accept error: {}", e);
                    continue;
                }
                Err(_) => continue, // timeout; re-check stop flag
            };
            // TCP_NODELAY once, at the socket level.
            let _ = stream.set_nodelay(true);

            let dispatch = dispatch.clone();
            tokio::task::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let dispatch = dispatch.clone();
                    async move { handle_request(req, dispatch).await }
                });
                if let Err(e) = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(io, svc)
                    .await
                {
                    // Chatty connection errors (resets, client timeouts) are
                    // normal; silently drop unless debug logging is on.
                    if std::env::var_os("TISH_HTTP_DEBUG").is_some() {
                        eprintln!("[tish http/hyper] conn closed: {}", e);
                    }
                }
            });
        }
    });
}

/// Per-request hyper service. Static-route fast path first; otherwise
/// dispatch via mpsc to the VM thread.
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    dispatch: std_mpsc::SyncSender<Job>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Fast path: static routes (pre-baked bodies).
    let uri_path_str = req.uri().path();
    if let Some(route) = lookup_static_route_pub(uri_path_str) {
        return Ok(static_route_response(route));
    }

    // Slow path: cross the mpsc to the VM thread.
    let (method, url, path, query, headers, body_bytes) = extract_request(req).await;
    let body = String::from_utf8(body_bytes).unwrap_or_default();
    let prim = RequestPrimitive::new_pub(method, url, path, query, headers, body);

    let (resp_tx, resp_rx) = std_mpsc::sync_channel::<ResponsePrimitive>(1);
    if dispatch.send((prim, resp_tx)).is_err() {
        return Ok(simple_error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "tish http: dispatch channel closed",
        ));
    }

    // Block on the VM response from a tokio blocking worker so we don't
    // starve hyper's reactor while the Tish handler runs.
    let resp_prim = tokio::task::spawn_blocking(move || {
        resp_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .ok()
    })
    .await
    .ok()
    .flatten();

    match resp_prim {
        Some(r) => Ok(primitive_to_hyper(r)),
        None => Ok(simple_error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "tish handler timed out",
        )),
    }
}

async fn extract_request(
    req: Request<hyper::body::Incoming>,
) -> (
    String,
    String,
    String,
    String,
    Vec<(String, String)>,
    Vec<u8>,
) {
    let method = req.method().to_string();
    let uri = req.uri().clone();
    let url = uri.to_string();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body_bytes = match http_body_util::BodyExt::collect(req.into_body()).await {
        Ok(c) => c.to_bytes().to_vec(),
        Err(_) => Vec::new(),
    };
    (method, url, path, query, headers, body_bytes)
}

fn primitive_to_hyper(resp: ResponsePrimitive) -> Response<Full<Bytes>> {
    let (body_bytes, default_ct): (Bytes, Option<&str>) = match resp.body {
        ResponseBody::Text(s) => (Bytes::copy_from_slice(s.as_bytes()), Some("text/plain")),
        ResponseBody::Bytes(b) => (Bytes::from(b), Some("application/octet-stream")),
        ResponseBody::File(p) => match std::fs::read(&p) {
            Ok(b) => (Bytes::from(b), Some("application/octet-stream")),
            Err(_) => (Bytes::from_static(b"file not found"), Some("text/plain")),
        },
    };
    let mut builder = Response::builder().status(resp.status);
    let mut has_ct = false;
    let mut has_server = false;
    let mut has_date = false;
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("content-type") {
            has_ct = true;
        }
        if k.eq_ignore_ascii_case("server") {
            has_server = true;
        }
        if k.eq_ignore_ascii_case("date") {
            has_date = true;
        }
        builder = builder.header(k, v);
    }
    if !has_ct {
        if let Some(ct) = default_ct {
            builder = builder.header("content-type", ct);
        }
    }
    if !has_server {
        builder = builder.header("server", "Tish");
    }
    if !has_date {
        builder = builder.header("date", cached_date_header_arc().as_str());
    }
    builder.body(Full::new(body_bytes)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::new()))
            .unwrap()
    })
}

fn static_route_response(route: StaticRouteSnapshot) -> Response<Full<Bytes>> {
    let body = Bytes::copy_from_slice(&route.body);
    Response::builder()
        .status(200)
        .header("content-type", route.content_type.as_ref())
        .header("server", "Tish")
        .header("date", cached_date_header_arc().as_str())
        .body(Full::new(body))
        .unwrap()
}

fn simple_error_response(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .header("server", "Tish")
        .body(Full::new(Bytes::copy_from_slice(msg.as_bytes())))
        .unwrap()
}

/// Marker used by external callers (like the bench) to gate the backend
/// choice. Returns true when `TISH_HTTP_BACKEND=hyper` is in the env.
pub fn is_enabled_via_env() -> bool {
    std::env::var("TISH_HTTP_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("hyper"))
        .unwrap_or(false)
}
