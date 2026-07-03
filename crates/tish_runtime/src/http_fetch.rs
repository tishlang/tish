//! Web Fetch–aligned Response, ReadableStream, reader.read(), text()/json().

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tishlang_core::VmRef;

use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use tishlang_core::{NativeFn, ObjectMap, TishOpaque, TishPromise, Value};

use crate::http::{build_error_response, extract_body, extract_headers, extract_method};

// --- Promises (Send payloads only; Value built on awaiting thread) ---

struct FetchResponsePromise {
    rx: Mutex<Option<tokio::sync::oneshot::Receiver<Result<reqwest::Response, String>>>>,
}

impl TishPromise for FetchResponsePromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let r = crate::http::block_on_http(rx);
            match r {
                Ok(Ok(resp)) => Ok(response_value_from_reqwest(resp)),
                Ok(Err(e)) => Ok(build_error_response(&e)),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

struct FetchAllResponsesPromise {
    #[allow(clippy::type_complexity)]
    rx: Mutex<
        Option<
            tokio::sync::oneshot::Receiver<Result<Vec<Result<reqwest::Response, String>>, String>>,
        >,
    >,
}

impl TishPromise for FetchAllResponsesPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let r = crate::http::block_on_http(rx);
            match r {
                Ok(Ok(vec)) => {
                    let out: Vec<Value> = vec
                        .into_iter()
                        .map(|x| {
                            x.map(response_value_from_reqwest)
                                .unwrap_or_else(|e| build_error_response(&e))
                        })
                        .collect();
                    Ok(Value::Array(VmRef::new(out)))
                }
                Ok(Err(e)) => Ok(build_error_response(&e)),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

enum ReadChunk {
    Done,
    Bytes(Vec<u8>),
}

struct ReadChunkPromise {
    rx: Mutex<Option<tokio::sync::oneshot::Receiver<Result<ReadChunk, String>>>>,
}

impl TishPromise for ReadChunkPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let r = crate::http::block_on_http(rx);
            match r {
                Ok(Ok(ReadChunk::Done)) => {
                    let mut o = ObjectMap::default();
                    o.insert(Arc::from("done"), Value::Bool(true));
                    o.insert(Arc::from("value"), Value::Null);
                    Ok(Value::object(o))
                }
                Ok(Ok(ReadChunk::Bytes(b))) => {
                    let arr: Vec<Value> = b.iter().map(|u| Value::Number(*u as f64)).collect();
                    let mut o = ObjectMap::default();
                    o.insert(Arc::from("done"), Value::Bool(false));
                    o.insert(Arc::from("value"), Value::Array(VmRef::new(arr)));
                    Ok(Value::object(o))
                }
                Ok(Err(e)) => Err({
                    let mut obj = ObjectMap::default();
                    obj.insert(Arc::from("error"), Value::String(e.into()));
                    Value::object(obj)
                }),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

struct JsonTextPromise {
    rx: Mutex<Option<tokio::sync::oneshot::Receiver<Result<String, String>>>>,
}

impl TishPromise for JsonTextPromise {
    fn block_until_settled(&self) -> std::result::Result<Value, Value> {
        let rx = self.rx.lock().unwrap().take();
        if let Some(rx) = rx {
            let r = crate::http::block_on_http(rx);
            match r {
                Ok(Ok(s)) => match tishlang_core::json_parse(&s) {
                    Ok(v) => Ok(v),
                    Err(e) => Err({
                        let mut obj = ObjectMap::default();
                        obj.insert(Arc::from("error"), Value::String(e.into()));
                        Value::object(obj)
                    }),
                },
                Ok(Err(e)) => Err({
                    let mut obj = ObjectMap::default();
                    obj.insert(Arc::from("error"), Value::String(e.into()));
                    Value::object(obj)
                }),
                Err(_) => Err(Value::String("Promise dropped".into())),
            }
        } else {
            Err(Value::String("Promise already consumed".into()))
        }
    }
}

// --- Body ---

pub struct HttpBody {
    state: Mutex<BodyState>,
}

enum BodyState {
    Fresh(Option<reqwest::Response>),
    ReadInProgress,
    Gone,
}

impl HttpBody {
    pub fn new(response: reqwest::Response) -> Self {
        Self {
            state: Mutex::new(BodyState::Fresh(Some(response))),
        }
    }

    #[allow(clippy::type_complexity)]
    fn take_stream(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>, String> {
        let mut g = self.state.lock().unwrap();
        match &mut *g {
            BodyState::Fresh(r) => {
                let resp = r
                    .take()
                    .ok_or_else(|| "Response body already consumed".to_string())?;
                *g = BodyState::ReadInProgress;
                Ok(Box::pin(resp.bytes_stream()))
            }
            BodyState::ReadInProgress => {
                Err("ReadableStream is locked; getReader() already called".into())
            }
            BodyState::Gone => Err("Response body already consumed".into()),
        }
    }

    pub fn take_text_async(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>
    {
        let resp = {
            let mut g = self.state.lock().unwrap();
            match &mut *g {
                BodyState::Fresh(r) => match r.take() {
                    Some(resp) => {
                        *g = BodyState::Gone;
                        Ok(resp)
                    }
                    None => Err("Response body already consumed".into()),
                },
                BodyState::ReadInProgress => {
                    Err("Cannot call text(): body is locked by ReadableStreamDefaultReader".into())
                }
                BodyState::Gone => Err("Response body already consumed".into()),
            }
        };
        Box::pin(async move {
            match resp {
                Ok(r) => read_text_capped(r, fetch_max_body()).await,
                Err(e) => Err(e),
            }
        })
    }

    fn mark_gone_after_stream(&self) {
        let mut g = self.state.lock().unwrap();
        *g = BodyState::Gone;
    }
}

pub struct HttpReadableStream {
    body: Arc<HttpBody>,
}

impl TishOpaque for HttpReadableStream {
    fn type_name(&self) -> &'static str {
        "ReadableStream"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_method(&self, name: &str) -> Option<NativeFn> {
        if name != "getReader" {
            return None;
        }
        let body = Arc::clone(&self.body);
        Some(tishlang_core::native_fn(move |_args: &[Value]| match body.take_stream() {
            Ok(stream) => {
                let inner = Arc::new(tokio::sync::Mutex::new(StreamSlot { stream }));
                Value::Opaque(Arc::new(HttpStreamReader {
                    inner,
                    body: Arc::clone(&body),
                }))
            }
            Err(e) => {
                let mut m = ObjectMap::default();
                m.insert(Arc::from("error"), Value::String(e.into()));
                Value::object(m)
            }
        }))
    }
}

struct StreamSlot {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
}

pub struct HttpStreamReader {
    inner: Arc<tokio::sync::Mutex<StreamSlot>>,
    body: Arc<HttpBody>,
}

impl TishOpaque for HttpStreamReader {
    fn type_name(&self) -> &'static str {
        "ReadableStreamDefaultReader"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_method(&self, name: &str) -> Option<NativeFn> {
        if name != "read" {
            return None;
        }
        let inner = Arc::clone(&self.inner);
        let body = Arc::clone(&self.body);
        Some(tishlang_core::native_fn(move |_args: &[Value]| {
            let inner = Arc::clone(&inner);
            let body = Arc::clone(&body);
            let (tx, rx) = tokio::sync::oneshot::channel();
            crate::http::RUNTIME.with(|rt| {
                rt.spawn(async move {
                    let mut slot = inner.lock().await;
                    match slot.stream.next().await {
                        None => {
                            body.mark_gone_after_stream();
                            let _ = tx.send(Ok(ReadChunk::Done));
                        }
                        Some(Ok(b)) => {
                            let _ = tx.send(Ok(ReadChunk::Bytes(b.to_vec())));
                        }
                        Some(Err(e)) => {
                            let _ = tx.send(Err(e.to_string()));
                        }
                    }
                });
            });
            Value::Promise(Arc::new(ReadChunkPromise {
                rx: Mutex::new(Some(rx)),
            }))
        }))
    }
}

fn headers_to_value(headers: &reqwest::header::HeaderMap) -> Value {
    let mut headers_obj: ObjectMap = ObjectMap::with_capacity(headers.len());
    for (key, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            headers_obj.insert(Arc::from(key.as_str()), Value::String(v.into()));
        }
    }
    Value::object(headers_obj)
}

pub fn response_value_from_reqwest(response: reqwest::Response) -> Value {
    let status = response.status().as_u16() as f64;
    let ok = response.status().is_success();
    let headers_val = headers_to_value(response.headers());
    let body_holder = Arc::new(HttpBody::new(response));
    let stream = HttpReadableStream {
        body: Arc::clone(&body_holder),
    };
    let body_stream_val = Value::Opaque(Arc::new(stream));
    let bh_text = Arc::clone(&body_holder);
    let bh_json = Arc::clone(&body_holder);
    let text_fn: NativeFn = tishlang_core::native_fn(move |_args: &[Value]| {
        let bh = Arc::clone(&bh_text);
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::http::RUNTIME.with(|rt| {
            rt.spawn(async move {
                let r = bh.take_text_async().await;
                let _ = tx.send(r);
            });
        });
        crate::promise_io::string_result_promise(rx)
    });
    let json_fn: NativeFn = tishlang_core::native_fn(move |_args: &[Value]| {
        let bh = Arc::clone(&bh_json);
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::http::RUNTIME.with(|rt| {
            rt.spawn(async move {
                let r = bh.take_text_async().await;
                let _ = tx.send(r);
            });
        });
        Value::Promise(Arc::new(JsonTextPromise {
            rx: Mutex::new(Some(rx)),
        }))
    });
    let mut obj: ObjectMap = ObjectMap::default();
    obj.insert(Arc::from("status"), Value::Number(status));
    obj.insert(Arc::from("ok"), Value::Bool(ok));
    obj.insert(Arc::from("headers"), headers_val);
    obj.insert(Arc::from("body"), body_stream_val);
    obj.insert(Arc::from("text"), Value::Function(text_fn));
    obj.insert(Arc::from("json"), Value::Function(json_fn));
    Value::object(obj)
}

/// Shared, hardened outbound HTTP client: request + connect timeouts (a no-timeout fetch is
/// an outbound resource-pinning / slowloris vector) and a bounded redirect chain. Cached so
/// the connection pool is reused across requests.
/// NOTE: this does NOT yet block internal/metadata IPs — full SSRF defense (deny loopback /
/// link-local / RFC1918 after DNS resolution) is a follow-up that needs a policy decision.
fn fetch_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Maximum outbound-`fetch` response body we will buffer, in bytes. `reqwest::Response::text()`
/// reads the whole body with no bound, so a compromised or hostile upstream (or a
/// compression-amplified response) could OOM the worker (#383). Override with `TISH_FETCH_MAX_BODY`;
/// default 100 MiB — generous enough for normal API responses and moderate downloads, low enough to
/// stop a pathological body. Set it higher for legitimately large downloads.
fn fetch_max_body() -> usize {
    use std::sync::OnceLock;
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| parse_max_body(std::env::var("TISH_FETCH_MAX_BODY").ok()))
}

/// Pure `TISH_FETCH_MAX_BODY` parse: a valid byte count, else the 100 MiB default. Split out so it is
/// testable without touching process-global env / the `OnceLock` cache.
fn parse_max_body(env: Option<String>) -> usize {
    env.and_then(|v| v.parse().ok()).unwrap_or(100 * 1024 * 1024)
}

/// Read a response body to a `String`, streaming with a byte budget so it can never buffer more than
/// `max` bytes (#383). Rejects early on a `Content-Length` that already exceeds the cap, and again mid
/// stream for chunked/lying responses. Decodes UTF-8 lossily (matching `reqwest::text()` for the
/// UTF-8 responses that dominate `fetch`).
async fn read_text_capped(mut resp: reqwest::Response, max: usize) -> Result<String, String> {
    if let Some(len) = resp.content_length() {
        if len as usize > max {
            return Err(format!(
                "fetch: response body ({len} bytes, Content-Length) exceeds the {max}-byte cap (set TISH_FETCH_MAX_BODY to raise it)"
            ));
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > max {
            return Err(format!(
                "fetch: response body exceeds the {max}-byte cap (set TISH_FETCH_MAX_BODY to raise it)"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn send_request_parts(
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Result<reqwest::Response, String> {
    let client = fetch_client();
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
    req.send().await.map_err(|e| e.to_string())
}

pub fn fetch_promise_from_args(args: Vec<Value>) -> Value {
    let url = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(Err("fetch requires a URL".into()));
            return Value::Promise(Arc::new(FetchResponsePromise {
                rx: Mutex::new(Some(rx)),
            }));
        }
    };
    let method = extract_method(args.get(1));
    let headers = extract_headers(args.get(1));
    let body = extract_body(args.get(1));
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::http::RUNTIME.with(|rt| {
        rt.spawn(async move {
            let r = send_request_parts(url, method, headers, body).await;
            let _ = tx.send(r);
        });
    });
    Value::Promise(Arc::new(FetchResponsePromise {
        rx: Mutex::new(Some(rx)),
    }))
}

pub fn fetch_all_promise_from_args(args: Vec<Value>) -> Value {
    let requests = match args.first() {
        Some(Value::Array(arr)) => arr.borrow().clone(),
        _ => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(Err("fetchAll requires an array of request objects".into()));
            return Value::Promise(Arc::new(FetchAllResponsesPromise {
                rx: Mutex::new(Some(rx)),
            }));
        }
    };
    #[allow(clippy::type_complexity)]
    let mut parts: Vec<(String, String, Vec<(String, String)>, Option<String>)> = Vec::new();
    for req in requests {
        let (url, opt) = match &req {
            Value::String(s) => (s.to_string(), None),
            Value::Object(obj) => {
                let obj_ref = obj.borrow();
                match obj_ref
                    .strings
                    .get("url")
                    .map(|v| v.to_display_string())
                {
                    Some(u) => (u, Some(req.clone())),
                    None => {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let _ =
                            tx.send(Err("Each request object must have a 'url' property".into()));
                        return Value::Promise(Arc::new(FetchAllResponsesPromise {
                            rx: Mutex::new(Some(rx)),
                        }));
                    }
                }
            }
            _ => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(Err(
                    "Each request must be a string URL or request object".into()
                ));
                return Value::Promise(Arc::new(FetchAllResponsesPromise {
                    rx: Mutex::new(Some(rx)),
                }));
            }
        };
        let method = extract_method(opt.as_ref());
        let headers = extract_headers(opt.as_ref());
        let body = extract_body(opt.as_ref());
        parts.push((url, method, headers, body));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::http::RUNTIME.with(|rt| {
        rt.spawn(async move {
            let futs: Vec<_> = parts
                .into_iter()
                .map(|(url, m, h, b)| send_request_parts(url, m, h, b))
                .collect();
            let results = futures::future::join_all(futs).await;
            let mapped: Vec<Result<reqwest::Response, String>> = results.into_iter().collect();
            let _ = tx.send(Ok(mapped));
        });
    });
    Value::Promise(Arc::new(FetchAllResponsesPromise {
        rx: Mutex::new(Some(rx)),
    }))
}

#[cfg(test)]
mod fetch_cap_tests_383 {
    use super::parse_max_body;

    #[test]
    fn default_cap_is_100_mib() {
        assert_eq!(parse_max_body(None), 100 * 1024 * 1024);
        assert_eq!(parse_max_body(Some("not-a-number".into())), 100 * 1024 * 1024);
    }

    #[test]
    fn env_override_parses() {
        assert_eq!(parse_max_body(Some("1048576".into())), 1024 * 1024);
        assert_eq!(parse_max_body(Some("0".into())), 0);
    }
}
