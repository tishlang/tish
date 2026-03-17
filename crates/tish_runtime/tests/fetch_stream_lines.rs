//! fetchStreamLines invokes the callback per line as bytes arrive.
#![cfg(feature = "http")]

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use tish_core::Value;
use tish_runtime::http_fetch_stream_lines;

fn accept_and_stream_lines(listener: TcpListener) {
    let (mut stream, _) = listener.accept().expect("accept");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = vec![0u8; 2048];
    let _ = stream.read(&mut buf);

    let body = "a\nb\nc\nd\n";
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    // Deliver lines in separate TCP writes so the client can read incrementally.
    for part in ["a\n", "b\n", "c\n", "d\n"] {
        let _ = stream.write_all(part.as_bytes());
        let _ = stream.flush();
        thread::sleep(Duration::from_millis(35));
    }
}

#[test]
fn fetch_stream_lines_fires_per_line() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let th = thread::spawn(move || accept_and_stream_lines(listener));

    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen_cb = Rc::clone(&seen);
    let url = format!("http://127.0.0.1:{}/stream", port);

    let out = http_fetch_stream_lines(&[
        Value::String(url.into()),
        Value::Function(Rc::new(move |args: &[Value]| {
            let line = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                Some(v) => v.to_display_string(),
                None => String::new(),
            };
            seen_cb.borrow_mut().push(line);
            Value::Null
        })),
    ]);

    th.join().expect("server thread");

    let ok = match &out {
        Value::Object(o) => o
            .borrow()
            .get(&std::sync::Arc::from("ok"))
            .map(|v| matches!(v, Value::Bool(true)))
            .unwrap_or(false),
        _ => false,
    };
    assert!(ok, "expected ok response, got {:?}", out);
    assert_eq!(&*seen.borrow(), &["a", "b", "c", "d"]);
}

#[test]
fn fetch_stream_lines_http_error_no_callback() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = vec![0u8; 512];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(
            b"HTTP/1.1 418 I'm a teapot\r\nContent-Length: 5\r\nConnection: close\r\n\r\noops\n",
        );
    });

    let calls = Rc::new(RefCell::new(0usize));
    let calls_cb = Rc::clone(&calls);
    let url = format!("http://127.0.0.1:{}/", port);
    let _ = http_fetch_stream_lines(&[
        Value::String(url.into()),
        Value::Function(Rc::new(move |_args: &[Value]| {
            *calls_cb.borrow_mut() += 1;
            Value::Null
        })),
    ]);

    assert_eq!(*calls.borrow(), 0, "callback must not run on error status");
}
