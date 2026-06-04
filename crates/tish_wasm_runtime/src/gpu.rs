//! Browser WebGPU / JS-interop FFI for the tish bytecode VM.
//!
//! Lets a tish program compiled to wasm drive the browser's WebGPU (and any
//! Web API) without hand-binding each call. The bridge is a tiny set of
//! reflection-based primitives exposed as VM globals:
//!
//! - `js_global(name)` — read a JS global (e.g. `navigator`, `GPUBufferUsage`)
//! - `js_get(handle, key)` / `js_set(handle, key, val)`
//! - `js_call(handle, method, argsArray)` — call a method (the whole WebGPU
//!   command API is synchronous, so this covers it)
//! - `js_new(ctorNameOrHandle, argsArray)`
//! - `js_typeof(handle)` — debugging
//! - `f32a(arr)` / `u16a(arr)` / `u8a(arr)` / `u32a(arr)` — tish `number[]` → real typed array
//! - `request_animation_frame(cb)` — drive a render loop
//!
//! GPU/JS objects (device, queue, context, buffers, pipelines, textures,
//! ImageBitmaps, the host env object …) round-trip through the VM as opaque
//! [`JsHandle`] values. Async startup (requestAdapter/requestDevice/fetch/
//! createImageBitmap) is done in JS glue; the ready handles are handed to the
//! VM via the `host` global by [`start`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use tishlang_bytecode::deserialize;
use tishlang_core::{value_call, NativeFn, TishOpaque, Value};
use tishlang_vm::Vm;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

// ---------------------------------------------------------------------------
// Opaque JS handle
// ---------------------------------------------------------------------------

/// Opaque tish value wrapping a browser `JsValue`. `JsValue` is `!Send`, which
/// is why `TishOpaque`'s `Send + Sync` bound is gated off in the browser
/// (`!send-values`) build — see `tish_core/src/value.rs`.
struct JsHandle(JsValue);

