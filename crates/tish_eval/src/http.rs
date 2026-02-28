//! HTTP support for the Tish interpreter.
//! Uses async reqwest with multi-threaded tokio runtime for client.
//! Uses tiny_http for synchronous HTTP server.

use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::Runtime;

thread_local! {
    static RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
}

/// Perform HTTP fetch - async internally, sync API externally
pub fn fetch(args: &[Value]) -> Result<Value, String> {
    let url = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_string(),
        None => return Err("fetch requires a URL".to_string()),
    };

    let options = args.get(1).cloned();

    RUNTIME.with(|rt| rt.block_on(fetch_async(&url, options.as_ref())))
}

/// Perform multiple HTTP fetches in parallel
pub fn fetch_all(args: &[Value]) -> Result<Value, String> {
    let requests = match args.first() {
        Some(Value::Array(arr)) => arr.borrow().clone(),
        _ => return Err("fetchAll requires an array of request objects".to_string()),
    };

    RUNTIME.with(|rt| rt.block_on(fetch_all_async(requests)))
}

async fn fetch_async(url: &str, options: Option<&Value>) -> Result<Value, String> {
    fetch_async_owned(url.to_string(), options.cloned()).await
}

async fn fetch_async_owned(url: String, options: Option<Value>) -> Result<Value, String> {
    let client = reqwest::Client::new();

    let method = extract_method(options.as_ref());
    let headers = extract_headers(options.as_ref());
    let body = extract_body(options.as_ref());

    let mut req = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        "HEAD" => client.head(&url),
        _ => client.get(&url),
    };

    for (key, value) in headers {
        req = req.header(key, value);
    }

    if let Some(body) = body {
        req = req.body(body);
    }

    let response = req.send().await.map_err(|e| e.to_string())?;

    let status = response.status().as_u16() as f64;
    let ok = response.status().is_success();
    let response_headers = response.headers().clone();
    let body_text = response.text().await.map_err(|e| e.to_string())?;

    Ok(build_response_object(status, ok, &response_headers, body_text))
}

async fn fetch_all_async(requests: Vec<Value>) -> Result<Value, String> {
    let mut futures = Vec::new();

    for req in requests {
        let (url, options) = match &req {
            Value::String(s) => (s.to_string(), None),
            Value::Object(obj) => {
                let obj_ref = obj.borrow();
                let url = obj_ref
                    .get(&Arc::from("url"))
                    .map(|v| v.to_string())
                    .ok_or("Each request object must have a 'url' property")?;
                (url, Some(req.clone()))
            }
            _ => return Err("Each request must be a string URL or request object".to_string()),
        };

        futures.push(fetch_async_owned(url, options));
    }

    let results = futures::future::join_all(futures).await;

    let response_values: Vec<Value> = results
        .into_iter()
        .map(|r| match r {
            Ok(v) => v,
            Err(e) => {
                let mut obj = HashMap::with_capacity(2);
                obj.insert(Arc::from("error"), Value::String(e.into()));
                obj.insert(Arc::from("ok"), Value::Bool(false));
                Value::Object(Rc::new(RefCell::new(obj)))
            }
        })
        .collect();

    Ok(Value::Array(Rc::new(RefCell::new(response_values))))
}

fn extract_method(options: Option<&Value>) -> String {
    options
        .and_then(|v| match v {
            Value::Object(obj) => obj.borrow().get(&Arc::from("method")).cloned(),
            _ => None,
        })
        .map(|v| v.to_string().to_uppercase())
        .unwrap_or_else(|| "GET".to_string())
}

fn extract_headers(options: Option<&Value>) -> Vec<(String, String)> {
    options
        .and_then(|v| match v {
            Value::Object(obj) => obj.borrow().get(&Arc::from("headers")).cloned(),
            _ => None,
        })
        .map(|v| match v {
            Value::Object(obj) => obj
                .borrow()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            _ => vec![],
        })
        .unwrap_or_default()
}

fn extract_body(options: Option<&Value>) -> Option<String> {
    options.and_then(|v| match v {
        Value::Object(obj) => obj.borrow().get(&Arc::from("body")).map(|v| v.to_string()),
        _ => None,
    })
}

fn build_response_object(
    status: f64,
    ok: bool,
    headers: &reqwest::header::HeaderMap,
    body: String,
) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(4);
    obj.insert(Arc::from("status"), Value::Number(status));
    obj.insert(Arc::from("ok"), Value::Bool(ok));
    obj.insert(Arc::from("body"), Value::String(body.into()));

    let mut headers_obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(headers.len());
    for (key, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            headers_obj.insert(Arc::from(key.as_str()), Value::String(v.into()));
        }
    }
    obj.insert(
        Arc::from("headers"),
        Value::Object(Rc::new(RefCell::new(headers_obj))),
    );

    Value::Object(Rc::new(RefCell::new(obj)))
}

/// Create an HTTP server that listens on the given port.
/// Returns a tiny_http::Server instance.
pub fn create_server(port: u16) -> Result<tiny_http::Server, String> {
    let addr = format!("0.0.0.0:{}", port);
    tiny_http::Server::http(&addr).map_err(|e| format!("Failed to start server: {}", e))
}

/// Convert a tiny_http::Request into a Tish Value object.
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

/// Extract response data from a Tish Value object.
/// Returns (status_code, headers, body).
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

            let body = obj_ref
                .get(&Arc::from("body"))
                .map(|v| v.to_string())
                .unwrap_or_default();

            let headers = obj_ref
                .get(&Arc::from("headers"))
                .and_then(|v| match v {
                    Value::Object(h) => Some(
                        h.borrow()
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
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

/// Send a response using tiny_http.
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
