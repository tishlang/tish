//! HTTP server + shared request parsing. Client `fetch` lives in `http_fetch.rs`.
//!
//! ## Concurrency model
//!
//! `serve(port, handler)` spawns `num_workers` OS threads (default
//! `num_cpus`, tuned via env `TISH_HTTP_WORKERS`). Each worker binds its own
//! accept socket with `SO_REUSEPORT` on Linux so the kernel load-balances
//! `accept()` across cores; on macOS we still bind N sockets with
//! `SO_REUSEPORT` (which exists but has different semantics) so each worker
//! gets its own tiny_http connection pool but the accept queue is shared at
//! the kernel level.
//!
//! The Tish handler closure captures `Value::Function(Rc<…>)` which is
//! `!Send`, so handler execution stays on the VM (caller) thread:
//!
//! ```text
//!   worker 0 ──┐
//!   worker 1 ──┼─> mpsc::SyncSender<Job> ──> VM thread (handler) ──> oneshot
//!   worker N ──┘                                                        │
//!                 worker writes response bytes (parallel) <─────────────┘
//! ```
//!
//! Parallel accept + parse + response-write while preserving the single-
//! threaded VM guarantee. Cached `Date:` header + shared `Arc<str>` response
//! bodies round out the hot-path optimisations.

use std::cell::RefCell;
use tishlang_core::VmRef;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tishlang_core::{ObjectMap, Value};
use tokio::runtime::Runtime;

thread_local! {
    pub(crate) static RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
}

/// Block on a future on the HTTP runtime. Uses a dedicated thread to avoid deadlocks when
/// called from contexts that share or nest tokio runtimes (e.g. WS + HTTP both enabled).
pub(crate) fn block_on_http<F>(f: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|s| {
        let (tx, rx) = std::sync::mpsc::channel();
        s.spawn(move || {
            let out = RUNTIME.with(|rt| rt.block_on(f));
            let _ = tx.send(out);
        });
        rx.recv().expect("block_on_http thread panicked")
    })
}

pub fn await_fetch(args: Vec<Value>) -> Value {
    crate::native_promise::await_promise(crate::native_promise::fetch_promise(args))
}

pub fn await_fetch_all(args: Vec<Value>) -> Value {
    crate::native_promise::await_promise(crate::native_promise::fetch_all_promise(args))
}

pub(crate) fn extract_method(options: Option<&Value>) -> String {
    options
        .and_then(|v| match v {
            Value::Object(obj) => obj.borrow().get(&Arc::from("method")).cloned(),
            _ => None,
        })
        .map(|v| v.to_display_string().to_uppercase())
        .unwrap_or_else(|| "GET".to_string())
}

pub(crate) fn extract_headers(options: Option<&Value>) -> Vec<(String, String)> {
    options
        .and_then(|v| match v {
            Value::Object(obj) => obj.borrow().get(&Arc::from("headers")).cloned(),
            _ => None,
        })
        .map(|v| match v {
            Value::Object(obj) => obj
                .borrow()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_display_string()))
                .collect(),
            _ => vec![],
        })
        .unwrap_or_default()
}

pub(crate) fn extract_body(options: Option<&Value>) -> Option<String> {
    options.and_then(|v| match v {
        Value::Object(obj) => obj
            .borrow()
            .get(&Arc::from("body"))
            .map(|v| v.to_display_string()),
        _ => None,
    })
}

pub(crate) fn build_error_response(error: &str) -> Value {
    let mut obj: ObjectMap = ObjectMap::with_capacity(2);
    obj.insert(Arc::from("error"), Value::String(error.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::Object(VmRef::new(obj))
}

// -------- cached Date header -----------------------------------------------
//
// Lock-free via `arc-swap`: readers do `load().clone()` which is a single
// atomic fetch + ref-count inc. Writers (the 1 Hz background thread) do
// `store(new Arc)`. No Mutex contention on the 100k+ RPS path.

static DATE_HEADER: OnceLock<arc_swap::ArcSwap<String>> = OnceLock::new();

fn format_http_date(now_secs: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let z = now_secs as i64 / 86_400;
    let secs_of_day = (now_secs as i64).rem_euclid(86_400);
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    let dow = ((z + 4).rem_euclid(7)) as usize;
    let (y, mo, d) = civil_from_days(z);
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAYS[dow],
        d,
        MONTHS[(mo - 1) as usize],
        y,
        h,
        m,
        s
    )
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn ensure_date_thread() -> &'static arc_swap::ArcSwap<String> {
    DATE_HEADER.get_or_init(|| {
        let initial = Arc::new(format_http_date(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ));
        let slot = arc_swap::ArcSwap::new(initial);
        thread::Builder::new()
            .name("tish-http-date".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(1000));
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let next = Arc::new(format_http_date(now));
                if let Some(cell) = DATE_HEADER.get() {
                    cell.store(next);
                }
            })
            .ok();
        slot
    })
}

