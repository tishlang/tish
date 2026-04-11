//! ReadableStream + reader.read() chunk boundaries over local HTTP.
#![cfg(feature = "http")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use tishlang_core::Value;
use tishlang_runtime::{await_promise, fetch_promise};

fn chunked_body_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().expect("accept");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = vec![0u8; 512];
    let _ = stream.read(&mut buf);

    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
    let _ = stream.write_all(b"3\r\nabc\r\n");
    let _ = stream.flush();
    thread::sleep(Duration::from_millis(40));
    let _ = stream.write_all(b"3\r\ndef\r\n0\r\n\r\n");
    let _ = stream.flush();
}

fn byte_array_to_vec(v: &Value) -> Vec<u8> {
    match v {
        Value::Array(a) => a
            .borrow()
            .iter()
            .filter_map(|x| match x {
                Value::Number(n) => Some(*n as u8),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

#[test]
fn fetch_readable_stream_read_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let th = thread::spawn(move || chunked_body_server(listener));

    let url = format!("http://127.0.0.1:{}/c", port);
    let prom = fetch_promise(vec![Value::String(url.into())]);
    let resp = await_promise(prom);

    let obj = match &resp {
        Value::Object(o) => o.borrow(),
        _ => panic!("expected object response, got {:?}", resp),
    };
    assert!(obj
        .get(&std::sync::Arc::from("ok"))
        .map(|v| matches!(v, Value::Bool(true)))
        .unwrap_or(false));

    let body = obj.get(&std::sync::Arc::from("body")).expect("body");
    let stream = match body {
        Value::Opaque(s) => s.as_ref(),
        _ => panic!("expected ReadableStream opaque"),
    };
    let reader_val = stream.get_method("getReader").expect("getReader")(&[]);
    let reader = match reader_val {
        Value::Opaque(r) => r,
        _ => panic!("expected reader opaque, got {:?}", reader_val),
    };

    let mut acc = Vec::new();
    loop {
        let read_p = reader.get_method("read").expect("read")(&[]);
        let chunk = await_promise(read_p);
        let (done, chunk_bytes) = match chunk {
            Value::Object(o) => {
                let m = o.borrow();
                let done = m
                    .get(&std::sync::Arc::from("done"))
                    .and_then(|v| match v {
                        Value::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(true);
                let bytes = m
                    .get(&std::sync::Arc::from("value"))
                    .map(|v| byte_array_to_vec(v))
                    .unwrap_or_default();
                (done, bytes)
            }
            _ => panic!("expected read result object"),
        };
        if done {
            break;
        }
        acc.extend(chunk_bytes);
    }

    th.join().expect("server");
    assert_eq!(acc, b"abcdef");
}