impl TishOpaque for JsHandle {
    fn type_name(&self) -> &'static str {
        "JsHandle"
    }
    fn get_method(&self, _name: &str) -> Option<NativeFn> {
        None
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn wrap(v: JsValue) -> Value {
    Value::Opaque(Arc::new(JsHandle(v)))
}

fn unwrap_handle(v: &Value) -> Option<JsValue> {
    match v {
        Value::Opaque(o) => o.as_any().downcast_ref::<JsHandle>().map(|h| h.0.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Marshalling  tish Value <-> JsValue
// ---------------------------------------------------------------------------

/// Convert a tish value to a JS value. Objects/arrays recurse; an opaque
/// `JsHandle` is spliced in **by reference** (so descriptors can embed live GPU
/// handles, e.g. `beginRenderPass({ colorAttachments:[{ view:<handle> }] })`).
/// Functions/promises/symbols are not marshalled (return `null`).
fn value_to_js(v: &Value) -> JsValue {
    match v {
        Value::Number(n) => JsValue::from_f64(*n),
        Value::String(s) => JsValue::from_str(s.as_ref()),
        Value::Bool(b) => JsValue::from_bool(*b),
        Value::Null => JsValue::NULL,
        Value::Opaque(o) => match o.as_any().downcast_ref::<JsHandle>() {
            Some(h) => h.0.clone(),
            None => JsValue::NULL,
        },
        Value::Array(arr) => {
            let out = js_sys::Array::new();
            for item in arr.borrow().iter() {
                out.push(&value_to_js(item));
            }
            out.into()
        }
        Value::Object(obj) => {
            let out = js_sys::Object::new();
            let b = obj.borrow();
            for (k, val) in b.strings.iter() {
                let _ = js_sys::Reflect::set(
                    &out,
                    &JsValue::from_str(k.as_ref()),
                    &value_to_js(val),
                );
            }
            out.into()
        }
        _ => JsValue::NULL,
    }
}

/// Convert a JS value back to tish. Primitives map directly; everything else
/// (objects, functions, typed arrays, GPU objects) becomes an opaque handle —
/// we deliberately do **not** expand JS containers into tish arrays/objects
/// (that would re-introduce boxing and lose typed-array identity).
fn js_to_value(v: JsValue) -> Value {
    if v.is_null() || v.is_undefined() {
        Value::Null
    } else if let Some(n) = v.as_f64() {
        Value::Number(n)
    } else if let Some(b) = v.as_bool() {
        Value::Bool(b)
    } else if let Some(s) = v.as_string() {
        Value::String(s.into())
    } else {
        wrap(v)
    }
}

// ---------------------------------------------------------------------------
// FFI builtins
// ---------------------------------------------------------------------------

fn ffi_js_global() -> Value {
    Value::native(|args: &[Value]| {
        let name = match args.first() {
            Some(Value::String(s)) => s.clone(),
            _ => return Value::Null,
        };
        match js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(name.as_ref())) {
            Ok(v) => js_to_value(v),
            Err(_) => Value::Null,
        }
    })
}

fn ffi_js_get() -> Value {
    Value::native(|args: &[Value]| {
        let obj = match args.first().and_then(unwrap_handle) {
            Some(o) => o,
            None => return Value::Null,
        };
        let key = args.get(1).map(value_to_js).unwrap_or(JsValue::NULL);
        match js_sys::Reflect::get(&obj, &key) {
            Ok(v) => js_to_value(v),
            Err(_) => Value::Null,
        }
    })
}

fn ffi_js_set() -> Value {
    Value::native(|args: &[Value]| {
        let obj = match args.first().and_then(unwrap_handle) {
            Some(o) => o,
            None => return Value::Null,
        };
        let key = args.get(1).map(value_to_js).unwrap_or(JsValue::NULL);
        let val = args.get(2).map(value_to_js).unwrap_or(JsValue::NULL);
        let _ = js_sys::Reflect::set(&obj, &key, &val);
        Value::Null
    })
}

fn ffi_js_call() -> Value {
    Value::native(|args: &[Value]| {
        let obj = match args.first().and_then(unwrap_handle) {
            Some(o) => o,
            None => return Value::Null,
        };
        let method = match args.get(1) {
            Some(Value::String(s)) => s.clone(),
            _ => return Value::Null,
        };
        let func = match js_sys::Reflect::get(&obj, &JsValue::from_str(method.as_ref())) {
            Ok(f) => match f.dyn_into::<js_sys::Function>() {
                Ok(f) => f,
                Err(_) => return Value::Null,
            },
            Err(_) => return Value::Null,
        };
        let js_args = js_sys::Array::new();
        if let Some(Value::Array(a)) = args.get(2) {
            for item in a.borrow().iter() {
                js_args.push(&value_to_js(item));
            }
        }
        match js_sys::Reflect::apply(&func, &obj, &js_args) {
            Ok(v) => js_to_value(v),
            Err(_) => Value::Null,
        }
    })
}

fn ffi_js_new() -> Value {
    Value::native(|args: &[Value]| {
        let ctor: JsValue = match args.first() {
            Some(Value::String(s)) => {
                match js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(s.as_ref())) {
                    Ok(v) => v,
                    Err(_) => return Value::Null,
                }
            }
            Some(v @ Value::Opaque(_)) => match unwrap_handle(v) {
                Some(h) => h,
                None => return Value::Null,
            },
            _ => return Value::Null,
        };
        let ctor_fn = match ctor.dyn_into::<js_sys::Function>() {
            Ok(f) => f,
            Err(_) => return Value::Null,
        };
        let js_args = js_sys::Array::new();
        if let Some(Value::Array(a)) = args.get(1) {
            for item in a.borrow().iter() {
                js_args.push(&value_to_js(item));
            }
        }
        match js_sys::Reflect::construct(&ctor_fn, &js_args) {
            Ok(v) => js_to_value(v),
            Err(_) => Value::Null,
        }
    })
}

fn ffi_js_typeof() -> Value {
    Value::native(|args: &[Value]| {
        let v = args.first().map(value_to_js).unwrap_or(JsValue::NULL);
        match v.js_typeof().as_string() {
            Some(s) => Value::String(s.into()),
            None => Value::Null,
        }
    })
}

/// `f32a(numberArray)` -> opaque `Float32Array` handle (one-shot copy). Use for
/// per-frame uniform/transform staging. Large static buffers should instead be
/// materialised host-side and passed in opaque (never boxed into a tish array).
fn ffi_f32a() -> Value {
    Value::native(|args: &[Value]| {
        let arr = match args.first() {
            Some(Value::Array(a)) => a.clone(),
            _ => return Value::Null,
        };
        let b = arr.borrow();
        let ta = js_sys::Float32Array::new_with_length(b.len() as u32);
        for (i, v) in b.iter().enumerate() {
            ta.set_index(i as u32, v.as_number().unwrap_or(0.0) as f32);
        }
        wrap(ta.into())
    })
}

fn ffi_u16a() -> Value {
    Value::native(|args: &[Value]| {
        let arr = match args.first() {
            Some(Value::Array(a)) => a.clone(),
            _ => return Value::Null,
        };
        let b = arr.borrow();
        let ta = js_sys::Uint16Array::new_with_length(b.len() as u32);
        for (i, v) in b.iter().enumerate() {
            ta.set_index(i as u32, v.as_number().unwrap_or(0.0) as u16);
        }
        wrap(ta.into())
    })
}

fn ffi_u8a() -> Value {
    Value::native(|args: &[Value]| {
        let arr = match args.first() {
            Some(Value::Array(a)) => a.clone(),
            _ => return Value::Null,
        };
        let b = arr.borrow();
        let ta = js_sys::Uint8Array::new_with_length(b.len() as u32);
        for (i, v) in b.iter().enumerate() {
            ta.set_index(i as u32, v.as_number().unwrap_or(0.0) as u8);
        }
        wrap(ta.into())
    })
}

fn ffi_u32a() -> Value {
    Value::native(|args: &[Value]| {
        let arr = match args.first() {
            Some(Value::Array(a)) => a.clone(),
            _ => return Value::Null,
        };
        let b = arr.borrow();
        let ta = js_sys::Uint32Array::new_with_length(b.len() as u32);
        for (i, v) in b.iter().enumerate() {
            ta.set_index(i as u32, v.as_number().unwrap_or(0.0) as u32);
        }
        wrap(ta.into())
    })
}

// ---------------------------------------------------------------------------
// requestAnimationFrame render loop
// ---------------------------------------------------------------------------

thread_local! {
    static RAF_CALLBACK: RefCell<Option<Value>> = const { RefCell::new(None) };
    static RAF_CLOSURE: RefCell<Option<Closure<dyn FnMut(f64)>>> = const { RefCell::new(None) };
    // True while a frame is pending, so repeated request_animation_frame calls
    // within one frame don't compound into runaway scheduling.
    static RAF_SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

/// Browser-driven per-frame entry: invoke the stored tish callback, then
/// re-arm for the next frame. We re-schedule from Rust (rather than requiring
/// the tish callback to call `request_animation_frame` again each frame) so the
/// loop runs continuously as long as a callback is registered. `cancel`-ing the
/// loop = clearing `RAF_CALLBACK`. The `value_call` runs the frame closure to
/// completion; all WebGPU recording happens synchronously inside.
fn tick(ts: f64) {
    let cb = RAF_CALLBACK.with(|c| c.borrow().clone());
    // This frame's pending schedule is now consumed.
    RAF_SCHEDULED.with(|f| f.set(false));
    if let Some(cb) = cb {
        if matches!(cb, Value::Function(_)) {
            value_call(&cb, &[Value::Number(ts)]);
        }
        // Re-arm only if the callback is still registered (allows a future
        // cancel by clearing RAF_CALLBACK).
        if RAF_CALLBACK.with(|c| c.borrow().is_some()) {
            schedule_raf();
        }
    }
}

fn schedule_raf() {
    // Coalesce: at most one rAF in flight at a time.
    if RAF_SCHEDULED.with(|f| f.get()) {
        return;
    }
    RAF_SCHEDULED.with(|f| f.set(true));
    RAF_CLOSURE.with(|slot| {
        let mut s = slot.borrow_mut();
        if s.is_none() {
            *s = Some(Closure::wrap(Box::new(|ts: f64| tick(ts)) as Box<dyn FnMut(f64)>));
        }
        let g = js_sys::global();
        if let Ok(raf) = js_sys::Reflect::get(&g, &JsValue::from_str("requestAnimationFrame")) {
            if let Ok(raf_fn) = raf.dyn_into::<js_sys::Function>() {
                let _ = raf_fn.call1(&g, s.as_ref().unwrap().as_ref().unchecked_ref());
            }
        }
    });
}

fn ffi_request_animation_frame() -> Value {
    Value::native(|args: &[Value]| {
        if let Some(cb) = args.first() {
            RAF_CALLBACK.with(|c| *c.borrow_mut() = Some(cb.clone()));
        }
        schedule_raf();
        Value::Null
    })
}

// ---------------------------------------------------------------------------
// Install + entry point
// ---------------------------------------------------------------------------

fn install_ffi(vm: &mut Vm) {
    vm.set_global("js_global".into(), ffi_js_global());
    vm.set_global("js_get".into(), ffi_js_get());
    vm.set_global("js_set".into(), ffi_js_set());
    vm.set_global("js_call".into(), ffi_js_call());
    vm.set_global("js_new".into(), ffi_js_new());
    vm.set_global("js_typeof".into(), ffi_js_typeof());
    vm.set_global("f32a".into(), ffi_f32a());
    vm.set_global("u16a".into(), ffi_u16a());
    vm.set_global("u8a".into(), ffi_u8a());
    vm.set_global("u32a".into(), ffi_u32a());
    vm.set_global("request_animation_frame".into(), ffi_request_animation_frame());
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(s: &str);
}

fn set_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            console_error(&format!("tish wasm panic: {}", info));
        }));
    });
}