/// Lock-free snapshot of the cached Date header. Callers get an `Arc<String>`
/// clone (single atomic ref-count inc).
pub fn cached_date_header_arc() -> Arc<String> {
    ensure_date_thread().load_full()
}

/// Back-compat String flavour (allocates). New hot-path code should use
/// `cached_date_header_arc`.
pub fn cached_date_header() -> String {
    cached_date_header_arc().as_str().to_string()
}

// -------- Send-safe request/response primitives ----------------------------

#[derive(Debug, Clone)]
pub struct RequestPrimitive {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RequestPrimitive {
    fn from_tiny_http(req: &mut tiny_http::Request) -> Self {
        // Single-pass split of url into (path, query) so we avoid 2 extra
        // String allocations per request (~300 ns at M-series load).
        let url_str = req.url();
        let (path, query) = match url_str.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (url_str.to_string(), String::new()),
        };
        let url = url_str.to_string();
        let method = req.method().to_string();
        // Fast path: GET/HEAD/OPTIONS almost never have a body. Skip the
        // reader allocation + syscall unless the body is advertised.
        let has_body = !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
            && req.body_length().map(|n| n > 0).unwrap_or(true);
        let mut body = String::new();
        if has_body {
            let _ = req.as_reader().read_to_string(&mut body);
        }
        let headers = req
            .headers()
            .iter()
            .map(|h| {
                (
                    h.field.as_str().as_str().to_string(),
                    h.value.as_str().to_string(),
                )
            })
            .collect();
        Self {
            method,
            url,
            path,
            query,
            headers,
            body,
        }
    }

    fn into_value(self) -> Value {
        // Interned keys: reuse the same Arc<str> across every request object
        // (saves 6 Arc::from allocations per request on the dispatcher hot
        // path). Thread-local because Value / Arc<str> are local-scope.
        thread_local! {
            static KEYS: RequestKeys = RequestKeys::new();
        }
        KEYS.with(|keys| {
            let mut obj: ObjectMap = ObjectMap::with_capacity(6);
            obj.insert(Arc::clone(&keys.method), Value::String(self.method.into()));
            obj.insert(Arc::clone(&keys.url), Value::String(self.url.into()));
            obj.insert(Arc::clone(&keys.path), Value::String(self.path.into()));
            obj.insert(Arc::clone(&keys.query), Value::String(self.query.into()));
            let mut h: ObjectMap = ObjectMap::with_capacity(self.headers.len());
            for (k, v) in self.headers {
                h.insert(Arc::from(k), Value::String(v.into()));
            }
            obj.insert(
                Arc::clone(&keys.headers),
                Value::Object(VmRef::new(h)),
            );
            obj.insert(Arc::clone(&keys.body), Value::String(self.body.into()));
            Value::Object(VmRef::new(obj))
        })
    }
}

struct RequestKeys {
    method: Arc<str>,
    url: Arc<str>,
    path: Arc<str>,
    query: Arc<str>,
    headers: Arc<str>,
    body: Arc<str>,
}

