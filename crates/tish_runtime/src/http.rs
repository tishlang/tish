//! HTTP server + shared request parsing. Client `fetch` lives in `http_fetch.rs`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::Value;
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
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(2);
    obj.insert(Arc::from("error"), Value::String(error.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::Object(Rc::new(RefCell::new(obj)))
}

/// Start an HTTP server that handles requests using the provided handler function.
pub fn serve<F>(args: &[Value], handler: F) -> Value
where
    F: Fn(&[Value]) -> Value,
{
    let port = match args.first() {
        Some(Value::Number(n)) => *n as u16,
        _ => return build_error_response("serve requires a port number"),
    };

    let max_requests: Option<usize> = args.get(2).and_then(|v| match v {
        Value::Number(n) if *n >= 1.0 => Some(*n as usize),
        _ => None,
    });

    let server = match create_server(port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[tish http] Failed to bind: {}", e);
            return build_error_response(&e);
        }
    };

    println!("Server listening on http://0.0.0.0:{}", port);

    if max_requests == Some(1) {
        let port = port;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(mut stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
                let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });
    }

    let mut count = 0usize;
    for mut request in server.incoming_requests() {
        let req_value = request_to_value(&mut request);
        let response_value = handler(&[req_value]);
        if let Some((status, headers, file_path)) = extract_file_from_response(&response_value) {
            send_file_response(request, status, headers, file_path);
        } else {
            let (status, headers, body) = value_to_response(&response_value);
            send_response(request, status, headers, body);
        }
        count += 1;
        if max_requests.map(|m| count >= m).unwrap_or(false) {
            break;
        }
    }

    Value::Null
}

pub fn create_server(port: u16) -> Result<tiny_http::Server, String> {
    let addr = format!("0.0.0.0:{}", port);
    tiny_http::Server::http(&addr).map_err(|e| format!("Failed to start server: {}", e))
}

pub fn request_to_value(request: &mut tiny_http::Request) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(6);

    obj.insert(
        Arc::from("method"),
        Value::String(request.method().to_string().into()),
    );
    obj.insert(
        Arc::from("url"),
        Value::String(request.url().to_string().into()),
    );

    let path = request.url().split('?').next().unwrap_or("/");
    obj.insert(Arc::from("path"), Value::String(path.into()));

    let query_string = request.url().split('?').nth(1).unwrap_or("");
    obj.insert(Arc::from("query"), Value::String(query_string.into()));

    let mut headers_obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(request.headers().len());
    for header in request.headers() {
        headers_obj.insert(
            Arc::from(header.field.as_str().as_str()),
            Value::String(header.value.as_str().into()),
        );
    }
    obj.insert(
        Arc::from("headers"),
        Value::Object(Rc::new(RefCell::new(headers_obj))),
    );

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    obj.insert(Arc::from("body"), Value::String(body.into()));

    Value::Object(Rc::new(RefCell::new(obj)))
}

pub fn value_to_response(value: &Value) -> (u16, Vec<(String, String)>, String) {
    let default_status = 200u16;
    let default_body = String::new();

    let (status, headers, body) = match value {
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
            let body = obj_ref
                .get(&Arc::from("body"))
                .map(|v| v.to_display_string())
                .unwrap_or_else(|| {
                    obj_ref
                        .get(&Arc::from("error"))
                        .map(|v| v.to_display_string())
                        .unwrap_or_default()
                });
            let status = if has_error && status == default_status {
                500u16
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

            (status, headers, body)
        }
        Value::String(s) => (default_status, vec![], s.to_string()),
        _ => (default_status, vec![], default_body),
    };

    (status, headers, body)
}

/// If the response value has a "file" key, extract (status, headers, file_path) for streaming.
/// Used for binary files (e.g. .wasm) where readFile/body would fail.
fn extract_file_from_response(value: &Value) -> Option<(u16, Vec<(String, String)>, String)> {
    let Value::Object(obj) = value else { return None };
    let obj_ref = obj.borrow();
    let Value::String(file_path) = obj_ref.get(&Arc::from("file"))? else { return None };
    let file_path = file_path.to_string();
    let status = obj_ref
        .get(&Arc::from("status"))
        .and_then(|v| match v { Value::Number(n) => Some(*n as u16), _ => None })
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

fn send_file_response(
    request: tiny_http::Request,
    status: u16,
    headers: Vec<(String, String)>,
    file_path: String,
) {
    let file = match File::open(&file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open file {}: {}", file_path, e);
            let fallback = tiny_http::Response::from_string(format!("File not found: {}", file_path))
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

pub fn send_response(
    request: tiny_http::Request,
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
) {
    let status_code = tiny_http::StatusCode(status);
    let mut response = tiny_http::Response::from_string(body).with_status_code(status_code);

    for (key, value) in headers {
        if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }

    let _ = request.respond(response);
}