/// Browser entry: run a tish bytecode `chunk` with the JS-interop FFI installed
/// and the host environment object (device/queue/context/format/canvas/assets,
/// built by the page's async startup glue) exposed as the `host` global.
///
/// Returns after top-level tish runs; the `requestAnimationFrame` loop keeps
/// the captured globals alive via the stored callback, so the VM state persists
/// across frames even though this call returns.
/// Invoke the registered frame callback exactly once, without re-scheduling.
/// For driving frames deterministically from JS when `requestAnimationFrame` is
/// throttled (e.g. a hidden/offscreen preview tab) — verification & debugging.
#[wasm_bindgen]
pub fn tick_once(ts: f64) {
    let cb = RAF_CALLBACK.with(|c| c.borrow().clone());
    if let Some(cb) = cb {
        if matches!(cb, Value::Function(_)) {
            value_call(&cb, &[Value::Number(ts)]);
        }
    }
}

#[wasm_bindgen]
pub fn start(chunk: Vec<u8>, env: JsValue) -> Result<(), JsValue> {
    set_panic_hook();
    let chunk = deserialize(&chunk).map_err(|e| JsValue::from_str(&e))?;
    let mut vm = Vm::new();
    install_ffi(&mut vm);
    vm.set_global("host".into(), wrap(env));
    vm.run(&chunk).map_err(|e| JsValue::from_str(&e))?;
    Ok(())
}