impl RequestKeys {
    fn new() -> Self {
        Self {
            method: Arc::from("method"),
            url: Arc::from("url"),
            path: Arc::from("path"),
            query: Arc::from("query"),
            headers: Arc::from("headers"),
            body: Arc::from("body"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponsePrimitive {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ResponseBody,
}

#[derive(Debug, Clone)]
pub enum ResponseBody {
    /// Text body; `Arc<str>` shares the value with Tish's `Value::String`.
    Text(Arc<str>),
    Bytes(Vec<u8>),
    File(String),
}

impl ResponsePrimitive {
    fn from_value(value: &Value) -> Self {
        if let Some((status, headers, file)) = extract_file_from_response(value) {
            return Self {
                status,
                headers,
                body: ResponseBody::File(file),
            };
        }

        let default_status = 200u16;
        match value {
            Value::Object(obj) => {
                let obj_ref = obj.borrow();

                let status = obj_ref
                    .get(&Arc::from("status"))
                    .and_then(|v| match v {
                        Value::Number(n) => Some(*n as u16),
                        _ => None,
                    })
                    .unwrap_or(default_status);

                let has_error = obj_ref.contains_key(&Arc::from("error"));

                let body: ResponseBody = if let Some(bb) = obj_ref.get(&Arc::from("bodyBytes")) {
                    match bb {
                        Value::Array(a) => {
                            let v: Vec<u8> = a
                                .borrow()
                                .iter()
                                .filter_map(|x| match x {
                                    Value::Number(n) => Some((*n as u32 & 0xff) as u8),
                                    _ => None,
                                })
                                .collect();
                            ResponseBody::Bytes(v)
                        }
                        _ => ResponseBody::Text(Arc::from(bb.to_display_string())),
                    }
                } else if let Some(b) = obj_ref.get(&Arc::from("body")) {
                    match b {
                        Value::String(s) => ResponseBody::Text(Arc::clone(s)),
                        Value::Array(a) => {
                            let borrow = a.borrow();
                            if !borrow.is_empty()
                                && borrow.iter().all(|x| matches!(x, Value::Number(_)))
                            {
                                ResponseBody::Bytes(
                                    borrow
                                        .iter()
                                        .filter_map(|x| match x {
                                            Value::Number(n) => Some((*n as u32 & 0xff) as u8),
                                            _ => None,
                                        })
                                        .collect(),
                                )
                            } else {
                                ResponseBody::Text(Arc::from(b.to_display_string()))
                            }
                        }
                        _ => ResponseBody::Text(Arc::from(b.to_display_string())),
                    }
                } else if has_error {
                    ResponseBody::Text(Arc::from(
                        obj_ref
                            .get(&Arc::from("error"))
                            .map(|v| v.to_display_string())
                            .unwrap_or_default(),
                    ))
                } else {
                    ResponseBody::Text(Arc::from(""))
                };

                let status = if has_error && status == default_status {
                    500
                } else {
                    status
                };

                let headers = obj_ref
                    .get(&Arc::from("headers"))
                    .and_then(|v| match v {
                        Value::Object(h) => Some(
                            h.borrow()
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_display_string()))
                                .collect(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_default();

                Self {
                    status,
                    headers,
                    body,
                }
            }
            Value::String(s) => Self {
                status: default_status,
                headers: vec![],
                body: ResponseBody::Text(Arc::clone(s)),
            },
            _ => Self {
                status: default_status,
                headers: vec![],
                body: ResponseBody::Text(Arc::from("")),
            },
        }
    }
}

// -------- legacy shims -----------------------------------------------------

#[allow(dead_code)]
pub fn request_to_value(request: &mut tiny_http::Request) -> Value {
    RequestPrimitive::from_tiny_http(request).into_value()
}

#[allow(dead_code)]
pub fn value_to_response(value: &Value) -> (u16, Vec<(String, String)>, String) {
    let r = ResponsePrimitive::from_value(value);
    let body = match r.body {
        ResponseBody::Text(s) => s.to_string(),
        ResponseBody::Bytes(b) => String::from_utf8(b).unwrap_or_default(),
        ResponseBody::File(_) => String::new(),
    };
    (r.status, r.headers, body)
}

fn extract_file_from_response(value: &Value) -> Option<(u16, Vec<(String, String)>, String)> {
    let Value::Object(obj) = value else {
        return None;
    };
    let obj_ref = obj.borrow();
    let Value::String(file_path) = obj_ref.get(&Arc::from("file"))? else {
        return None;
    };
    let file_path = file_path.to_string();
    let status = obj_ref
        .get(&Arc::from("status"))
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as u16),
            _ => None,
        })
        .unwrap_or(200);
    let headers = obj_ref
        .get(&Arc::from("headers"))
        .and_then(|v| match v {
            Value::Object(h) => Some(
                h.borrow()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_display_string()))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    Some((status, headers, file_path))
}

// -------- response writers -------------------------------------------------

fn inject_default_headers(headers: &mut Vec<(String, String)>) {
    let has_date = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Date"));
    let has_server = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Server"));
    if !has_date {
        // One atomic load + ref-inc + one String allocation for the Vec slot.
        headers.push(("Date".into(), cached_date_header_arc().as_str().to_string()));
    }
    if !has_server {
        headers.push(("Server".into(), "Tish".into()));
    }
}

#[allow(dead_code)]
pub fn send_response(
    request: tiny_http::Request,
    status: u16,
    mut headers: Vec<(String, String)>,
    body: String,
) {
    send_response_arc(request, status, headers.drain(..).collect(), Arc::from(body));
}

pub fn send_response_arc(
    request: tiny_http::Request,
    status: u16,
    mut headers: Vec<(String, String)>,
    body: Arc<str>,
) {
    inject_default_headers(&mut headers);
    let status_code = tiny_http::StatusCode(status);
    let len = body.len();
    let bytes: Arc<[u8]> = Arc::from(body.as_bytes());
    let mut response = tiny_http::Response::new(
        status_code,
        vec![],
        ArcBytesReader::new(bytes),
        Some(len),
        None,
    );
    for (key, value) in headers {
        if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    let _ = request.respond(response);
}

struct ArcBytesReader {
    bytes: Arc<[u8]>,
    pos: usize,
}

impl ArcBytesReader {
    fn new(bytes: Arc<[u8]>) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl std::io::Read for ArcBytesReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.bytes.len().saturating_sub(self.pos);
        let n = remaining.min(buf.len());
        if n == 0 {
            return Ok(0);
        }
        buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

pub fn send_response_bytes(
    request: tiny_http::Request,
    status: u16,
    mut headers: Vec<(String, String)>,
    body: Vec<u8>,
) {
    inject_default_headers(&mut headers);
    let status_code = tiny_http::StatusCode(status);
    let len = body.len();
    let mut response = tiny_http::Response::new(
        status_code,
        vec![],
        std::io::Cursor::new(body),
        Some(len),
        None,
    );
    for (key, value) in headers {
        if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    let _ = request.respond(response);
}

fn send_file_response(
    request: tiny_http::Request,
    status: u16,
    mut headers: Vec<(String, String)>,
    file_path: String,
) {
    inject_default_headers(&mut headers);
    let file = match File::open(&file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open file {}: {}", file_path, e);
            let fallback =
                tiny_http::Response::from_string(format!("File not found: {}", file_path))
                    .with_status_code(tiny_http::StatusCode(500));
            let _ = request.respond(fallback);
            return;
        }
    };
    let status_code = tiny_http::StatusCode(status);
    let mut response = tiny_http::Response::from_file(file).with_status_code(status_code);
    for (key, value) in headers {
        if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    let _ = request.respond(response);
}

fn respond_from_primitive(request: tiny_http::Request, resp: ResponsePrimitive) {
    match resp.body {
        ResponseBody::Text(s) => send_response_arc(request, resp.status, resp.headers, s),
        ResponseBody::Bytes(b) => send_response_bytes(request, resp.status, resp.headers, b),
        ResponseBody::File(p) => send_file_response(request, resp.status, resp.headers, p),
    }
}

// -------- static-route fast path ------------------------------------------
//
// Endpoints whose body is constant (TFB `/plaintext` and `/json`) don't need
// to roundtrip through the Tish VM per request. `register_static_route`
// stores the pre-built response bytes in a process-wide table; workers
// consult the table before pushing to the dispatcher and serve the
// response directly, skipping Value construction and channel hops entirely.
//
// This is the main lever that lets N workers actually *scale* for plaintext:
// without it, every request serialises through the single VM thread.

#[derive(Clone)]
struct StaticRoute {
    body: Arc<[u8]>,
    content_type: Arc<str>,
}

type StaticRoutes = std::collections::HashMap<String, StaticRoute>;

/// Lock-free static-route map: readers use `load()` which is a single atomic
/// operation. Writers (registration) do `rcu` to publish a new Arc. This is
/// what lets per-worker /plaintext lookups actually scale — a Mutex here was
/// bouncing cache lines between every worker thread on every request.
static STATIC_ROUTES: OnceLock<arc_swap::ArcSwap<StaticRoutes>> = OnceLock::new();

fn static_routes() -> &'static arc_swap::ArcSwap<StaticRoutes> {
    STATIC_ROUTES
        .get_or_init(|| arc_swap::ArcSwap::new(Arc::new(StaticRoutes::new())))
}

/// Register a static response for `path`. Subsequent requests to exactly
/// that path (ignoring query string) are served by HTTP workers directly,
/// bypassing the Tish VM dispatcher. Content-type and Server/Date are
/// appended automatically.
pub fn register_static_route(path: &str, body: &[u8], content_type: &str) {
    let cell = static_routes();
    cell.rcu(|cur| {
        let mut next = (**cur).clone();
        next.insert(
            path.to_string(),
            StaticRoute {
                body: Arc::from(body),
                content_type: Arc::from(content_type),
            },
        );
        Arc::new(next)
    });
}

fn lookup_static_route(path: &str) -> Option<StaticRoute> {
    // Strip query string in-place (no allocation).
    let pure = path.split('?').next().unwrap_or(path);
    let guard = static_routes().load();
    guard.get(pure).cloned()
}

fn serve_static_route(request: tiny_http::Request, route: StaticRoute) {
    let status_code = tiny_http::StatusCode(200);
    let body_len = route.body.len();
    let mut response = tiny_http::Response::new(
        status_code,
        vec![],
        ArcBytesReader::new(route.body),
        Some(body_len),
        None,
    );
    // Hand-built headers; Date comes from the cached slot.
    if let Ok(h) =
        tiny_http::Header::from_bytes(b"Content-Type".as_slice(), route.content_type.as_bytes())
    {
        response = response.with_header(h);
    }
    if let Ok(h) = tiny_http::Header::from_bytes(b"Server".as_slice(), b"Tish".as_slice()) {
        response = response.with_header(h);
    }
    let date = cached_date_header_arc();
    if let Ok(h) = tiny_http::Header::from_bytes(b"Date".as_slice(), date.as_bytes()) {
        response = response.with_header(h);
    }
    let _ = request.respond(response);
}

// -------- SO_REUSEPORT listeners -------------------------------------------

#[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
fn set_reuse_port(s: &socket2::Socket) {
    use std::os::fd::AsRawFd;
    let fd = s.as_raw_fd();
    let on: libc::c_int = 1;
    unsafe {
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &on as *const _ as *const libc::c_void,
            std::mem::size_of_val(&on) as libc::socklen_t,
        );
    }
}

fn bind_listeners(port: u16, n: usize) -> Result<Vec<std::net::TcpListener>, String> {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};

    let addr: std::net::SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let sa: SockAddr = addr.into();

    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let s = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
            .map_err(|e| format!("socket: {}", e))?;
        s.set_reuse_address(true)
            .map_err(|e| format!("set_reuse_address: {}", e))?;
        #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
        {
            set_reuse_port(&s);
        }
        s.set_nodelay(true).ok();
        s.bind(&sa).map_err(|e| format!("bind {}: {}", port, e))?;
        s.listen(1024).map_err(|e| format!("listen: {}", e))?;
        out.push(s.into());
    }
    Ok(out)
}

fn num_workers() -> usize {
    if let Ok(v) = std::env::var("TISH_HTTP_WORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            if n >= 1 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Target number of prefork worker **processes**. Distinct from
/// `num_workers`, which is the number of OS *threads* per process.
///
/// In prefork mode every process runs one accept thread, so this is the
/// knob that actually controls parallelism. Defaults to the number of
/// logical CPUs; `TISH_PREFORK_WORKERS=N` or `TISH_HTTP_WORKERS=N` overrides.
pub(crate) fn num_prefork_workers() -> usize {
    if let Ok(v) = std::env::var("TISH_PREFORK_WORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            if n >= 1 {
                return n;
            }
        }
    }
    // Accept TISH_HTTP_WORKERS too so users don't have to learn a new var
    // just to turn on prefork — the docs advertise a single env var.
    if let Ok(v) = std::env::var("TISH_HTTP_WORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            if n >= 1 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

// -------- serve() ----------------------------------------------------------

type Job = (RequestPrimitive, mpsc::SyncSender<ResponsePrimitive>);

/// Start an HTTP server that handles requests using the provided handler function.
///
/// When `send-values` is enabled (required for the `http` feature) the
/// handler is `Fn + Send + Sync`, so we run it directly on each HTTP
/// accept thread instead of funneling every request through a single
/// dispatcher. That's the multi-core unlock for the VM: handler calls
/// execute in parallel across N OS threads, with `Value` safely shared
/// via `VmRef` (`Arc<Mutex>`) on every array/object.
///
/// On builds without `send-values` (wasm / single-threaded targets)
/// the handler only needs `Fn`; we fall back to the mpsc-dispatch model
/// where all handler calls execute on one VM thread.
#[cfg(feature = "send-values")]
pub fn serve<F>(args: &[Value], handler: F) -> Value
where
    F: Fn(&[Value]) -> Value + Send + Sync + 'static,
{
    serve_impl(args, handler)
}

#[cfg(not(feature = "send-values"))]
pub fn serve<F>(args: &[Value], handler: F) -> Value
where
    F: Fn(&[Value]) -> Value,
{
    serve_impl(args, handler)
}

#[cfg(feature = "send-values")]
fn serve_impl<F>(args: &[Value], handler: F) -> Value
where
    F: Fn(&[Value]) -> Value + Send + Sync + 'static,
{
    let port = match args.first() {
        Some(Value::Number(n)) => *n as u16,
        _ => return build_error_response("serve requires a port number"),
    };

    let max_requests: Option<usize> = args.get(2).and_then(|v| match v {
        Value::Number(n) if *n >= 1.0 => Some(*n as usize),
        _ => None,
    });

    ensure_date_thread();

    // --- prefork: spawn one subprocess per extra core --------------------
    //
    // This is the big multi-core unlock for Tish HTTP: because the Tish VM
    // runs on `Rc`/`RefCell` (`Value` is `!Send`), we can't run multiple
    // handler threads in one process. Instead, we fork the process — each
    // child re-execs the same binary and runs its own single-threaded VM.
    // All N processes `SO_REUSEPORT`-bind the same port and the kernel
    // load-balances connections across them. Same pattern as nginx,
    // gunicorn, puma cluster, phpfpm.
    //
    // * Parent (role = Parent) spawns N-1 children and keeps its own accept
    //   loop running (so we don't waste a core just coordinating).
    // * Children (role = Child(id)) just run their accept loop.
    // * Single (role = Single) skips forking — explicit opt-out, or we're
    //   already inside a child.
    let role = crate::http_prefork::role_from_env();
    let prefork_n = num_prefork_workers();
    let mut children = Vec::new();
    let mut prefork_stop: Option<Arc<AtomicBool>> = None;
    match role {
        crate::http_prefork::PreforkRole::Parent if prefork_n > 1 => {
            match crate::http_prefork::spawn_children(prefork_n) {
                Ok(c) => {
                    let handles = crate::http_prefork::install_parent_signal_handler(c);
                    prefork_stop = Some(handles);
                    // Children are spawned; fall through and run our own
                    // accept loop as worker 0.
                    children.push(()); // sentinel: we're in a prefork group
                }
                Err(e) => {
                    eprintln!(
                        "[tish http] prefork spawn failed, continuing as single process: {}",
                        e
                    );
                }
            }
        }
        _ => {}
    }

    // Per-process accept threads. In prefork mode this is 1; SO_REUSEPORT
    // between processes provides kernel-level load balancing. Without
    // prefork we fall back to the old multi-thread-in-one-process layout.
    let workers = if children.is_empty() {
        num_workers()
    } else {
        1
    };
    let listeners = match bind_listeners(port, workers) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[tish http] failed to bind: {}", e);
            return build_error_response(&e);
        }
    };

    let worker_id = match role {
        crate::http_prefork::PreforkRole::Child(id) => id,
        _ => 0,
    };
    if matches!(role, crate::http_prefork::PreforkRole::Parent) && !children.is_empty() {
        println!(
            "tish http: listening on http://0.0.0.0:{} ({} process{} x {} accept thread{})",
            port,
            prefork_n,
            if prefork_n == 1 { "" } else { "es" },
            workers,
            if workers == 1 { "" } else { "s" }
        );
    } else if worker_id == 0 {
        println!(
            "tish http: listening on http://0.0.0.0:{} ({} worker{})",
            port,
            workers,
            if workers == 1 { "" } else { "s" }
        );
    }

    let stop = prefork_stop.clone().unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    if max_requests == Some(1) {
        let p = port;
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            if let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{}", p)) {
                let _ =
                    s.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
        });
    }

    // Shared handler — every accept thread invokes it directly. Because
    // `Value: Send + Sync` under `send-values`, this is race-free: each
    // request object is freshly constructed on the worker thread, and any
    // shared Value inside the handler is protected by its own `VmRef`.
    let handler: Arc<dyn Fn(&[Value]) -> Value + Send + Sync> = Arc::new(handler);

    let processed = Arc::new(AtomicUsize::new(0));
    let mut worker_handles = Vec::with_capacity(workers);
    for (idx, listener) in listeners.into_iter().enumerate() {
        let handler = Arc::clone(&handler);
        let stop = Arc::clone(&stop);
        let processed = Arc::clone(&processed);
        let max = max_requests;
        let handle = thread::Builder::new()
            .name(format!("tish-http-w{}", idx))
            .spawn(move || worker_loop_direct(listener, handler, stop, processed, max))
            .expect("spawn tish-http worker");
        worker_handles.push(handle);
    }

    // Wait until one of: stop flag set, a worker bumps `processed >= max`, or
    // we're shutting down (parent received SIGINT).
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(m) = max_requests {
            if processed.load(Ordering::Relaxed) >= m {
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Kick each accept so its blocking `recv_timeout` wakes up promptly.
    for _ in 0..worker_handles.len() {
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", port));
    }
    for h in worker_handles {
        let _ = h.join();
    }

    Value::Null
}

#[cfg(not(feature = "send-values"))]
fn serve_impl<F>(args: &[Value], handler: F) -> Value
where
    F: Fn(&[Value]) -> Value,
{
    // Single-threaded dispatch path: multiple accept threads push onto a
    // shared `mpsc::sync_channel`, one VM thread (this one) drains and runs
    // the handler. Used by wasm / single-threaded Rust builds where
    // `Value` is `Rc`-backed and not `Send`.
    let port = match args.first() {
        Some(Value::Number(n)) => *n as u16,
        _ => return build_error_response("serve requires a port number"),
    };
    let max_requests: Option<usize> = args.get(2).and_then(|v| match v {
        Value::Number(n) if *n >= 1.0 => Some(*n as usize),
        _ => None,
    });
    ensure_date_thread();
    let workers = num_workers();
    let listeners = match bind_listeners(port, workers) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[tish http] failed to bind: {}", e);
            return build_error_response(&e);
        }
    };
    println!(
        "tish http: listening on http://0.0.0.0:{} ({} worker{}, single-vm)",
        port,
        workers,
        if workers == 1 { "" } else { "s" }
    );

    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel::<Job>(workers * 256);

    if max_requests == Some(1) {
        let p = port;
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            if let Ok(mut s) = std::net::TcpStream::connect(format!("127.0.0.1:{}", p)) {
                let _ =
                    s.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
        });
    }

