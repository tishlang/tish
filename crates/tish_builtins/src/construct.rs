//! `new` lowering for non-JS targets: `construct(callee, args)` approximates JS `[[Construct]]`.
//! Browser-exact behavior remains on `tish build --target js`.

use std::sync::Arc;
use tishlang_core::{ObjectMap, Value, VmRef};

const CONSTRUCT: &str = "__construct";

/// Host `new`: `Object` with `__construct` (or, failing that, `__call`), `Function` as plain call,
/// else `Null`. The `__call` fallback matters because builtin objects like `Promise` expose their
/// constructor as `__call` (so `Promise(f)` works) but have no `__construct`; without this fallback
/// `new Promise((resolve, reject) => …)` returned `Null` and never ran the executor on the VM family,
/// while the interpreter (which routes `new` through the same callable) worked — a cross-backend
/// divergence on the most common promise idiom.
pub fn construct(callee: &Value, args: &[Value]) -> Value {
    match callee {
        Value::Function(f) => f.call(args),
        Value::Object(o) => {
            let b = o.borrow();
            if let Some(Value::Function(ctor)) = b.strings.get(CONSTRUCT) {
                let c = ctor.clone();
                drop(b);
                return c.call(args);
            }
            if let Some(Value::Function(call)) = b.strings.get("__call") {
                let c = call.clone();
                drop(b);
                return c.call(args);
            }
            Value::Null
        }
        _ => Value::Null,
    }
}

/// A JS-style error object `{ name, message }` (issue #60).
pub fn error_object(name: &str, message: &str) -> Value {
    let mut e = ObjectMap::default();
    e.insert(Arc::from("name"), Value::String(name.into()));
    e.insert(Arc::from("message"), Value::String(message.into()));
    Value::object(e)
}

fn make_error_from_args(name: &str, args: &[Value]) -> Value {
    let message = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    error_object(name, &message)
}

/// `Error(msg)` / `new Error(msg)` (and `TypeError` / `RangeError`) → `{ name, message }`
/// (issue #60). `__call` and `__construct` behave identically, matching JS where `Error(x)`
/// and `new Error(x)` produce the same object.
pub fn error_constructor_value(name: &'static str) -> Value {
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(CONSTRUCT),
        Value::native(move |args: &[Value]| make_error_from_args(name, args)),
    );
    m.insert(
        Arc::from("__call"),
        Value::native(move |args: &[Value]| make_error_from_args(name, args)),
    );
    Value::object(m)
}

fn param(initial: f64) -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("value"), Value::Number(initial));
    Value::object(m)
}

fn connect_fn() -> Value {
    Value::native(|_| Value::Null)
}

/// Shared audio-node shape: connect, gain, optional filter fields.
fn audio_node_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("gain"), param(0.0));
    m.insert(Arc::from("frequency"), param(440.0));
    m.insert(Arc::from("Q"), param(1.0));
    m.insert(Arc::from("type"), Value::String("peaking".into()));
    Value::object(m)
}

fn analyser_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("fftSize"), Value::Number(2048.0));
    Value::object(m)
}

fn stereo_panner_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("pan"), param(0.0));
    Value::object(m)
}

fn audio_buffer_stub(len: usize) -> Value {
    let n = len.max(1);
    let data = VmRef::new(vec![Value::Number(0.0); n]);
    let data2 = data.clone();
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from("getChannelData"),
        Value::native(move |_args| Value::Array(data2.clone())),
    );
    Value::object(m)
}

fn buffer_source_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("buffer"), Value::Null);
    m.insert(Arc::from("loop"), Value::Bool(false));
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("start"), Value::native(|_| Value::Null));
    m.insert(Arc::from("stop"), Value::native(|_| Value::Null));
    Value::object(m)
}

fn oscillator_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("frequency"), param(440.0));
    m.insert(Arc::from("type"), Value::String("sine".into()));
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("start"), Value::native(|_| Value::Null));
    m.insert(Arc::from("stop"), Value::native(|_| Value::Null));
    Value::object(m)
}

fn audio_context_instance() -> Value {
    let mut ctx = ObjectMap::default();
    ctx.insert(Arc::from("sampleRate"), Value::Number(48_000.0));
    ctx.insert(Arc::from("destination"), audio_node_stub());

    ctx.insert(
        Arc::from("createGain"),
        Value::native(|_| audio_node_stub()),
    );
    ctx.insert(
        Arc::from("createBiquadFilter"),
        Value::native(|_| audio_node_stub()),
    );
    ctx.insert(
        Arc::from("createStereoPanner"),
        Value::native(|_| stereo_panner_stub()),
    );
    ctx.insert(
        Arc::from("createAnalyser"),
        Value::native(|_| analyser_stub()),
    );
    ctx.insert(
        Arc::from("createBuffer"),
        Value::native(|args: &[Value]| {
            let len = args
                .get(1)
                .and_then(Value::as_number)
                .unwrap_or(0.0)
                .clamp(0.0, 1_000_000_000.0) as usize;
            audio_buffer_stub(len)
        }),
    );
    ctx.insert(
        Arc::from("createBufferSource"),
        Value::native(|_| buffer_source_stub()),
    );
    ctx.insert(
        Arc::from("createOscillator"),
        Value::native(|_| oscillator_stub()),
    );
    ctx.insert(Arc::from("decodeAudioData"), Value::native(|_| Value::Null));

    Value::object(ctx)
}

/// Global `AudioContext` for native/VM: stub graph (no real audio).
pub fn audio_context_constructor_value() -> Value {
    let ctor = Value::native(|_args: &[Value]| audio_context_instance());
    let mut m = ObjectMap::default();
    m.insert(Arc::from(CONSTRUCT), ctor);
    Value::object(m)
}
