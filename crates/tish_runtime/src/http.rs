//! HTTP support for compiled Tish programs.
//! Uses async reqwest with multi-threaded tokio runtime.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tish_core::Value;
use tokio::runtime::Runtime;

thread_local! {
    static RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
}

/// Perform HTTP fetch - async internally, sync API externally
pub fn fetch(args: &[Value]) -> Value {
    match fetch_impl(args) {
        Ok(v) => v,
        Err(e) => build_error_response(&e),
    }
}

/// Perform multiple HTTP fetches in parallel
pub fn fetch_all(args: &[Value]) -> Value {
    match fetch_all_impl(args) {
        Ok(v) => v,
        Err(e) => build_error_response(&e),
    }
}

fn fetch_impl(args: &[Value]) -> Result<Value, String> {
    let url = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => return Err("fetch requires a URL".to_string()),
    };

    let options = args.get(1).cloned();

    RUNTIME.with(|rt| rt.block_on(fetch_async(&url, options.as_ref())))
}

fn fetch_all_impl(args: &[Value]) -> Result<Value, String> {
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
                    .map(|v| v.to_display_string())
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
            Err(e) => build_error_response(&e),
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
        .map(|v| v.to_display_string().to_uppercase())
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
                .map(|(k, v)| (k.to_string(), v.to_display_string()))
                .collect(),
            _ => vec![],
        })
        .unwrap_or_default()
}

fn extract_body(options: Option<&Value>) -> Option<String> {
    options.and_then(|v| match v {
        Value::Object(obj) => obj
            .borrow()
            .get(&Arc::from("body"))
            .map(|v| v.to_display_string()),
        _ => None,
    })
}

fn build_response_object(
    status: f64,
    ok: bool,
    headers: &reqwest::header::HeaderMap,
    body: String,
) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::new();
    obj.insert(Arc::from("status"), Value::Number(status));
    obj.insert(Arc::from("ok"), Value::Bool(ok));
    obj.insert(Arc::from("body"), Value::String(body.into()));

    let mut headers_obj: HashMap<Arc<str>, Value> = HashMap::new();
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

fn build_error_response(error: &str) -> Value {
    let mut obj: HashMap<Arc<str>, Value> = HashMap::new();
    obj.insert(Arc::from("error"), Value::String(error.into()));
    obj.insert(Arc::from("ok"), Value::Bool(false));
    Value::Object(Rc::new(RefCell::new(obj)))
}