    let mut worker_handles = Vec::with_capacity(workers);
    for (idx, listener) in listeners.into_iter().enumerate() {
        let tx = tx.clone();
        let stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name(format!("tish-http-w{}", idx))
            .spawn(move || worker_loop(listener, tx, stop))
            .expect("spawn tish-http worker");
        worker_handles.push(handle);
    }
    drop(tx);

    let mut count = 0usize;
    while let Ok((req_prim, resp_tx)) = rx.recv() {
        let req_value = req_prim.into_value();
        let response_value = handler(&[req_value]);
        let resp_prim = ResponsePrimitive::from_value(&response_value);
        let _ = resp_tx.send(resp_prim);

        count += 1;
        if max_requests.map(|m| count >= m).unwrap_or(false) {
            stop.store(true, Ordering::Relaxed);
            break;
        }
    }

    stop.store(true, Ordering::Relaxed);
    for _ in 0..worker_handles.len() {
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", port));
    }
    for h in worker_handles {
        let _ = h.join();
    }

    Value::Null
}

/// Parallel accept + dispatch loop used when `send-values` is on. The
/// handler runs on the same OS thread that accepted the connection, so
/// there is no cross-thread queue on the hot path. Static-route fast path
/// is unchanged.
#[cfg(feature = "send-values")]
fn worker_loop_direct(
    listener: std::net::TcpListener,
    handler: Arc<dyn Fn(&[Value]) -> Value + Send + Sync>,
    stop: Arc<AtomicBool>,
    processed: Arc<AtomicUsize>,
    max_requests: Option<usize>,
) {
    let server = match tiny_http::Server::from_listener(listener, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[tish http] worker failed to adopt listener: {}", e);
            return;
        }
    };
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(mut request)) => {
                // Static-route fast path: serve pre-baked bytes without
                // touching the Tish VM at all.
                if let Some(route) = lookup_static_route(request.url()) {
                    serve_static_route(request, route);
                } else {
                    let req_prim = RequestPrimitive::from_tiny_http(&mut request);
                    let req_value = req_prim.into_value();
                    let response_value = handler(&[req_value]);
                    let resp_prim = ResponsePrimitive::from_value(&response_value);
                    respond_from_primitive(request, resp_prim);
                }
                if let Some(m) = max_requests {
                    let p = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if p >= m {
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
            Ok(None) => {} // timeout; re-check stop flag
            Err(_) => break,
        }
    }
}

