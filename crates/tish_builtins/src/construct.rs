//! `new` lowering for non-JS targets: `construct(callee, args)` approximates JS `[[Construct]]`.
//! Browser-exact behavior remains on `tish build --target js`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use tishlang_core::{ObjectMap, Value};

const CONSTRUCT: &str = "__construct";

/// Host `new`: `Object` with `__construct`, `Function` as plain call, else `Null`.
pub fn construct(callee: &Value, args: &[Value]) -> Value {
    match callee {
        Value::Function(f) => f(args),
        Value::Object(o) => {
            let b = o.borrow();
            if let Some(Value::Function(ctor)) = b.get(&Arc::from(CONSTRUCT)) {
                let c = Rc::clone(ctor);
                drop(b);
                return c(args);
            }
            Value::Null
        }
        _ => Value::Null,
    }
}

fn param(initial: f64) -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("value"), Value::Number(initial));
    Value::Object(Rc::new(RefCell::new(m)))
}

fn connect_fn() -> Value {
    Value::Function(Rc::new(|_| Value::Null))
}

/// Shared audio-node shape: connect, gain, optional filter fields.
fn audio_node_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("gain"), param(0.0));
    m.insert(Arc::from("frequency"), param(440.0));
    m.insert(Arc::from("Q"), param(1.0));
    m.insert(Arc::from("type"), Value::String("peaking".into()));
    Value::Object(Rc::new(RefCell::new(m)))
}

fn analyser_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("fftSize"), Value::Number(2048.0));
    Value::Object(Rc::new(RefCell::new(m)))
}

fn stereo_panner_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(Arc::from("pan"), param(0.0));
    Value::Object(Rc::new(RefCell::new(m)))
}

fn audio_buffer_stub(len: usize) -> Value {
    let n = len.max(1);
    let data = Rc::new(RefCell::new(vec![Value::Number(0.0); n]));
    let data2 = Rc::clone(&data);
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from("getChannelData"),
        Value::Function(Rc::new(move |_args| Value::Array(Rc::clone(&data2)))),
    );
    Value::Object(Rc::new(RefCell::new(m)))
}

fn buffer_source_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("buffer"), Value::Null);
    m.insert(Arc::from("loop"), Value::Bool(false));
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(
        Arc::from("start"),
        Value::Function(Rc::new(|_| Value::Null)),
    );
    m.insert(
        Arc::from("stop"),
        Value::Function(Rc::new(|_| Value::Null)),
    );
    Value::Object(Rc::new(RefCell::new(m)))
}

fn oscillator_stub() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("frequency"), param(440.0));
    m.insert(Arc::from("type"), Value::String("sine".into()));
    m.insert(Arc::from("connect"), connect_fn());
    m.insert(
        Arc::from("start"),
        Value::Function(Rc::new(|_| Value::Null)),
    );
    m.insert(
        Arc::from("stop"),
        Value::Function(Rc::new(|_| Value::Null)),
    );
    Value::Object(Rc::new(RefCell::new(m)))
}

fn audio_context_instance() -> Value {
    let mut ctx = ObjectMap::default();
    ctx.insert(Arc::from("sampleRate"), Value::Number(48_000.0));
    ctx.insert(Arc::from("destination"), audio_node_stub());

    ctx.insert(
        Arc::from("createGain"),
        Value::Function(Rc::new(|_| audio_node_stub())),
    );
    ctx.insert(
        Arc::from("createBiquadFilter"),
        Value::Function(Rc::new(|_| audio_node_stub())),
    );
    ctx.insert(
        Arc::from("createStereoPanner"),
        Value::Function(Rc::new(|_| stereo_panner_stub())),
    );
    ctx.insert(
        Arc::from("createAnalyser"),
        Value::Function(Rc::new(|_| analyser_stub())),
    );
    ctx.insert(
        Arc::from("createBuffer"),
        Value::Function(Rc::new(|args: &[Value]| {
            let len = args
                .get(1)
                .and_then(Value::as_number)
                .unwrap_or(0.0)
                .clamp(0.0, 1_000_000_000.0) as usize;
            audio_buffer_stub(len)
        })),
    );
    ctx.insert(
        Arc::from("createBufferSource"),
        Value::Function(Rc::new(|_| buffer_source_stub())),
    );
    ctx.insert(
        Arc::from("createOscillator"),
        Value::Function(Rc::new(|_| oscillator_stub())),
    );
    ctx.insert(
        Arc::from("decodeAudioData"),
        Value::Function(Rc::new(|_| Value::Null)),
    );

    Value::Object(Rc::new(RefCell::new(ctx)))
}

/// Global `Uint8Array` for native/VM: `new Uint8Array(n)` → numeric array of zeros (not real bytes).
pub fn uint8_array_constructor_value() -> Value {
    let ctor = Rc::new(|args: &[Value]| {
        let len = args
            .first()
            .and_then(Value::as_number)
            .unwrap_or(0.0)
            .clamp(0.0, 1_000_000_000.0) as usize;
        Value::Array(Rc::new(RefCell::new(vec![Value::Number(0.0); len])))
    });
    let mut m = ObjectMap::default();
    m.insert(Arc::from(CONSTRUCT), Value::Function(ctor));
    Value::Object(Rc::new(RefCell::new(m)))
}

/// Global `AudioContext` for native/VM: stub graph (no real audio).
pub fn audio_context_constructor_value() -> Value {
    let ctor = Rc::new(|_args: &[Value]| audio_context_instance());
    let mut m = ObjectMap::default();
    m.insert(Arc::from(CONSTRUCT), Value::Function(ctor));
    Value::Object(Rc::new(RefCell::new(m)))
}
