//! HTTP server for the Tish interpreter. Client `fetch` uses `tishlang_runtime` from eval.

use crate::value::{PropMap, Value};
use std::sync::Arc;

use tokio::runtime::Runtime;

thread_local! {
    pub(crate) static RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
}

/// Create an HTTP server that listens on the given port.
pub fn create_server(port: u16) -> Result<tiny_http::Server, String> {
    let addr = format!("0.0.0.0:{}", port);
    tiny_http::Server::http(&addr).map_err(|e| format!("Failed to start server: {}", e))
}

/// Convert a tiny_http::Request into a Tish Value object.
pub fn request_to_value(request: &mut tiny_http::Request) -> Value {
    let mut obj: PropMap = PropMap::with_capacity(6);

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

    let mut headers_obj: PropMap = PropMap::with_capacity(request.headers().len());
    for header in request.headers() {
        headers_obj.insert(
            Arc::from(header.field.as_str().as_str()),
            Value::String(header.value.as_str().into()),
        );
    }
    obj.insert(
        Arc::from("headers"),
        Value::Object(std::rc::Rc::new(std::cell::RefCell::new(headers_obj))),
    );

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    obj.insert(Arc::from("body"), Value::String(body.into()));

    Value::Object(std::rc::Rc::new(std::cell::RefCell::new(obj)))
}

/// Extract response data from a Tish Value object.
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