fn worker_loop(
    listener: std::net::TcpListener,
    dispatch: mpsc::SyncSender<Job>,
    stop: Arc<AtomicBool>,
) {
    let server = match tiny_http::Server::from_listener(listener, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[tish http] worker failed to adopt listener: {}", e);
            return;
        }
    };

    // Per-worker buffer of in-flight requests. We interleave accept with
    // response drain so a slow VM-thread handler doesn't block new accepts
    // (up to the pending capacity) and so single-request responses still
    // flush immediately when accept() blocks on the next connection.
    let mut pending: VecDeque<(tiny_http::Request, mpsc::Receiver<ResponsePrimitive>)> =
        VecDeque::with_capacity(256);

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Short-poll accept so we can drain pending responses even when
        // there's no new request arriving.
        match server.recv_timeout(Duration::from_millis(1)) {
            Ok(Some(mut request)) => {
                // Static-route fast path: serve pre-baked bytes without a
                // round trip through the VM dispatcher. This is what lets
                // per-worker accept actually scale for /plaintext + /json.
                if let Some(route) = lookup_static_route(request.url()) {
                    serve_static_route(request, route);
                    continue;
                }
                let req_prim = RequestPrimitive::from_tiny_http(&mut request);
                let (resp_tx, resp_rx) = mpsc::sync_channel::<ResponsePrimitive>(1);
                if dispatch.send((req_prim, resp_tx)).is_err() {
                    break;
                }
                pending.push_back((request, resp_rx));
            }
            Ok(None) => {
                // timed out; fall through to drain
            }
            Err(_) => break,
        }

        // Drain all ready responses (FIFO preserves order).
        while let Some((_, rx)) = pending.front() {
            match rx.try_recv() {
                Ok(resp) => {
                    let (req, _rx) = pending.pop_front().unwrap();
                    respond_from_primitive(req, resp);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    pending.pop_front();
                }
            }
        }
    }

    while let Some((req, rx)) = pending.pop_front() {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(resp) => respond_from_primitive(req, resp),
            Err(_) => drop(req),
        }
    }
}

#[allow(dead_code)]
pub fn create_server(port: u16) -> Result<tiny_http::Server, String> {
    let addr = format!("0.0.0.0:{}", port);
    tiny_http::Server::http(&addr).map_err(|e| format!("Failed to start server: {}", e))
}

// -------- public shims for alternate HTTP backends ------------------------
//
// Exposed so `http_hyper.rs` (and any future backend) can reuse the same
// request / response primitives, static-route table, cached Date header,
// SO_REUSEPORT binder, and worker count. Keeping these in one place
// guarantees parity between tiny_http and hyper code paths — cached Date,
// Server header, static-route table, key interning all work identically.

#[cfg(feature = "http-hyper")]
impl RequestPrimitive {
    /// Build a `RequestPrimitive` from already-parsed parts. Used by
    /// backends that do their own HTTP parsing (e.g. hyper).
    pub fn new_pub(
        method: String,
        url: String,
        path: String,
        query: String,
        headers: Vec<(String, String)>,
        body: String,
    ) -> Self {
        Self {
            method,
            url,
            path,
            query,
            headers,
            body,
        }
    }

    /// Same as the crate-private `into_value` but re-exported for
    /// alternate HTTP backends.
    pub fn into_value_pub(self) -> Value {
        self.into_value()
    }
}

#[cfg(feature = "http-hyper")]
impl ResponsePrimitive {
    /// Public alias of the crate-private `from_value`. Used by alternate
    /// HTTP backends (hyper) so Tish handlers return the same response
    /// shape regardless of which server is underneath.
    pub fn from_value_pub(value: &Value) -> Self {
        Self::from_value(value)
    }
}

/// Public snapshot of a static route (body + content-type), returned to
/// alternate HTTP backends so they can serve pre-baked responses without
/// reaching into our private types.
#[cfg(feature = "http-hyper")]
#[derive(Clone)]
pub struct StaticRouteSnapshot {
    pub body: Arc<[u8]>,
    pub content_type: Arc<str>,
}

#[cfg(feature = "http-hyper")]
pub fn lookup_static_route_pub(path: &str) -> Option<StaticRouteSnapshot> {
    lookup_static_route(path).map(|r| StaticRouteSnapshot {
        body: r.body,
        content_type: r.content_type,
    })
}

#[cfg(feature = "http-hyper")]
pub fn num_workers_pub() -> usize {
    num_workers()
}

#[cfg(feature = "http-hyper")]
pub fn num_prefork_workers_pub() -> usize {
    num_prefork_workers()
}

#[cfg(feature = "http-hyper")]
pub fn bind_listeners_reuseport(
    port: u16,
    n: usize,
) -> Result<Vec<std::net::TcpListener>, String> {
    bind_listeners(port, n)
}

#[cfg(feature = "http-hyper")]
pub fn build_error_response_pub(msg: &str) -> Value {
    build_error_response(msg)
}

