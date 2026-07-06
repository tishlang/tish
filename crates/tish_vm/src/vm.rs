//! Stack-based bytecode VM.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(not(feature = "send-values"))]
use std::rc::Rc;
use tishlang_core::VmRef;

use tishlang_ast::{BinOp, UnaryOp};
use tishlang_builtins::array as arr_builtins;
use tishlang_builtins::construct as construct_builtin;
use tishlang_builtins::globals as globals_builtins;
use tishlang_builtins::math as math_builtins;
use tishlang_builtins::number as num_builtins;
use tishlang_builtins::string as str_builtins;
use tishlang_bytecode::{u8_to_binop, u8_to_unaryop, Chunk, Constant, Opcode, NO_REST_PARAM};
use tishlang_core::{
    merge_object_data, object_get, object_has, object_set, to_int32, to_uint32, NativeFn,
    ObjectData, ObjectMap, PropMap, Value,
};

/// Error string returned by `run_chunk`/`run_framed` to mean "a thrown value is parked in
/// [`VM_PENDING_THROW`]; keep unwinding toward an enclosing `catch`" (issue #60). The leading
/// control char makes it unmistakable for a real diagnostic. `Callable::call` returns a bare
/// `Value`, so the thrown *value* can't ride the `Result`; it travels in this thread-local
/// instead and is picked up at the next call site (or the top-level boundary).
const PENDING_THROW_SENTINEL: &str = "\u{1}__tish_pending_throw__";

// #303 — the VM shares the one pending-throw slot defined in `tishlang_core`, so the array builtins
// it calls (`tishlang_builtins::array`) can poll the same slot and so native + VM agree. These thin
// wrappers keep every VM call site unchanged. `core::set_pending_throw` is first-throw-wins, but the
// VM only sets (one site) after draining at a try boundary, so behavior is unchanged.
fn set_pending_throw(v: Value) {
    tishlang_core::set_pending_throw(v);
}
fn take_pending_throw() -> Option<Value> {
    tishlang_core::take_pending_throw()
}
fn pending_throw_is_set() -> bool {
    tishlang_core::has_pending_throw()
}

// The recursion ceiling + trip error moved to `tishlang_core` (#381) so the native backend's
// boxed-call guard shares one implementation with the VM. Same semantics: thread-local ceiling
// lazily read from `TISH_MAX_CALL_DEPTH` (default 20000), `{name, message}` RangeError.
use tishlang_core::{max_call_depth, stack_overflow_error};

/// Headroom (bytes) a self-recursive JIT'd function leaves below the real stack bottom before it
/// bails (#381). Must cover the bail path plus building the `RangeError` back in the VM; 256 KiB is
/// comfortably more than either needs while still far larger than a single f64-ABI recursion frame.
const RECUR_STACK_MARGIN: usize = 256 * 1024;

/// OSR back-edge count at which a hot top-level loop is first offered to the region JIT (#190). High
/// enough that the compile + first-attempt cost is amortized over a genuinely hot loop.
#[cfg(not(target_arch = "wasm32"))]
const OSR_THRESHOLD: u32 = 10_000;
/// After the first attempt, retry OSR every this-many back-edges — only relevant to the live-in-miss
/// path (a numeric loop compiles and consumes itself on the first attempt). Keeps the re-check off the
/// per-iteration hot path.
#[cfg(not(target_arch = "wasm32"))]
const OSR_RETRY: u32 = 50_000;

/// Outcome of an OSR attempt (#190).
#[cfg(not(target_arch = "wasm32"))]
enum OsrResult {
    /// Ran the loop natively; resume interpreting at this chunk `ip` (the loop exit).
    Compiled(usize),
    /// A live-in slot is non-numeric; keep interpreting (may become eligible later → retry).
    LiveInMiss,
    /// The region is not a pure-numeric slot loop; give up on this loop (negative-cached).
    NotCompilable,
}

/// Append the source location of the instruction at `off` to a runtime-error message, e.g.
/// `Cannot read property 'x' of null (at app.tish:4)` (issue #74). No-ops when the chunk
/// carries no line table (e.g. deserialized bytecode).
fn locate_error(chunk: &Chunk, off: usize, msg: &str) -> String {
    match chunk.line_at(off) {
        Some(line) => match &chunk.source {
            Some(src) => format!("{msg} (at {src}:{line})"),
            None => format!("{msg} (at line {line})"),
        },
        None => msg.to_string(),
    }
}

/// Wrap a closure in the right shared pointer for the current build.
/// Under `send-values` that's `Arc<dyn Fn + Send + Sync>`; otherwise it's
/// plain `Rc<dyn Fn>`. Call sites can stay ignorant of the distinction.
#[cfg(feature = "send-values")]
#[inline]
fn make_native_fn<F>(f: F) -> NativeFn
where
    F: Fn(&[Value]) -> Value + Send + Sync + 'static,
{
    tishlang_core::native_fn(f)
}

#[cfg(not(feature = "send-values"))]
#[inline]
fn make_native_fn<F>(f: F) -> NativeFn
where
    F: Fn(&[Value]) -> Value + 'static,
{
    tishlang_core::native_fn(f)
}

// Array / string / object methods have the same shape as `NativeFn`, which
// is already feature-gated (`Rc<dyn Fn>` vs `Arc<dyn Fn + Send + Sync>`).
// Alias to that so the VM picks the right pointer type automatically.
type ArrayMethodFn = NativeFn;

/// Feature names enabled for this VM run (`tish run --feature …`). `full` enables every optional capability.
#[cfg_attr(
    not(any(
        feature = "fs",
        feature = "http",
        feature = "promise",
        feature = "timers",
        feature = "process",
        feature = "ws"
    )),
    allow(dead_code)
)]
#[inline]
fn value_object_from_map(m: ObjectMap) -> Value {
    Value::Object(VmRef::new(ObjectData::from_strings(m)))
}

#[cfg(any(
    feature = "fs",
    feature = "http",
    feature = "promise",
    feature = "timers",
    feature = "process",
    feature = "ws"
))]
#[inline]
fn cap_allows(enabled: &HashSet<String>, name: &str) -> bool {
    enabled.contains("full") || enabled.contains(name)
}

/// Capabilities linked into this `tishlang_vm` binary (compile-time). Used by [`Vm::new`] and `run()`.
pub fn all_compiled_capabilities() -> HashSet<String> {
    #[allow(unused_mut)]
    let mut s = HashSet::new();
    #[cfg(feature = "http")]
    s.insert("http".to_string());
    #[cfg(feature = "promise")]
    s.insert("promise".to_string());
    #[cfg(feature = "timers")]
    s.insert("timers".to_string());
    #[cfg(feature = "fs")]
    s.insert("fs".to_string());
    #[cfg(feature = "process")]
    s.insert("process".to_string());
    #[cfg(feature = "regex")]
    s.insert("regex".to_string());
    #[cfg(feature = "ws")]
    s.insert("ws".to_string());
    #[cfg(feature = "tty")]
    s.insert("tty".to_string());
    s
}

/// Look up built-in module export for LoadNativeExport. Returns None if unknown or feature disabled.
#[cfg_attr(
    not(any(
        feature = "fs",
        feature = "http",
        feature = "promise",
        feature = "timers",
        feature = "process",
        feature = "ws",
        feature = "tty"
    )),
    allow(unused_variables)
)]
fn get_builtin_export(enabled: &HashSet<String>, spec: &str, export_name: &str) -> Option<Value> {
    #[cfg(feature = "fs")]
    if spec == "tish:fs" && cap_allows(enabled, "fs") {
        return match export_name {
            "readFile" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::read_file(args)
            })),
            "writeFile" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::write_file(args)
            })),
            "fileExists" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::file_exists(args)
            })),
            "isDir" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::is_dir(args)
            })),
            "readDir" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::read_dir(args)
            })),
            "mkdir" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::mkdir(args)
            })),
            "readFileBytes" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::read_file_bytes(args)
            })),
            _ => None,
        };
    }
    #[cfg(feature = "http")]
    if spec == "tish:http" && cap_allows(enabled, "http") {
        return match export_name {
            // Bytecode compiler lowers `await expr` to `tish:http.await(promise)` (see tish_bytecode compiler).
            "await" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::await_promise(args.first().cloned().unwrap_or(Value::Null))
            })),
            "fetch" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::fetch_promise(args.to_vec())
            })),
            "fetchAll" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::fetch_all_promise(args.to_vec())
            })),
            "Promise" => Some(tishlang_runtime::promise_object()),
            "serve" => Some(Value::native(|args: &[Value]| {
                // Phase-1 item 2: support `serve(port, { handler, onWorker })`
                // in addition to `serve(port, handler)`. When an options
                // object is given and onWorker is a function, invoke it with
                // worker id 0 and expect it to return the request handler.
                let raw = args.get(1).cloned().unwrap_or(Value::Null);
                let handler_value = match raw {
                    Value::Function(_) => raw,
                    Value::Object(ref obj) => {
                        let obj_ref = obj.borrow();
                        if let Some(Value::Function(on_worker)) = obj_ref
                            .strings
                            .get(&std::sync::Arc::from("onWorker"))
                            .cloned()
                        {
                            let args_for_init = [Value::Number(0.0)];
                            on_worker.call(&args_for_init)
                        } else if let Some(h) = obj_ref
                            .strings
                            .get(&std::sync::Arc::from("handler"))
                            .cloned()
                        {
                            h
                        } else {
                            Value::Null
                        }
                    }
                    _ => Value::Null,
                };
                if let Value::Function(f) = handler_value {
                    tishlang_runtime::http_serve(args, move |req_args| f.call(req_args))
                } else {
                    Value::Null
                }
            })),
            _ => None,
        };
    }
    #[cfg(all(feature = "promise", not(feature = "http")))]
    if spec == "tish:http" && cap_allows(enabled, "promise") {
        return match export_name {
            "Promise" => Some(tishlang_runtime::promise_object()),
            "await" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::await_promise(args.first().cloned().unwrap_or(Value::Null))
            })),
            _ => None,
        };
    }
    #[cfg(feature = "timers")]
    if spec == "tish:timers" && cap_allows(enabled, "timers") {
        return match export_name {
            "setTimeout" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::timer_set_timeout(args)
            })),
            "setInterval" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::timer_set_interval(args)
            })),
            "clearTimeout" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::timer_clear_timeout(args)
            })),
            "clearInterval" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::timer_clear_interval(args)
            })),
            _ => None,
        };
    }
    #[cfg(feature = "process")]
    if spec == "tish:process" && cap_allows(enabled, "process") {
        return match export_name {
            "exit" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::process_exit(args)
            })),
            "cwd" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::process_cwd(args)
            })),
            "exec" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::process_exec(args)
            })),
            "execFile" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::process_exec_file(args)
            })),
            "argv" => Some(Value::Array(VmRef::new(
                tishlang_core::process_argv()
                    .into_iter()
                    .map(|s| Value::String(s.into()))
                    .collect(),
            ))),
            "env" => Some(value_object_from_map(
                std::env::vars()
                    .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                    .collect(),
            )),
            "process" => {
                let mut m = ObjectMap::default();
                m.insert(
                    "exit".into(),
                    Value::native(|args: &[Value]| tishlang_runtime::process_exit(args)),
                );
                m.insert(
                    "cwd".into(),
                    Value::native(|args: &[Value]| tishlang_runtime::process_cwd(args)),
                );
                m.insert(
                    "exec".into(),
                    Value::native(|args: &[Value]| tishlang_runtime::process_exec(args)),
                );
                m.insert(
                    "execFile".into(),
                    Value::native(|args: &[Value]| tishlang_runtime::process_exec_file(args)),
                );
                m.insert(
                    "argv".into(),
                    Value::Array(VmRef::new(
                        tishlang_core::process_argv()
                            .into_iter()
                            .map(|s| Value::String(s.into()))
                            .collect(),
                    )),
                );
                m.insert(
                    "env".into(),
                    value_object_from_map(
                        std::env::vars()
                            .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                            .collect(),
                    ),
                );
                Some(value_object_from_map(m))
            }
            _ => None,
        };
    }
    #[cfg(feature = "ws")]
    if spec == "tish:ws" && cap_allows(enabled, "ws") {
        return match export_name {
            "WebSocket" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::web_socket_client(args)
            })),
            "Server" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::web_socket_server_construct(args)
            })),
            "wsSend" => Some(Value::native(|args: &[Value]| {
                Value::Bool(tishlang_runtime::ws_send_native(
                    args.first().unwrap_or(&Value::Null),
                    &args
                        .get(1)
                        .map(|v| v.to_display_string())
                        .unwrap_or_default(),
                ))
            })),
            "wsBroadcast" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::ws_broadcast_native(args)
            })),
            _ => None,
        };
    }
    #[cfg(feature = "tty")]
    if spec == "tish:tty" && cap_allows(enabled, "tty") {
        return match export_name {
            "size" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_size(args)
            })),
            "isTTY" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_is_tty(args)
            })),
            "setRawMode" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_set_raw_mode(args)
            })),
            "enterAltScreen" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_enter_alt_screen(args)
            })),
            "leaveAltScreen" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_leave_alt_screen(args)
            })),
            "read" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_read(args)
            })),
            "readLine" => Some(Value::native(|args: &[Value]| {
                tishlang_runtime::tty_read_line(args)
            })),
            _ => None,
        };
    }
    None
}

/// Console output: println! on native, web_sys::console on wasm
#[cfg(not(feature = "wasm"))]
fn vm_log(s: &str) {
    println!("{}", s);
}
#[cfg(not(feature = "wasm"))]
fn vm_log_err(s: &str) {
    eprintln!("{}", s);
}
#[cfg(feature = "wasm")]
fn vm_log(s: &str) {
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console)]
        fn log(s: &str);
    }
    log(s);
}
#[cfg(feature = "wasm")]
fn vm_log_err(s: &str) {
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console)]
        fn error(s: &str);
    }
    error(s);
}

/// Initialize default globals (console, Math, JSON, etc.)
#[allow(unused_variables)]
fn init_globals(enabled: &HashSet<String>) -> ObjectMap {
    let mut g = ObjectMap::default();

    let mut console = ObjectMap::default();
    console.insert(
        "debug".into(),
        Value::native(|args: &[Value]| {
            let s =
                tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors());
            vm_log(&s);
            Value::Null
        }),
    );
    console.insert(
        "log".into(),
        Value::native(|args: &[Value]| {
            let s =
                tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors());
            vm_log(&s);
            Value::Null
        }),
    );
    console.insert(
        "info".into(),
        Value::native(|args: &[Value]| {
            let s =
                tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors());
            vm_log(&s);
            Value::Null
        }),
    );
    console.insert(
        "warn".into(),
        Value::native(|args: &[Value]| {
            let s =
                tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors());
            vm_log_err(&s);
            Value::Null
        }),
    );
    console.insert(
        "error".into(),
        Value::native(|args: &[Value]| {
            let s =
                tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors());
            vm_log_err(&s);
            Value::Null
        }),
    );
    g.insert("console".into(), value_object_from_map(console));

    let mut math = ObjectMap::default();
    math.insert(
        "abs".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.abs())
        }),
    );
    math.insert(
        "sqrt".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.sqrt())
        }),
    );
    math.insert(
        "floor".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.floor())
        }),
    );
    math.insert(
        "ceil".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.ceil())
        }),
    );
    // round/min/max delegate to the shared builtins (JS round-half-to-+∞, empty → ±∞, NaN
    // propagation) so the vm never diverges from interp/native on Math semantics (#247).
    math.insert(
        "round".into(),
        Value::native(|args: &[Value]| math_builtins::round(args)),
    );
    math.insert(
        "random".into(),
        Value::native(|_| Value::Number(rand::random::<f64>())),
    );
    math.insert(
        "min".into(),
        Value::native(|args: &[Value]| math_builtins::min(args)),
    );
    math.insert(
        "max".into(),
        Value::native(|args: &[Value]| math_builtins::max(args)),
    );
    math.insert(
        "pow".into(),
        Value::native(|args: &[Value]| math_builtins::pow(args)),
    );
    // `imul` delegates to the shared builtin (ToInt32 + wrapping i32 multiply) so the vm never
    // diverges from interp/native (#247). Was previously absent from the vm `Math` object, so
    // `Math.imul(...)` threw "Call of non-function" on the bytecode VM.
    math.insert(
        "imul".into(),
        Value::native(|args: &[Value]| math_builtins::imul(args)),
    );
    math.insert(
        "sin".into(),
        Value::native(|args: &[Value]| math_builtins::sin(args)),
    );
    math.insert(
        "cos".into(),
        Value::native(|args: &[Value]| math_builtins::cos(args)),
    );
    math.insert(
        "tan".into(),
        Value::native(|args: &[Value]| math_builtins::tan(args)),
    );
    math.insert(
        "log".into(),
        Value::native(|args: &[Value]| math_builtins::log(args)),
    );
    math.insert(
        "exp".into(),
        Value::native(|args: &[Value]| math_builtins::exp(args)),
    );
    math.insert(
        "sign".into(),
        Value::native(|args: &[Value]| math_builtins::sign(args)),
    );
    math.insert(
        "trunc".into(),
        Value::native(|args: &[Value]| math_builtins::trunc(args)),
    );
    // Trig/hypot not covered by `math_builtins`; needed by the 3D engine's
    // camera + character-controller math (atan2/hypot) on the wasm VM, where
    // (unlike `--target js`) there is no host `Math` to fall through to.
    math.insert(
        "atan2".into(),
        Value::native(|args: &[Value]| {
            let y = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            let x = args.get(1).and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(y.atan2(x))
        }),
    );
    math.insert(
        "atan".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.atan())
        }),
    );
    math.insert(
        "asin".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.asin())
        }),
    );
    math.insert(
        "acos".into(),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.acos())
        }),
    );
    // hypot delegates to the shared builtin (variadic, NaN/Infinity-correct) so the vm never diverges
    // from interp/native (#247). The old inline `filter_map(as_number)` dropped non-number args
    // instead of propagating NaN.
    math.insert(
        "hypot".into(),
        Value::native(|args: &[Value]| math_builtins::hypot(args)),
    );
    // Hyperbolic, inverse-hyperbolic, cbrt and base-2/10 logs. Like the trig block above
    // these aren't in `math_builtins`, and on the wasm/native VM there is no host `Math`
    // to fall through to, so they previously returned `undefined` (issue #61).
    macro_rules! math_unary {
        ($name:literal, $method:ident) => {
            math.insert(
                $name.into(),
                Value::native(|args: &[Value]| {
                    let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
                    Value::Number(n.$method())
                }),
            );
        };
    }
    math_unary!("sinh", sinh);
    math_unary!("cosh", cosh);
    math_unary!("tanh", tanh);
    math_unary!("asinh", asinh);
    math_unary!("acosh", acosh);
    math_unary!("atanh", atanh);
    math_unary!("cbrt", cbrt);
    math_unary!("log2", log2);
    math_unary!("log10", log10);
    math.insert("PI".into(), Value::Number(std::f64::consts::PI));
    math.insert("E".into(), Value::Number(std::f64::consts::E));
    g.insert("Math".into(), value_object_from_map(math));

    let mut json = ObjectMap::default();
    json.insert(
        "parse".into(),
        Value::native(|args: &[Value]| {
            let s = args
                .first()
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            tishlang_core::json_parse(&s).unwrap_or(Value::Null)
        }),
    );
    json.insert(
        "stringify".into(),
        Value::native(|args: &[Value]| {
            let v = args.first().unwrap_or(&Value::Null);
            Value::String(tishlang_core::json_stringify(v).into())
        }),
    );
    g.insert("JSON".into(), value_object_from_map(json));

    g.insert(
        "parseInt".into(),
        Value::native(|args: &[Value]| globals_builtins::parse_int(args)),
    );
    g.insert(
        "parseFloat".into(),
        Value::native(|args: &[Value]| globals_builtins::parse_float(args)),
    );
    g.insert(
        "encodeURI".into(),
        Value::native(|args: &[Value]| globals_builtins::encode_uri(args)),
    );
    g.insert(
        "decodeURI".into(),
        Value::native(|args: &[Value]| globals_builtins::decode_uri(args)),
    );
    g.insert(
        "htmlEscape".into(),
        Value::native(|args: &[Value]| {
            tishlang_builtins::string::escape_html(args.first().unwrap_or(&Value::Null))
        }),
    );
    g.insert(
        "Boolean".into(),
        Value::native(|args: &[Value]| globals_builtins::boolean(args)),
    );
    g.insert(
        "isFinite".into(),
        Value::native(|args: &[Value]| globals_builtins::is_finite(args)),
    );
    g.insert(
        "isNaN".into(),
        Value::native(|args: &[Value]| globals_builtins::is_nan(args)),
    );
    g.insert("Infinity".into(), Value::Number(f64::INFINITY));
    g.insert("NaN".into(), Value::Number(f64::NAN));
    g.insert(
        "typeof".into(),
        Value::native(|args: &[Value]| {
            let v = args.first().unwrap_or(&Value::Null);
            Value::String(v.type_name().into())
        }),
    );
    g.insert("Symbol".into(), tishlang_builtins::symbol::symbol_object());

    // Date - full constructor (new Date(...)) plus statics now()/parse()/UTC().
    g.insert(
        "Date".into(),
        tishlang_builtins::date::date_constructor_value(),
    );
    g.insert(
        "Set".into(),
        tishlang_builtins::collections::set_constructor_value(),
    );
    g.insert(
        "Map".into(),
        tishlang_builtins::collections::map_constructor_value(),
    );

    for (name, ctor) in [
        (
            "Float64Array",
            tishlang_builtins::typedarrays::float64_array_constructor_value as fn() -> Value,
        ),
        (
            "Float32Array",
            tishlang_builtins::typedarrays::float32_array_constructor_value,
        ),
        (
            "Int8Array",
            tishlang_builtins::typedarrays::int8_array_constructor_value,
        ),
        (
            "Uint8Array",
            tishlang_builtins::typedarrays::uint8_array_constructor_value,
        ),
        (
            "Uint8ClampedArray",
            tishlang_builtins::typedarrays::uint8_clamped_array_constructor_value,
        ),
        (
            "Int16Array",
            tishlang_builtins::typedarrays::int16_array_constructor_value,
        ),
        (
            "Uint16Array",
            tishlang_builtins::typedarrays::uint16_array_constructor_value,
        ),
        (
            "Int32Array",
            tishlang_builtins::typedarrays::int32_array_constructor_value,
        ),
        (
            "Uint32Array",
            tishlang_builtins::typedarrays::uint32_array_constructor_value,
        ),
    ] {
        g.insert(name.into(), ctor());
    }
    g.insert(
        "AudioContext".into(),
        construct_builtin::audio_context_constructor_value(),
    );
    // Error constructors (issue #60): `new Error(msg)` / `Error(msg)` → `{ name, message }`.
    for name in ["Error", "TypeError", "RangeError", "SyntaxError"] {
        g.insert(
            name.into(),
            construct_builtin::error_constructor_value(name),
        );
    }

    // Object methods - delegate to tishlang_builtins::globals
    let mut object_methods = ObjectMap::default();
    object_methods.insert(
        "assign".into(),
        Value::native(|args: &[Value]| globals_builtins::object_assign(args)),
    );
    object_methods.insert(
        "is".into(),
        Value::native(|args: &[Value]| globals_builtins::object_is(args)),
    );
    object_methods.insert(
        "fromEntries".into(),
        Value::native(|args: &[Value]| globals_builtins::object_from_entries(args)),
    );
    object_methods.insert(
        "keys".into(),
        Value::native(|args: &[Value]| globals_builtins::object_keys(args)),
    );
    object_methods.insert(
        "values".into(),
        Value::native(|args: &[Value]| globals_builtins::object_values(args)),
    );
    object_methods.insert(
        "entries".into(),
        Value::native(|args: &[Value]| globals_builtins::object_entries(args)),
    );
    g.insert("Object".into(), value_object_from_map(object_methods));

    // Array.isArray + the `Array(n)` / `new Array(n)` constructor (issue #72). `__call`
    // serves both forms — `construct()` falls back to `__call` when there's no `__construct`.
    let mut array_static = ObjectMap::default();
    array_static.insert(
        "isArray".into(),
        Value::native(|args: &[Value]| globals_builtins::array_is_array(args)),
    );
    array_static.insert(
        "of".into(),
        Value::native(|args: &[Value]| globals_builtins::array_of(args)),
    );
    array_static.insert(
        Arc::from("__call"),
        Value::native(|args: &[Value]| construct_builtin::array_construct(args)),
    );
    g.insert("Array".into(), value_object_from_map(array_static));

    // String(value) as callable + String.fromCharCode
    let string_convert_fn = Value::native(|args: &[Value]| globals_builtins::string_convert(args));
    let mut string_static = ObjectMap::default();
    string_static.insert(
        "fromCharCode".into(),
        Value::native(|args: &[Value]| globals_builtins::string_from_char_code(args)),
    );
    string_static.insert(Arc::from("__call"), string_convert_fn);
    g.insert("String".into(), value_object_from_map(string_static));

    // Number(value) coercion as a callable global (issue #36) + the `Number.*` statics.
    let mut number_static = ObjectMap::default();
    number_static.insert(
        Arc::from("__call"),
        Value::native(|args: &[Value]| globals_builtins::number_convert(args)),
    );
    number_static.insert(Arc::from("isInteger"), Value::native(|a: &[Value]| globals_builtins::number_is_integer(a)));
    number_static.insert(Arc::from("isSafeInteger"), Value::native(|a: &[Value]| globals_builtins::number_is_safe_integer(a)));
    number_static.insert(Arc::from("isNaN"), Value::native(|a: &[Value]| globals_builtins::number_is_nan(a)));
    number_static.insert(Arc::from("isFinite"), Value::native(|a: &[Value]| globals_builtins::number_is_finite(a)));
    number_static.insert(Arc::from("parseInt"), Value::native(|a: &[Value]| globals_builtins::parse_int(a)));
    number_static.insert(Arc::from("parseFloat"), Value::native(|a: &[Value]| globals_builtins::parse_float(a)));
    number_static.insert(Arc::from("MAX_SAFE_INTEGER"), Value::Number(9_007_199_254_740_991.0));
    number_static.insert(Arc::from("MIN_SAFE_INTEGER"), Value::Number(-9_007_199_254_740_991.0));
    number_static.insert(Arc::from("EPSILON"), Value::Number(f64::EPSILON));
    number_static.insert(Arc::from("MAX_VALUE"), Value::Number(f64::MAX));
    number_static.insert(Arc::from("MIN_VALUE"), Value::Number(5e-324));
    number_static.insert(Arc::from("POSITIVE_INFINITY"), Value::Number(f64::INFINITY));
    number_static.insert(Arc::from("NEGATIVE_INFINITY"), Value::Number(f64::NEG_INFINITY));
    number_static.insert(Arc::from("NaN"), Value::Number(f64::NAN));
    g.insert("Number".into(), value_object_from_map(number_static));

    // JSX / Lattish: stubs for bytecode VM when no DOM (e.g. console). Override via set_global in browser.
    g.insert("h".into(), Value::native(|_args: &[Value]| Value::Null));
    g.insert(
        "Fragment".into(),
        value_object_from_map(ObjectMap::default()),
    );
    g.insert(
        "createRoot".into(),
        Value::native(|_args: &[Value]| {
            let mut render_obj = ObjectMap::default();
            render_obj.insert(
                "render".into(),
                Value::native(|_args: &[Value]| Value::Null),
            );
            value_object_from_map(render_obj)
        }),
    );
    g.insert(
        "useState".into(),
        Value::native(|args: &[Value]| {
            let init = args.first().cloned().unwrap_or(Value::Null);
            let arr = vec![init, Value::native(|_| Value::Null)];
            Value::Array(VmRef::new(arr))
        }),
    );
    let mut document_obj = ObjectMap::default();
    document_obj.insert("body".into(), Value::Null);
    g.insert("document".into(), value_object_from_map(document_obj));

    #[cfg(feature = "process")]
    if cap_allows(enabled, "process") {
        let mut process_obj = ObjectMap::default();
        process_obj.insert(
            "exit".into(),
            Value::native(|args: &[Value]| tishlang_runtime::process_exit(args)),
        );
        process_obj.insert(
            "cwd".into(),
            Value::native(|args: &[Value]| tishlang_runtime::process_cwd(args)),
        );
        process_obj.insert(
            "exec".into(),
            Value::native(|args: &[Value]| tishlang_runtime::process_exec(args)),
        );
        process_obj.insert(
            "execFile".into(),
            Value::native(|args: &[Value]| tishlang_runtime::process_exec_file(args)),
        );
        process_obj.insert(
            "argv".into(),
            Value::Array(VmRef::new(
                tishlang_core::process_argv()
                    .into_iter()
                    .map(|s| Value::String(s.into()))
                    .collect(),
            )),
        );
        process_obj.insert(
            "env".into(),
            value_object_from_map(
                std::env::vars()
                    .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                    .collect(),
            ),
        );
        g.insert("process".into(), value_object_from_map(process_obj));
    }

    #[cfg(feature = "timers")]
    if cap_allows(enabled, "timers") {
        g.insert(
            "setTimeout".into(),
            Value::native(|args: &[Value]| tishlang_runtime::timer_set_timeout(args)),
        );
        g.insert(
            "clearTimeout".into(),
            Value::native(|args: &[Value]| tishlang_runtime::timer_clear_timeout(args)),
        );
        g.insert(
            "setInterval".into(),
            Value::native(|args: &[Value]| tishlang_runtime::timer_set_interval(args)),
        );
        g.insert(
            "clearInterval".into(),
            Value::native(|args: &[Value]| tishlang_runtime::timer_clear_interval(args)),
        );
    }

    #[cfg(feature = "http")]
    if cap_allows(enabled, "http") {
        g.insert(
            "fetch".into(),
            Value::native(|args: &[Value]| tishlang_runtime::fetch_promise(args.to_vec())),
        );
        g.insert(
            "fetchAll".into(),
            Value::native(|args: &[Value]| tishlang_runtime::fetch_all_promise(args.to_vec())),
        );
        g.insert(
            "registerStaticRoute".into(),
            Value::native(|args: &[Value]| {
                let path = match args.first() {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Value::Null,
                };
                let body = match args.get(1) {
                    Some(Value::String(s)) => s.as_bytes().to_vec(),
                    _ => return Value::Null,
                };
                let ct = match args.get(2) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => "application/octet-stream".to_string(),
                };
                tishlang_runtime::register_static_route(&path, &body, &ct);
                Value::Null
            }),
        );
        g.insert(
            "serve".into(),
            Value::native(|args: &[Value]| {
                // Phase-1 item 2 (see tish:http.serve above for full docs).
                let raw = args.get(1).cloned().unwrap_or(Value::Null);
                let handler_value = match raw {
                    Value::Function(_) => raw,
                    Value::Object(ref obj) => {
                        let obj_ref = obj.borrow();
                        if let Some(Value::Function(on_worker)) = obj_ref
                            .strings
                            .get(&std::sync::Arc::from("onWorker"))
                            .cloned()
                        {
                            let args_for_init = [Value::Number(0.0)];
                            on_worker.call(&args_for_init)
                        } else if let Some(h) = obj_ref
                            .strings
                            .get(&std::sync::Arc::from("handler"))
                            .cloned()
                        {
                            h
                        } else {
                            Value::Null
                        }
                    }
                    _ => Value::Null,
                };
                if let Value::Function(f) = handler_value {
                    tishlang_runtime::http_serve(args, move |req_args| f.call(req_args))
                } else {
                    Value::Null
                }
            }),
        );
    }

    #[cfg(any(feature = "http", feature = "promise"))]
    if cap_allows(enabled, "http") || cap_allows(enabled, "promise") {
        g.insert("Promise".into(), tishlang_runtime::promise_object());
    }

    // `RegExp(pattern, flags)` constructor. A language feature (not a sandboxed capability),
    // so it's available whenever the `regex` feature is compiled — matching the interpreter.
    // Routes to the same `regexp_new` the rust backend uses (full-backend-parity-plan.md).
    #[cfg(feature = "regex")]
    g.insert(
        "RegExp".into(),
        Value::native(|args: &[Value]| tishlang_runtime::regexp_new(args)),
    );

    g
}

/// Shared scope for closure capture (parent frame's locals).
type ScopeMap = VmRef<ObjectMap>;

/// The captured lexical chain for closures. Shared immutably (never mutated after a closure is
/// built — `run_chunk` only reads it: `.len()`/`.iter()`/`.is_empty()`), so it lives behind an
/// `Rc`/`Arc` instead of a `Vec` that would be deep-cloned on every call. This makes the per-call
/// `enclosing` propagation a single refcount bump rather than a `Vec` allocation + N element clones
/// — a direct cut to function-call overhead. `Arc` under `send-values` (closures must be `Send`),
/// `Rc` otherwise.
#[cfg(feature = "send-values")]
type SharedChain = std::sync::Arc<Vec<ScopeMap>>;
#[cfg(not(feature = "send-values"))]
type SharedChain = std::rc::Rc<Vec<ScopeMap>>;

/// Options for the convenience [`run_with_options`] helper (one-shot VM run from the CLI).
#[derive(Clone, Debug, Default)]
pub struct VmRunOptions {
    /// When true and not inside a nested chunk (`enclosing` is `None`), top-level [`Opcode::DeclareVar`]
    /// also writes to globals so the REPL keeps bindings across input lines.
    pub repl_mode: bool,
    /// Enabled capabilities for this run (e.g. `fs`, `http`, `full`). Empty = none (secure default).
    pub capabilities: HashSet<String>,
}

pub struct Vm {
    stack: Vec<Value>,
    scope: ObjectMap,
    /// Captured enclosing scopes for closures, **innermost first**. A free variable resolves by
    /// walking `local_scope` → each entry here in order → `scope` → `globals`. This is the full
    /// lexical chain: a closure captures its defining frame's scope *plus that frame's own
    /// enclosing chain*, so a function nested N levels deep still sees every ancestor's locals
    /// (was a fixed `enclosing` + `enclosing2`, which silently lost captures >2 levels deep — see
    /// `nested_complex`). Per-iteration `let`: a fresh frozen overlay of the loop var(s) is
    /// prepended as the innermost entry, shadowing the still-shared frame scope that follows it,
    /// so the loop var is frozen per-iteration while everything else stays live. Empty at top level.
    /// Shared via `SharedChain` (Rc/Arc) so per-call propagation is a refcount bump, not a Vec clone.
    enclosing: SharedChain,
    globals: VmRef<ObjectMap>,
    /// Capabilities for `LoadNativeExport` and globals such as `process` / `serve`.
    capabilities: Arc<HashSet<String>>,
    /// Externally registered native modules, keyed by import spec (e.g.
    /// `"cargo:tish_pg"`). Populated by embedders before `run` (see
    /// [`register_native_module`]). Phase-2 item 11: unblocks `cargo:`
    /// imports on the cranelift and llvm backends which run this VM.
    native_modules: VmRef<HashMap<String, VmRef<ObjectMap>>>,
}

/// A bytecode-VM closure: a compiled chunk plus its captured lexical chain and shared VM state.
/// Implements [`tishlang_core::Callable`] so it lives in `Value::Function` like any callable, but
/// the `Call` opcode can `as_any`-downcast to it to run the call on the VM's explicit frame stack
/// (the frame-VM, task #39) instead of recursively re-entering `run_chunk`. `call()` is the
/// fallback path (builtin callbacks, and any not-yet-framed call) — byte-identical to the former
/// inline `Value::native` closure, so building these instead of raw closures changes nothing on
/// its own; the behavioural win comes when `Call` starts using the downcast + frame stack.
/// Try the array-mode JIT for `nf` (`array_param_mask != 0`). Splits `args` into numeric `f64`s and
/// flat [`crate::jit::ArrayHandle`]s — extracting all-numeric `Value::Array`s into scratch `Vec<f64>`s
/// that outlive the call. Returns `None` (caller falls back to the interpreter, so behaviour is always
/// correct) when an array arg is not an all-numeric `Value::Array` (covers `NumberArray`, whose
/// NaN-hole semantics differ), a numeric arg isn't a `Number`, or the JIT signals an OOB deopt.
#[cfg(not(target_arch = "wasm32"))]
fn try_call_array_jit(
    nf: &crate::jit::NumericFn,
    args: &[Value],
    arity: usize,
    mask: u8,
) -> Option<Value> {
    let writable = nf.array_writable_mask();
    let mut numeric: Vec<f64> = Vec::with_capacity(arity);
    // `scratch` OWNS the extracted f64 data; handles point into it. Build handles only AFTER scratch is
    // fully populated so its backing buffers never reallocate out from under a live pointer.
    let mut scratch: Vec<Vec<f64>> = Vec::new();
    let mut array_arg: Vec<usize> = Vec::new(); // the `args` index each scratch buffer came from
    let mut is_bool_arr: Vec<bool> = Vec::new(); // #187: parallel to `scratch` — was it a Bool array?
                                                 // #187: track whether ANY arg is a (non-empty) Bool array and whether ANY is a Number array. If both
                                                 // element types are present the JIT could launder a bool value read from one array into a number
                                                 // array (or vice versa) — the flattened `f64` writeback would then re-box it wrong — so we bail
                                                 // (below). queens (all-bool arrays) and spectral_norm (all-number) never mix, so they stay JIT'd.
    let mut any_bool_arr = false;
    let mut any_num_arr = false;
    #[allow(clippy::needless_range_loop)]
    // `i` drives bit-mask math (`mask >> i`), not just indexing
    for i in 0..arity {
        if (mask >> i) & 1 == 1 {
            match &args[i] {
                Value::Array(a) => {
                    // #187: a writable array must not alias another array arg — the JIT reads/writes a
                    // private scratch copy, so aliasing would make reads see stale data or a writeback
                    // silently drop the other param's writes. Bail (→ interpret) on any such overlap.
                    if writable != 0 {
                        for &j in &array_arg {
                            if let Value::Array(other) = &args[j] {
                                if VmRef::ptr_eq(a, other)
                                    && ((writable >> i) & 1 == 1 || (writable >> j) & 1 == 1)
                                {
                                    return None;
                                }
                            }
                        }
                    }
                    // #187: accept an all-`Number` OR an all-`Bool` array (bool → `f64` 0/1, e.g.
                    // queens' `cols`/`diag*`). A MIXED array is ambiguous for the typed writeback → bail.
                    let b = a.borrow();
                    let mut buf: Vec<f64> = Vec::with_capacity(b.len());
                    let mut seen_bool = false;
                    let mut seen_num = false;
                    for el in b.iter() {
                        match el {
                            Value::Number(n) => {
                                seen_num = true;
                                buf.push(*n);
                            }
                            Value::Bool(bl) => {
                                seen_bool = true;
                                buf.push(if *bl { 1.0 } else { 0.0 });
                            }
                            _ => return None, // non-numeric/non-bool element → interpreter
                        }
                    }
                    if seen_bool && seen_num {
                        return None; // mixed Bool/Number array → can't type the writeback
                    }
                    any_bool_arr |= seen_bool;
                    any_num_arr |= seen_num;
                    scratch.push(buf);
                    array_arg.push(i);
                    is_bool_arr.push(seen_bool);
                }
                _ => return None, // NumberArray / non-array → interpreter
            }
        } else {
            match &args[i] {
                Value::Number(n) => numeric.push(*n),
                _ => return None,
            }
        }
    }
    // #187: mixed Bool + Number array args in one call could launder a value across element types (a bool
    // read from one array stored into a number array, or vice versa) — the flat `f64` writeback can't
    // re-box that correctly. Bail when both element types are present among the args.
    if any_bool_arr && any_num_arr {
        return None;
    }
    // #187: the writeback re-boxes a writable array by its ENTRY element type (bool vs number, per
    // `is_bool_arr`), so the value kind the JIT actually stored must match that type. A function that
    // writes bool consts (`arr[i] = true`) handed a NUMBER array — or one that writes numbers handed a
    // BOOL array — would re-box to the wrong type (e.g. `arr[i] = true` on `[0,0,0]` boxing `Number(1)`
    // where the interpreter stores `Bool(true)`). Bail to the interpreter on any such mismatch. (A single
    // array with mixed bool + non-bool writes was already rejected at compile time in `classify_params`.)
    let bool_write_mask = nf.array_bool_write_mask();
    if writable != 0 {
        for (k, &entry_bool) in is_bool_arr.iter().enumerate() {
            let i = array_arg[k];
            if (writable >> i) & 1 == 1 && entry_bool != ((bool_write_mask >> i) & 1 == 1) {
                return None;
            }
        }
    }
    // `as_mut_ptr`: #187 writable params store through this pointer, so it must carry mut provenance
    // (read-only params only load — harmless). Handles are built only AFTER `scratch` is fully
    // populated so its backing buffers never reallocate out from under a live pointer.
    let handles: Vec<crate::jit::ArrayHandle> = scratch
        .iter_mut()
        .map(|buf| crate::jit::ArrayHandle {
            ptr: buf.as_mut_ptr(),
            len: buf.len(),
        })
        .collect();
    // #187: a self-recursive array-mode function (queens' `place`) recurses on the NATIVE stack, so
    // pass a RecurGuard (like #381) — its entry SP-bail turns unbounded recursion into a catchable
    // RangeError instead of a SIGSEGV. Non-recursive array fns pass a null guard (unchanged ABI).
    let (res, deopt, tripped) = if nf.recur_guarded() {
        let anchor = 0u8;
        let current_sp = &anchor as *const u8 as usize;
        let stack_limit = match stacker::remaining_stack() {
            Some(rem) => {
                let margin = RECUR_STACK_MARGIN.min(rem / 2);
                current_sp.saturating_sub(rem).saturating_add(margin)
            }
            None => 0,
        };
        let mut guard = crate::jit::RecurGuard {
            stack_limit,
            tripped: 0,
        };
        let (res, deopt) = nf.call_arrays_guarded(&numeric, &handles, &mut guard);
        (res, deopt, guard.tripped != 0)
    } else {
        let (res, deopt) = nf.call_arrays(&numeric, &handles);
        (res, deopt, false)
    };
    if deopt {
        return None; // OOB access → re-run interpreter (the discarded scratch writes never escaped)
    }
    // #187: copy each WRITTEN array param's (possibly-mutated) scratch back into its `Value::Array`. The
    // length is unchanged (`SetIndex` can't grow; an OOB write deopts above), so this is a same-length
    // element overwrite. On a normal return this reflects the final state; on a recursion-guard TRIP
    // (below) it reflects the partial in-place mutations made before the overflow — matching the
    // interpreter/node, which likewise leak whatever was mutated when a deep recursion throws.
    if writable != 0 {
        for (k, buf) in scratch.iter().enumerate() {
            let i = array_arg[k];
            if (writable >> i) & 1 == 1 {
                if let Value::Array(a) = &args[i] {
                    let bool_arr = is_bool_arr[k]; // #187: re-box a bool array's elements as Bool
                    let mut b = a.borrow_mut();
                    for (j, &v) in buf.iter().enumerate() {
                        if let Some(slot) = b.get_mut(j) {
                            *slot = if bool_arr {
                                Value::Bool(v != 0.0)
                            } else {
                                Value::Number(v)
                            };
                        }
                    }
                }
            }
        }
    }
    // #187: a self-recursive array-mode fn that overflowed the native stack raises the same catchable
    // RangeError as every other tier (#381). The writeback above already flushed the partial mutations,
    // so the caller's arrays reflect the same "leaked" state the interpreter/node leave on a deep throw.
    if tripped {
        set_pending_throw(stack_overflow_error());
        return Some(Value::Null);
    }
    // #187: a VOID function (side-effect writer, e.g. `multiplyAv`) returns the implicit `null` — its
    // `f64` result is a dummy. Return `Value::Null` to match the interpreter; the real effect is the
    // array writeback above.
    if nf.returns_void() {
        return Some(Value::Null);
    }
    Some(Value::Number(res))
}

/// #187 native HOF fusion. When `arr.map/filter/reduce/forEach(cb)` has a JIT-compiled pure-numeric
/// callback (`cb` is a [`VmClosure`] with a plain register-`f64` [`crate::jit::NumericFn`]) AND the
/// array is all-numeric, the whole higher-order call runs as a tight native loop calling the
/// `NumericFn` per element — skipping the per-element boxed `Callable::call` dispatch AND the
/// `Value` (un)boxing of each element that the generic `arr_builtins` path pays.
///
/// This is PURELY ADDITIVE: every helper returns `None` to fall back to the existing generic path
/// (which is byte-identical to the interpreter), so a bail is always sound. The gate is deliberately
/// narrow — see [`fused_numeric_fn`] and the per-helper arity/`result_bool` conditions.
#[cfg(not(target_arch = "wasm32"))]
mod hof_fusion {
    use super::VmClosure;
    use tishlang_core::{Value, VmRef};

    /// Extract the callback's fusable [`crate::jit::NumericFn`], if any. Fires ONLY for a `VmClosure`
    /// whose JIT is a plain pure-numeric register-`f64` function:
    ///   * `array_param_mask() == 0` — not an array-mode ABI (that path reads `arr[i]`, needs handles);
    ///   * `!is_jv()` — no function-local JIT arrays (those signal a deopt via a per-thread flag we'd
    ///     have to poll — the generic path handles them correctly, so bail);
    ///   * `!recur_guarded()` — not self-recursive (would need the `RecurGuard` trailing-ptr ABI; a
    ///     lambda callback is never self-recursive, so this only ever excludes pathological input).
    ///
    /// Any other callable (builtin native fn, non-JIT closure, array-mode/JV/recursive fn) → `None`.
    #[inline]
    pub(super) fn fused_numeric_fn(cb: &Value) -> Option<crate::jit::NumericFn> {
        let Value::Function(f) = cb else { return None };
        let vc = f.as_any().downcast_ref::<VmClosure>()?;
        let nf = vc.jit_fn?;
        if nf.array_param_mask() != 0 || nf.is_jv() || nf.recur_guarded() {
            return None;
        }
        Some(nf)
    }

    /// Snapshot an all-numeric array as a `Vec<f64>`. Accepts a packed [`Value::NumberArray`] and a
    /// boxed [`Value::Array`] whose every element is a [`Value::Number`]. Returns `None` for any other
    /// shape (mixed/boxed non-number element, a non-array) so the caller falls back to the generic
    /// path. Snapshots (clones) so — exactly like the generic `arr_builtins` path (#382) — a callback
    /// that re-enters the same array observes the pre-call contents and can never deadlock the borrow.
    #[inline]
    fn numeric_snapshot(arr: &Value) -> Option<Vec<f64>> {
        match arr {
            Value::NumberArray(a) => Some(a.borrow().clone()),
            Value::Array(a) => {
                let b = a.borrow();
                let mut out = Vec::with_capacity(b.len());
                for v in b.iter() {
                    match v {
                        Value::Number(n) => out.push(*n),
                        _ => return None,
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Box a `NumericFn` f64 result the SAME way the interpreter/generic path does: as `Value::Bool`
    /// when the callback's result is a comparison (`result_is_bool`), else `Value::Number`. A bool
    /// `NumericFn` returns exactly `0.0`/`1.0`, so `r != 0.0` is exact.
    #[inline]
    fn box_result(nf: &crate::jit::NumericFn, r: f64) -> Value {
        if nf.result_is_bool() {
            Value::Bool(r != 0.0)
        } else {
            Value::Number(r)
        }
    }

    /// Truthiness of a `NumericFn` result under tish's rules — matches `Value::is_truthy` on the
    /// boxed form: a number is truthy iff `!= 0` and not NaN; a bool-result fn returns `0.0`/`1.0`
    /// (never NaN) so the same test is exact for it too.
    #[inline]
    fn result_truthy(r: f64) -> bool {
        r != 0.0 && !r.is_nan()
    }

    /// Fused `map`. Fires when the callback is a fusable numeric fn of arity ≤ 2 (it may read the
    /// element and the index; a 3rd `array` param can't be a number, so arity 3+ bails). Returns the
    /// SAME `Value` variant the generic path would: a numeric-returning map over a `NumberArray` yields
    /// a packed `NumberArray` (empty → boxed empty `Array`, matching `packed_or_empty`); a
    /// bool-returning map, or any map over a boxed `Array`, yields a boxed `Value::Array`.
    pub(super) fn map(arr: &Value, cb: &Value) -> Option<Value> {
        let nf = fused_numeric_fn(cb)?;
        if nf.arity() > 2 {
            return None; // 3rd param is the array arg — not a number; bail (sound).
        }
        let data = numeric_snapshot(arr)?;
        let arity = nf.arity();
        let packed_input = matches!(arr, Value::NumberArray(_));
        // A numeric-returning map over a packed input keeps its result packed (byte-identical to
        // `packed_or_empty`); every other case (bool result, or a boxed `Array` input) boxes.
        if !nf.result_is_bool() && packed_input {
            let mut out: Vec<f64> = Vec::with_capacity(data.len());
            let mut args = [0.0f64; 2];
            for (i, &n) in data.iter().enumerate() {
                args[0] = n;
                args[1] = i as f64;
                out.push(nf.call(&args[..arity]));
            }
            // Empty → boxed empty `Array`, matching `packed_or_empty`.
            if out.is_empty() {
                return Some(Value::Array(VmRef::new(Vec::new())));
            }
            return Some(Value::number_array(out));
        }
        let mut out: Vec<Value> = Vec::with_capacity(data.len());
        let mut args = [0.0f64; 2];
        for (i, &n) in data.iter().enumerate() {
            args[0] = n;
            args[1] = i as f64;
            out.push(box_result(&nf, nf.call(&args[..arity])));
        }
        Some(Value::Array(VmRef::new(out)))
    }

    /// Fused `filter`. Callback is a predicate of arity ≤ 2. Keeps the ORIGINAL element (not the
    /// callback result) when the result is truthy — so the output is always a subset of the numeric
    /// input. Packed input → packed output (`packed_or_empty`); boxed `Array` input → boxed `Array`.
    pub(super) fn filter(arr: &Value, cb: &Value) -> Option<Value> {
        let nf = fused_numeric_fn(cb)?;
        if nf.arity() > 2 {
            return None;
        }
        let data = numeric_snapshot(arr)?;
        let arity = nf.arity();
        let packed_input = matches!(arr, Value::NumberArray(_));
        let mut args = [0.0f64; 2];
        if packed_input {
            let mut out: Vec<f64> = Vec::new();
            for (i, &n) in data.iter().enumerate() {
                args[0] = n;
                args[1] = i as f64;
                if result_truthy(nf.call(&args[..arity])) {
                    out.push(n);
                }
            }
            if out.is_empty() {
                return Some(Value::Array(VmRef::new(Vec::new())));
            }
            return Some(Value::number_array(out));
        }
        let mut out: Vec<Value> = Vec::new();
        for (i, &n) in data.iter().enumerate() {
            args[0] = n;
            args[1] = i as f64;
            if result_truthy(nf.call(&args[..arity])) {
                out.push(Value::Number(n));
            }
        }
        Some(Value::Array(VmRef::new(out)))
    }

    /// Fused `reduce`. Callback is `(acc, element, index)` — arity ≤ 3 (a 4th `array` param bails).
    /// The accumulator stays an `f64` across the whole fold, so the callback result must NOT be a bool
    /// (`!result_is_bool()`): a bool acc fed back into the numeric fn would diverge from the interpreter
    /// (which would pass a `Value::Bool`, not a number). No-initial-value semantics match the generic
    /// path: absent init (`Value::Null`) with a non-empty array seeds `acc = data[0]` and scans from
    /// index 1; an empty array with no init throws (bail to the generic path, which raises it).
    pub(super) fn reduce(arr: &Value, cb: &Value, initial: Option<&Value>) -> Option<Value> {
        let nf = fused_numeric_fn(cb)?;
        if nf.arity() > 3 || nf.result_is_bool() {
            return None;
        }
        let data = numeric_snapshot(arr)?;
        let arity = nf.arity();
        // Determine seed. Reduce with no initial value AND an empty array is a TypeError in JS — let
        // the generic path produce that exact throw rather than replicate it here.
        let (start, mut acc) = match initial {
            None if !data.is_empty() => (1usize, data[0]),
            None => return None,             // empty + no init → generic path throws
            Some(Value::Number(n)) => (0usize, *n),
            Some(_) => return None, // non-numeric/explicit-null init → generic path (acc wouldn't stay f64)
        };
        let mut args = [0.0f64; 3];
        for (i, &x) in data.iter().enumerate().skip(start) {
            args[0] = acc;
            args[1] = x;
            args[2] = i as f64;
            acc = nf.call(&args[..arity]);
        }
        Some(Value::Number(acc))
    }

    /// Fused `forEach`. Callback arity ≤ 2; result discarded; always returns `Value::Null`. Only worth
    /// fusing when the callback is side-effect-free numeric arithmetic — but a `NumericFn` is pure by
    /// construction (no calls/member/throw), so a fused `forEach` is observably a no-op EXCEPT for its
    /// (absent) side effects: it computes and discards. Kept for completeness / uniformity; it can
    /// never diverge because there is nothing observable to diverge on.
    pub(super) fn for_each(arr: &Value, cb: &Value) -> Option<Value> {
        let nf = fused_numeric_fn(cb)?;
        if nf.arity() > 2 {
            return None;
        }
        let data = numeric_snapshot(arr)?;
        let arity = nf.arity();
        let mut args = [0.0f64; 2];
        for (i, &n) in data.iter().enumerate() {
            args[0] = n;
            args[1] = i as f64;
            let _ = nf.call(&args[..arity]);
        }
        Some(Value::Null)
    }
}

struct VmClosure {
    chunk: Arc<Chunk>,
    /// Whether this closure can run on the frame stack — computed ONCE at creation (eligibility is an
    /// O(chunk) bytecode scan; doing it per call regressed perf). `true` iff the chunk is frame-eligible
    /// and there is no numeric JIT for it.
    frameable: bool,
    #[cfg(not(target_arch = "wasm32"))]
    jit_fn: Option<crate::jit::NumericFn>,
    enclosing: SharedChain,
    globals: VmRef<ObjectMap>,
    capabilities: Arc<HashSet<String>>,
    native_modules: VmRef<HashMap<String, VmRef<ObjectMap>>>,
}

impl tishlang_core::Callable for VmClosure {
    fn call(&self, args: &[Value]) -> Value {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(nf) = self.jit_fn {
                let arity = nf.arity();
                if args.len() >= arity {
                    let mask = nf.array_param_mask();
                    if mask == 0 {
                        // Pure-numeric register-f64 path.
                        let mut nums = [0f64; 8];
                        let mut all_numbers = true;
                        for i in 0..arity {
                            if let Value::Number(n) = &args[i] {
                                nums[i] = *n;
                            } else {
                                all_numbers = false;
                                break;
                            }
                        }
                        if all_numbers {
                            // #189: a JV function has function-local `f64` arrays lowered to
                            // `tish_jv_*` calls over a per-thread arena. An out-of-bounds access (or a
                            // non-numeric return) sets a per-thread deopt flag; we discard the numeric
                            // result and re-run the interpreter. Sound because JV arrays never escape
                            // the function — nothing observable was mutated, so re-execution
                            // reproduces identical behaviour.
                            if nf.is_jv() {
                                crate::jit::jv_reset_deopt();
                                let r = nf.call(&nums[..arity]);
                                if !crate::jit::jv_take_deopt() {
                                    return if nf.result_is_bool() {
                                        Value::Bool(r != 0.0)
                                    } else {
                                        Value::Number(r)
                                    };
                                }
                                // deopt ⇒ fall through to the interpreter below.
                            } else {
                                // #381: a self-recursive JIT'd function carries a RecurGuard so it bails
                                // (rather than overflowing the native stack) when the recursion nears the
                                // real remaining stack. On a bail we raise the same catchable RangeError as
                                // the non-JIT paths — tish's deopt tier producing the throw, not the JIT.
                                let res = if nf.recur_guarded() {
                                    let anchor = 0u8;
                                    let current_sp = &anchor as *const u8 as usize;
                                    // `stack_limit` = the stack address below which we bail, leaving a
                                    // headroom margin. If the remaining stack is unknown we pass 0 (SP is
                                    // never below 0 → never trips), i.e. behave as before.
                                    let stack_limit = match stacker::remaining_stack() {
                                        Some(rem) => {
                                            // Cap the margin at half of what's left: when a JIT'd function
                                            // is first entered on an already-deep stack (rem < margin),
                                            // `bottom + margin` would sit ABOVE the current SP and trip on
                                            // the very first call — a false positive on shallow recursion.
                                            // Capping keeps the limit strictly below SP for any rem, while
                                            // still bailing with real headroom to spare.
                                            let margin = RECUR_STACK_MARGIN.min(rem / 2);
                                            current_sp.saturating_sub(rem).saturating_add(margin)
                                        }
                                        None => 0,
                                    };
                                    let mut guard = crate::jit::RecurGuard {
                                        stack_limit,
                                        tripped: 0,
                                    };
                                    let r = nf.call_guarded(&nums[..arity], &mut guard);
                                    if guard.tripped != 0 {
                                        set_pending_throw(stack_overflow_error());
                                        return Value::Null;
                                    }
                                    r
                                } else {
                                    nf.call(&nums[..arity])
                                };
                                return if nf.result_is_bool() {
                                    Value::Bool(res != 0.0)
                                } else {
                                    Value::Number(res)
                                };
                            }
                        }
                    } else if let Some(v) = try_call_array_jit(&nf, args, arity, mask) {
                        // Array-mode path: succeeded (all-numeric arrays, in-bounds). On any bail
                        // (non-numeric element, NumberArray, OOB deopt) this returns None and we fall
                        // through to the interpreter — so behaviour is always correct.
                        return v;
                    }
                }
            }
        }
        // #381: bound recursion so a runaway recursive closure throws a catchable RangeError rather
        // than overflowing the native stack (this recursive re-entry is the DEFAULT `tish run` path).
        // The counter is thread-local because each closure call builds a fresh `Vm`; the parked throw
        // is picked up by the caller's `take_pending_throw()` check (Call/SelfCall post-call).
        let depth = tishlang_core::inc_call_depth();
        if depth > max_call_depth() {
            tishlang_core::dec_call_depth();
            set_pending_throw(stack_overflow_error());
            return Value::Null;
        }
        let mut vm = Vm {
            stack: Vec::new(),
            scope: ObjectMap::default(),
            enclosing: self.enclosing.clone(),
            globals: self.globals.clone(),
            capabilities: Arc::clone(&self.capabilities),
            native_modules: self.native_modules.clone(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
                vm.run_chunk(self.chunk.as_ref(), &self.chunk.nested, args, false)
                    .unwrap_or(Value::Null)
            })
        };
        #[cfg(target_arch = "wasm32")]
        let result = {
            vm.run_chunk(&self.chunk, &self.chunk.nested, args, false)
                .unwrap_or(Value::Null)
        };
        tishlang_core::dec_call_depth();
        result
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Vm {
    /// VM with every capability that exists in this `tishlang_vm` build (embedders, tests, `run()`).
    pub fn new() -> Self {
        Self::with_capabilities_arc(Arc::new(all_compiled_capabilities()))
    }

    /// VM with an explicit capability set (e.g. from `tish run --feature …`).
    pub fn with_capabilities(capabilities: HashSet<String>) -> Self {
        Self::with_capabilities_arc(Arc::new(capabilities))
    }

    fn with_capabilities_arc(capabilities: Arc<HashSet<String>>) -> Self {
        Self {
            stack: Vec::new(),
            scope: ObjectMap::default(),
            enclosing: SharedChain::new(Vec::new()),
            globals: VmRef::new(init_globals(capabilities.as_ref())),
            capabilities,
            native_modules: VmRef::new(HashMap::new()),
        }
    }

    /// Register an externally-supplied native module under a `cargo:`-style
    /// spec (e.g. `"cargo:tish_pg"`). The `exports` map is what
    /// `LoadNativeExport` will index into when user code imports from this
    /// spec. Intended to be called by the `tishlang_cranelift_runtime` /
    /// `tishlang_llvm` link step, or by external embedders that want to
    /// expose Rust crates to `.tish` programs running on the bytecode VM.
    pub fn register_native_module(&mut self, spec: impl Into<String>, exports: ObjectMap) {
        self.native_modules
            .borrow_mut()
            .insert(spec.into(), VmRef::new(exports));
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.globals.borrow().get(name).cloned()
    }

    pub fn set_global(&mut self, name: Arc<str>, value: Value) {
        self.globals.borrow_mut().insert(name, value);
    }

    /// Names of all globals (for REPL bare-word tab completion).
    pub fn global_names(&self) -> Vec<String> {
        self.globals
            .borrow()
            .keys()
            .map(|k| k.as_ref().to_string())
            .collect()
    }

    fn read_u16(code: &[u8], ip: &mut usize) -> u16 {
        let a = code[*ip] as u16;
        let b = code[*ip + 1] as u16;
        *ip += 2;
        (a << 8) | b
    }

    fn read_i16(code: &[u8], ip: &mut usize) -> i16 {
        Self::read_u16(code, ip) as i16
    }

    /// Pop innermost try handler, truncate stack, push thrown value, jump to catch.
    fn unwind_throw(
        try_handlers: &mut Vec<(usize, usize)>,
        stack: &mut Vec<Value>,
        ip: &mut usize,
        v: Value,
    ) -> Result<(), String> {
        let (catch_ip, stack_len) = try_handlers
            .pop()
            .ok_or_else(|| format!("Uncaught throw: {}", v.to_display_string()))?;
        stack.truncate(stack_len);
        stack.push(v);
        *ip = catch_ip;
        Ok(())
    }

    pub fn run(&mut self, chunk: &Chunk) -> Result<Value, String> {
        self.run_with_options(chunk, false)
    }

    /// Run a chunk using this VM's capability set. `repl_mode` persists top-level `let` across REPL lines.
    pub fn run_with_options(&mut self, chunk: &Chunk, repl_mode: bool) -> Result<Value, String> {
        // #187: the directly-callable-callee registry is scoped to ONE top-level program run. Clear it
        // here (the outermost entry — closures run via `run_chunk`, not this) so a long-lived process
        // (REPL / embedder) running a NEW program never resolves a stale callee registered by a prior
        // one. A cross-caller and its callee always compile within the same run, so this is sufficient.
        #[cfg(not(target_arch = "wasm32"))]
        crate::jit::reset_callees();
        let result = self.run_chunk(chunk, &chunk.nested, &[], repl_mode);
        // A throw that escaped every `catch` reaches here as the pending-throw sentinel; turn the
        // parked value into the conventional uncaught-error message (issue #60).
        if let Err(e) = &result {
            if e == PENDING_THROW_SENTINEL {
                let v = take_pending_throw().unwrap_or(Value::Null);
                return Err(format!("Uncaught {}", v.to_display_string()));
            }
        }
        result
    }

    /// Whether the experimental frame-VM path is on (`TISH_FRAME_VM=1`). Flag-off (default) is
    /// byte-identical to the recursive `run_chunk` model — every `Value::Function` call goes through
    /// `VmClosure::call` exactly as before.
    #[inline]
    fn frame_vm_enabled() -> bool {
        // Read the env var ONCE and cache it. This is checked on the hot path (every Call opcode +
        // every closure creation), so a per-call `std::env::var` (a lock + String alloc) is a severe
        // regression to the DEFAULT path — caching makes the flag-off check a single atomic load.
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("TISH_FRAME_VM")
                .map(|v| v == "1")
                .unwrap_or(false)
        })
    }

    /// A `VmClosure` runs on the frame stack iff its chunk is frame-eligible AND it has no numeric
    /// JIT (jit'd functions stay on the faster native path via `VmClosure::call`; the frame loop's
    /// niche is non-jit'd call-heavy / mutually-recursive functions + wasi where there is no JIT).
    fn vmclosure_frameable(vc: &VmClosure) -> bool {
        vc.frameable
    }

    /// A chunk is frame-eligible iff slot-based and every opcode is one `run_framed` handles.
    /// `LoadConst` of a nested `Closure` is excluded (closure creation needs the full `run_chunk`).
    fn chunk_frame_eligible(chunk: &Chunk) -> bool {
        if !chunk.slot_based {
            return false;
        }
        let code = &chunk.code;
        let mut ip = 0usize;
        while ip < code.len() {
            let op = match Opcode::from_u8(code[ip]) {
                Some(o) => o,
                None => return false,
            };
            match op {
                Opcode::Nop
                | Opcode::LoadLocal
                | Opcode::StoreLocal
                | Opcode::LoadVar
                | Opcode::BinOp
                | Opcode::Jump
                | Opcode::JumpIfFalse
                | Opcode::JumpBack
                | Opcode::Pop
                | Opcode::Call
                | Opcode::SelfCall
                | Opcode::Return => {}
                Opcode::LoadConst => {
                    let idx = (((*code.get(ip + 1).unwrap_or(&0)) as usize) << 8)
                        | ((*code.get(ip + 2).unwrap_or(&0)) as usize);
                    if matches!(chunk.constants.get(idx), Some(Constant::Closure(_))) {
                        return false;
                    }
                }
                _ => return false,
            }
            ip += match op.instruction_size(code, ip) {
                Some(s) => s,
                None => return false,
            };
        }
        true
    }

    /// On-stack replacement (#190): run one hot loop region natively, or report why we can't.
    ///
    /// Called from the frame VM's `JumpBack` handler once a loop's back-edge counter trips. Compiles
    /// (cached) the region `[header_ip, region_end)`; if it is a pure-numeric slot loop AND every live
    /// slot currently holds a `Number` (else the native f64 math would diverge from the interpreter),
    /// copies the live-ins into an f64 buffer, runs the whole remaining loop natively, writes the
    /// live-outs back as `Number`s, and returns the chunk `ip` of the loop exit to resume dispatch at.
    ///
    /// Soundness: the v1 region whitelist is pure slot/stack arithmetic — no calls, no member/index,
    /// no object or array mutation — so nothing observable happens inside the region, and the
    /// interpreter running the same bytecode from the same numeric live-ins produces bit-identical
    /// slots (the `emit_simple_op` lowering matches `eval_binop`). A non-numeric live-in is the deopt
    /// case: we simply don't OSR and keep interpreting, so state is never corrupted.
    #[cfg(not(target_arch = "wasm32"))]
    fn run_osr(
        chunk: &Chunk,
        header_ip: usize,
        region_end: usize,
        slots: &mut [Value],
        slot_base: usize,
    ) -> OsrResult {
        let loopfn = match crate::jit::try_compile_loop(chunk, header_ip, region_end) {
            Some(lf) => lf,
            None => return OsrResult::NotCompilable,
        };
        let mut buf: Vec<f64> = Vec::with_capacity(loopfn.used_slots.len());
        for &slot in &loopfn.used_slots {
            match slots.get(slot_base + slot as usize) {
                Some(Value::Number(n)) => buf.push(*n),
                _ => return OsrResult::LiveInMiss, // a live slot is non-numeric → deopt, keep interpreting
            }
        }
        let mut deopt: u8 = 0;
        // SAFETY: `buf` is a valid `[f64; used_slots.len()]`; `deopt` is a valid `*mut u8`. The region
        // was compiled for exactly this chunk+header (fingerprint-checked in the cache).
        let exit_id = loopfn.call(&mut buf, &mut deopt);
        for (p, &slot) in loopfn.used_slots.iter().enumerate() {
            if let Some(d) = slots.get_mut(slot_base + slot as usize) {
                *d = Value::Number(buf[p]);
            }
        }
        match loopfn.exits.get(exit_id as usize) {
            Some(&ip) => OsrResult::Compiled(ip),
            // Out-of-range exit id is impossible (the region returns an in-range id), but if it ever
            // happened the slots are already a consistent post-loop state, so re-interpreting the
            // (now-false) loop header exits correctly.
            None => OsrResult::LiveInMiss,
        }
    }

    /// OSR is **default ON**; `TISH_OSR=0` disables it (escape hatch, mirrors `TISH_JIT_ARRAYS`).
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    fn osr_enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("TISH_OSR").map(|v| v != "0").unwrap_or(true))
    }

    /// Iterative frame-stack execution of a frame-eligible `VmClosure` (the frame-VM, flag-on).
    /// Returns `None` if the entry chunk is ineligible (caller falls back to `VmClosure::call`).
    /// Calls + recursion run on the heap `frames` stack — no per-call `Vm`, no recursive `run_chunk`
    /// re-entry, so deep + mutual recursion can't overflow and it works on wasi (no JIT there).
    fn run_framed(&mut self, top: &VmClosure, args: &[Value]) -> Option<Result<Value, String>> {
        if !Self::vmclosure_frameable(top) {
            return None;
        }
        let mut cur: Arc<Chunk> = top.chunk.clone();
        let mut enclosing: SharedChain = top.enclosing.clone();
        let mut ip: usize = 0;
        let mut stack_base: usize = self.stack.len();
        // Slot-region pooling: ALL frames' locals share one `slots` Vec; each frame occupies
        // `slots[slot_base .. slot_base + num_slots]`. A call does `resize` (amortized, no per-call
        // heap alloc — unlike `run_chunk` which `vec!`s a fresh `slot_locals` every call); a return
        // does `truncate`. This is what makes the frame loop cheaper than the recursive path.
        let mut slots: Vec<Value> = Vec::new();
        let mut slot_base: usize = 0;
        slots.resize(cur.num_slots as usize, Value::Null);
        // #190 OSR: per-`(chunk, loop header)` back-edge counters (`u32::MAX` = gave up), with a
        // single-entry fast slot for the loop currently spinning (see the `run_chunk` copy for the
        // rationale) so a resolved hot loop stays off the HashMap.
        #[cfg(not(target_arch = "wasm32"))]
        let mut osr_counters: std::collections::HashMap<(usize, usize), u32> =
            std::collections::HashMap::new();
        #[cfg(not(target_arch = "wasm32"))]
        let mut osr_hot: ((usize, usize), u32) = ((usize::MAX, usize::MAX), 0);
        for i in 0..(cur.param_count as usize) {
            if let Some(v) = args.get(i) {
                if let Some(d) = slots.get_mut(slot_base + i) {
                    *d = v.clone();
                }
            }
        }
        // Suspended callers: (chunk, return ip, caller slot_base, caller stack_base, enclosing).
        let mut frames: Vec<(Arc<Chunk>, usize, usize, usize, SharedChain)> = Vec::new();

        macro_rules! ferr {
            ($($t:tt)*) => {
                return Some(Err(format!($($t)*)))
            };
        }
        macro_rules! fpop {
            () => {
                match self.stack.pop() {
                    Some(v) => v,
                    None => ferr!("Stack underflow in run_framed"),
                }
            };
        }

        // SAFETY: `code` aliases the current frame's chunk bytecode. The chunk is kept alive by `cur`
        // (and suspended-frame chunks by `frames`), so the slice stays valid for as long as we read
        // it; it is re-derived via `rebind_code!()` after every frame switch (Call/Return/end).
        // Laundering the borrow lets the hot opcode path index `code[ip]` directly with no per-opcode
        // Arc deref — matching run_chunk (the per-opcode Arc deref was a measured ~10% shallow-call regression).
        let mut code: &[u8] = unsafe { &*(cur.code.as_slice() as *const [u8]) };

        loop {
            if ip >= code.len() {
                self.stack.truncate(stack_base);
                slots.truncate(slot_base);
                match frames.pop() {
                    Some((c, rip, sbase, sb, enc)) => {
                        cur = c;
                        ip = rip;
                        slot_base = sbase;
                        stack_base = sb;
                        enclosing = enc;
                        code = unsafe { &*(cur.code.as_slice() as *const [u8]) };
                        self.stack.push(Value::Null);
                        continue;
                    }
                    None => return Some(Ok(Value::Null)),
                }
            }
            let op = match Opcode::from_u8(code[ip]) {
                Some(o) => o,
                None => ferr!("Bad opcode {} in run_framed", code[ip]),
            };
            ip += 1;
            match op {
                Opcode::Nop => {}
                Opcode::LoadLocal => {
                    let slot = Self::read_u16(code, &mut ip) as usize;
                    match slots.get(slot_base + slot) {
                        Some(v) => self.stack.push(v.clone()),
                        None => ferr!("Local slot out of bounds: {}", slot),
                    }
                }
                Opcode::StoreLocal => {
                    let slot = Self::read_u16(code, &mut ip) as usize;
                    let v = fpop!();
                    match slots.get_mut(slot_base + slot) {
                        Some(d) => *d = v,
                        None => ferr!("Local slot out of bounds: {}", slot),
                    }
                }
                Opcode::LoadConst => {
                    let idx = Self::read_u16(code, &mut ip) as usize;
                    let v = match cur.constants.get(idx) {
                        Some(Constant::Number(n)) => Value::Number(*n),
                        Some(Constant::String(s)) => {
                            Value::String(tishlang_core::ArcStr::from(s.as_ref()))
                        }
                        Some(Constant::Bool(b)) => Value::Bool(*b),
                        Some(Constant::Null) => Value::Null,
                        _ => ferr!("Ineligible constant {} in run_framed", idx),
                    };
                    self.stack.push(v);
                }
                Opcode::LoadVar => {
                    let idx = Self::read_u16(code, &mut ip) as usize;
                    let name = match cur.names.get(idx) {
                        Some(n) => n.clone(),
                        None => ferr!("Name index out of bounds: {}", idx),
                    };
                    let v = enclosing
                        .iter()
                        .find_map(|e| e.borrow().get(name.as_ref()).cloned())
                        .or_else(|| self.scope.get(name.as_ref()).cloned())
                        .or_else(|| self.globals.borrow().get(name.as_ref()).cloned());
                    match v {
                        Some(v) => self.stack.push(v),
                        None => ferr!("Undefined variable: {}", name),
                    }
                }
                Opcode::BinOp => {
                    let op_u8 = Self::read_u16(code, &mut ip) as u8;
                    let r = fpop!();
                    let l = fpop!();
                    let bop = match u8_to_binop(op_u8) {
                        Some(b) => b,
                        None => ferr!("Unknown binop: {}", op_u8),
                    };
                    match eval_binop(bop, &l, &r) {
                        Ok(res) => self.stack.push(res),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Opcode::Jump => {
                    let offset = Self::read_i16(code, &mut ip) as isize;
                    ip = (ip as isize + offset).max(0) as usize;
                }
                Opcode::JumpIfFalse => {
                    let offset = Self::read_i16(code, &mut ip) as isize;
                    let v = fpop!();
                    if !v.is_truthy() {
                        ip = (ip as isize + offset).max(0) as usize;
                    }
                }
                Opcode::JumpBack => {
                    let dist = Self::read_u16(code, &mut ip) as usize;
                    // #190 OSR: `ip` now points just past the JumpBack (region end); the loop header is
                    // `region_end - dist`. Once a loop is hot, try to run its remaining iterations in
                    // native code (numeric slot loops only; anything else falls straight through).
                    #[cfg(not(target_arch = "wasm32"))]
                    if Self::osr_enabled() {
                        let region_end = ip;
                        let header_ip = ip.saturating_sub(dist);
                        let key = (Arc::as_ptr(&cur) as usize, header_ip);
                        if key != osr_hot.0 {
                            if osr_hot.0 .0 != usize::MAX {
                                osr_counters.insert(osr_hot.0, osr_hot.1);
                            }
                            osr_hot = (key, osr_counters.get(&key).copied().unwrap_or(0));
                        }
                        if osr_hot.1 != u32::MAX {
                            osr_hot.1 = osr_hot.1.saturating_add(1);
                            if osr_hot.1 >= OSR_THRESHOLD
                                && osr_hot
                                    .1
                                    .saturating_sub(OSR_THRESHOLD)
                                    .is_multiple_of(OSR_RETRY)
                            {
                                match Self::run_osr(
                                    &cur, header_ip, region_end, &mut slots, slot_base,
                                ) {
                                    OsrResult::Compiled(exit_ip) => {
                                        ip = exit_ip;
                                        continue;
                                    }
                                    OsrResult::LiveInMiss => {}
                                    OsrResult::NotCompilable => osr_hot.1 = u32::MAX,
                                }
                            }
                        }
                    }
                    ip = ip.saturating_sub(dist);
                }
                Opcode::Pop => {
                    let _ = fpop!();
                }
                Opcode::SelfCall => {
                    let argc = Self::read_u16(code, &mut ip) as usize;
                    let mut call_args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        call_args.push(fpop!());
                    }
                    call_args.reverse();
                    // #381: bound the heap frame stack (the opt-in frame-VM path grows a Vec instead
                    // of the native stack, so unbounded recursion here is OOM rather than overflow).
                    if frames.len() >= max_call_depth() {
                        set_pending_throw(stack_overflow_error());
                        return Some(Err(PENDING_THROW_SENTINEL.to_string()));
                    }
                    frames.push((cur.clone(), ip, slot_base, stack_base, enclosing.clone()));
                    let new_base = slots.len();
                    slots.resize(new_base + cur.num_slots as usize, Value::Null);
                    slot_base = new_base;
                    ip = 0;
                    stack_base = self.stack.len();
                    for i in 0..(cur.param_count as usize) {
                        if let Some(v) = call_args.get(i) {
                            if let Some(d) = slots.get_mut(slot_base + i) {
                                *d = v.clone();
                            }
                        }
                    }
                }
                Opcode::Call => {
                    let argc = Self::read_u16(code, &mut ip) as usize;
                    let mut call_args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        call_args.push(fpop!());
                    }
                    call_args.reverse();
                    let callee = fpop!();
                    match &callee {
                        Value::Function(f) => {
                            let framed = f
                                .as_any()
                                .downcast_ref::<VmClosure>()
                                .filter(|vc| Self::vmclosure_frameable(vc));
                            if let Some(vc) = framed {
                                let next_chunk = vc.chunk.clone();
                                let next_enc = vc.enclosing.clone();
                                // Move (not clone) the caller's chunk+chain into the frame; the Arc
                                // refcounts are unchanged (the chunk heap data doesn't move, so the
                                // laundered `code` ptr stays valid until rebind below). Halves the
                                // per-call Arc traffic vs cloning for the push.
                                if frames.len() >= max_call_depth() {
                                    set_pending_throw(stack_overflow_error());
                                    return Some(Err(PENDING_THROW_SENTINEL.to_string()));
                                }
                                frames.push((cur, ip, slot_base, stack_base, enclosing));
                                cur = next_chunk;
                                enclosing = next_enc;
                                code = unsafe { &*(cur.code.as_slice() as *const [u8]) };
                                let new_base = slots.len();
                                slots.resize(new_base + cur.num_slots as usize, Value::Null);
                                slot_base = new_base;
                                ip = 0;
                                stack_base = self.stack.len();
                                for i in 0..(cur.param_count as usize) {
                                    if let Some(v) = call_args.get(i) {
                                        if let Some(d) = slots.get_mut(slot_base + i) {
                                            *d = v.clone();
                                        }
                                    }
                                }
                            } else {
                                let r = f.call(&call_args);
                                // A throw escaping the callee can't be caught here (frameable
                                // chunks have no `try`); bubble it to an enclosing frame (#60).
                                if pending_throw_is_set() {
                                    return Some(Err(PENDING_THROW_SENTINEL.to_string()));
                                }
                                self.stack.push(r);
                            }
                        }
                        Value::Object(o) => {
                            let cf = match o.borrow().strings.get("__call") {
                                Some(Value::Function(cf)) => cf.clone(),
                                _ => ferr!("Call of non-function: {}", callee.type_name()),
                            };
                            let r = cf.call(&call_args);
                            if pending_throw_is_set() {
                                return Some(Err(PENDING_THROW_SENTINEL.to_string()));
                            }
                            self.stack.push(r);
                        }
                        _ => ferr!("Call of non-function: {}", callee.type_name()),
                    }
                }
                Opcode::Return => {
                    let result = self.stack.pop().unwrap_or(Value::Null);
                    self.stack.truncate(stack_base);
                    slots.truncate(slot_base);
                    match frames.pop() {
                        Some((c, rip, sbase, sb, enc)) => {
                            cur = c;
                            ip = rip;
                            slot_base = sbase;
                            stack_base = sb;
                            enclosing = enc;
                            code = unsafe { &*(cur.code.as_slice() as *const [u8]) };
                            self.stack.push(result);
                        }
                        None => return Some(Ok(result)),
                    }
                }
                other => ferr!("Unhandled opcode {:?} in run_framed", other),
            }
        }
    }

    fn run_chunk(
        &mut self,
        chunk: &Chunk,
        nested: &[Chunk],
        args: &[Value],
        repl_mode: bool,
    ) -> Result<Value, String> {
        let code = &chunk.code;
        let constants = &chunk.constants;
        let names = &chunk.names;

        let mut ip = 0;
        // Lazily allocated name-keyed scope. Slot-based chunks never WRITE it (params + body locals
        // live in `slot_locals`; `StoreVar` checks-then-falls-through to globals; a slot-based chunk
        // has no captured locals by construction), so on the hot slot-based call path we skip the
        // `VmRef::new(Arc<Mutex<HashMap>>)` box entirely. Non-slot chunks need it eagerly for params.
        // `ls_get_or_init!()` lazily creates it on the first write/capture; reads treat `None` as empty.
        let mut local_scope: Option<ScopeMap> = if chunk.slot_based {
            None
        } else {
            Some(VmRef::new(ObjectMap::default()))
        };
        macro_rules! ls_get_or_init {
            () => {{
                local_scope.get_or_insert_with(|| VmRef::new(ObjectMap::default()))
            }};
        }
        // Slot-based chunks (self-contained functions) use a bare `Vec<Value>`
        // frame indexed by slot — no per-call hashmap, no name lookups. Args bind
        // to slots 0..param_count. Empty for name-based chunks.
        let mut slot_locals: Vec<Value> = Vec::new();
        // #190 OSR: per-loop-header back-edge counters for this frame (`u32::MAX` = gave up). `osr_hot`
        // is a single-entry fast slot for the loop currently spinning — its header is constant across
        // iterations, so the common hot loop never touches the HashMap (which is hit only when the
        // active loop changes: entry, exit, or a nested-loop switch). Keeps the per-back-edge cost of a
        // resolved loop to two integer compares.
        #[cfg(not(target_arch = "wasm32"))]
        let mut osr_counters: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
        #[cfg(not(target_arch = "wasm32"))]
        let mut osr_hot: (usize, u32) = (usize::MAX, 0);
        // Frame-local string builder for `acc += x` on a slot local (Opcode::AppendLocal): keeps the
        // accumulator in a growable `sb_buf` so appends are amortized O(1) instead of reallocating the
        // whole string each time. `sb_slot` is the slot currently buffered (at most one). The slot's
        // own value is stale while buffered; `sb_flush!()` writes the buffer back. Slots are
        // frame-private (never captured by closures and invisible to callees), and every read of a
        // slot goes through `LoadLocal`, so flushing there is sufficient for soundness — no
        // cross-frame state. JS string immutability is preserved: reads always see the full string.
        let mut sb_slot: Option<usize> = None;
        let mut sb_buf = String::new();
        macro_rules! sb_flush {
            () => {{
                if let Some(s) = sb_slot.take() {
                    if let Some(dst) = slot_locals.get_mut(s) {
                        *dst = Value::String(std::mem::take(&mut sb_buf).into());
                    }
                    sb_buf.clear();
                }
            }};
        }
        if chunk.slot_based {
            slot_locals = vec![Value::Null; chunk.num_slots as usize];
            let param_count = chunk.param_count as usize;
            for i in 0..param_count {
                if let Some(v) = args.get(i) {
                    if let Some(dst) = slot_locals.get_mut(i) {
                        *dst = v.clone();
                    }
                }
            }
        } else {
            let mut ls = ls_get_or_init!().borrow_mut();
            let param_count = chunk.param_count as usize;
            if chunk.rest_param_index != NO_REST_PARAM {
                let ri = chunk.rest_param_index as usize;
                for (i, name) in chunk.names.iter().take(param_count).enumerate() {
                    if i < ri {
                        let v = args.get(i).cloned().unwrap_or(Value::Null);
                        ls.insert(Arc::clone(name), v);
                    } else if i == ri {
                        let rest_arr: Vec<Value> = args.iter().skip(ri).cloned().collect();
                        ls.insert(Arc::clone(name), Value::Array(VmRef::new(rest_arr)));
                    }
                }
            } else {
                for (i, name) in chunk.names.iter().take(param_count).enumerate() {
                    if let Some(v) = args.get(i) {
                        ls.insert(Arc::clone(name), v.clone());
                    }
                }
            }
        }
        let mut try_handlers: Vec<(usize, usize)> = vec![];
        let mut block_undo_stack: Vec<Vec<(Arc<str>, Option<Value>)>> = vec![];
        // Names of loop variables currently in a per-iteration binding region (ES `let` semantics).
        // A closure created while this is non-empty snapshots these into a fresh overlay so it
        // captures the loop variable's value for THIS iteration. Pushed/popped by LoopVarsBegin/End.
        let mut active_loop_vars: Vec<Arc<str>> = Vec::new();
        // Offset of the instruction currently executing — updated each iteration, read by the
        // error macros to attach a source location (issue #74). Declared here (not in the loop)
        // so it's in scope where `catchable!` is defined (macro hygiene).
        let mut instr_off = 0usize;

        // Throw `$v` to the nearest enclosing handler (issue #60): if this frame has a live
        // `try`, jump to its `catch` with `$v` on the stack; otherwise park `$v` in the
        // thread-local and bubble the sentinel so an enclosing frame's catch can take it.
        macro_rules! raise {
            ($v:expr) => {{
                let __thrown = $v;
                if let Some((catch_ip, stack_len)) = try_handlers.pop() {
                    self.stack.truncate(stack_len);
                    self.stack.push(__thrown);
                    ip = catch_ip;
                    continue;
                } else {
                    set_pending_throw(__thrown);
                    return Err(PENDING_THROW_SENTINEL.to_string());
                }
            }};
        }
        // Evaluate a fallible, JS-throwable opcode helper: on `Err(msg)` the message becomes a
        // catchable `TypeError` (`x.foo()` on null, calling a non-function, …) routed through
        // `raise!` instead of aborting the whole VM.
        macro_rules! catchable {
            ($expr:expr) => {
                match $expr {
                    Ok(v) => v,
                    Err(msg) => raise!(construct_builtin::error_object(
                        "TypeError",
                        &locate_error(chunk, instr_off, &msg)
                    )),
                }
            };
        }

        loop {
            if ip >= code.len() {
                break;
            }
            // Offset of the instruction about to execute (read by the error macros, #74).
            instr_off = ip;
            let op = code[ip];
            ip += 1;
            if op == Opcode::Nop as u8 {
                continue;
            }
            let opcode = Opcode::from_u8(op).ok_or_else(|| format!("Unknown opcode: {}", op))?;

            match opcode {
                Opcode::Nop => {}
                Opcode::LoadLocal => {
                    let slot = Self::read_u16(code, &mut ip) as usize;
                    // Flush a pending string-builder for this slot so the read sees the full string.
                    if sb_slot == Some(slot) {
                        sb_flush!();
                    }
                    let v = slot_locals
                        .get(slot)
                        .cloned()
                        .ok_or_else(|| format!("Local slot out of bounds: {}", slot))?;
                    self.stack.push(v);
                }
                Opcode::StoreLocal => {
                    let slot = Self::read_u16(code, &mut ip) as usize;
                    // A plain store overwrites the slot — drop any builder buffering it.
                    if sb_slot == Some(slot) {
                        sb_slot = None;
                        sb_buf.clear();
                    }
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow in StoreLocal".to_string())?;
                    match slot_locals.get_mut(slot) {
                        Some(dst) => *dst = v,
                        None => return Err(format!("Local slot out of bounds: {}", slot)),
                    }
                }
                Opcode::AppendLocal => {
                    let slot = Self::read_u16(code, &mut ip) as usize;
                    let rhs = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow in AppendLocal".to_string())?;
                    if sb_slot == Some(slot) {
                        // Continue the active builder for this slot — amortized O(1) append.
                        append_value_for_string_concat(&mut sb_buf, &rhs);
                    } else {
                        // Switching slots: flush the previous builder, then (re)start for this one.
                        sb_flush!();
                        match slot_locals.get(slot) {
                            Some(Value::String(a)) => {
                                sb_buf.clear();
                                sb_buf.reserve(a.len() + estimate_string_concat_len(&rhs));
                                sb_buf.push_str(a);
                                append_value_for_string_concat(&mut sb_buf, &rhs);
                                sb_slot = Some(slot);
                            }
                            _ => {
                                // Non-string (or absent) accumulator: generic `+=` in place,
                                // identical to LoadLocal; BinOp Add; StoreLocal.
                                let current = slot_locals.get(slot).cloned().unwrap_or(Value::Null);
                                let result = eval_binop(BinOp::Add, &current, &rhs)?;
                                match slot_locals.get_mut(slot) {
                                    Some(dst) => *dst = result,
                                    None => {
                                        return Err(format!("Local slot out of bounds: {}", slot))
                                    }
                                }
                            }
                        }
                    }
                }
                Opcode::LoadUpvalue | Opcode::StoreUpvalue => {
                    // Reserved for the linked-frame upvalue model (not emitted yet).
                    return Err("Upvalue opcodes not supported in this VM build".to_string());
                }
                Opcode::MathUnary => {
                    // #186 — `Math.<fn>(x)` on a number. The compiler only emits this when `Math` is
                    // the global builtin, so it is behaviour-identical to the general call: a number
                    // maps through `MathUnaryFn::apply`, a non-number coerces to NaN (matching the
                    // builtin's `extract_num(..).unwrap_or(NaN)`).
                    let id = Self::read_u16(code, &mut ip);
                    let x = match self.stack.pop() {
                        Some(Value::Number(n)) => n,
                        Some(_) | None => f64::NAN,
                    };
                    let mfn = tishlang_bytecode::MathUnaryFn::from_u16(id)
                        .ok_or_else(|| format!("Bad MathUnary id: {}", id))?;
                    self.stack.push(Value::Number(mfn.apply(x)));
                }
                Opcode::LoadConst => {
                    let idx = Self::read_u16(code, &mut ip);
                    let c = constants
                        .get(idx as usize)
                        .ok_or_else(|| format!("Constant index out of bounds: {}", idx))?;
                    let v = match c {
                        Constant::Number(n) => Value::Number(*n),
                        Constant::String(s) => {
                            Value::String(tishlang_core::ArcStr::from(s.as_ref()))
                        }
                        Constant::Bool(b) => Value::Bool(*b),
                        Constant::Null => Value::Null,
                        Constant::Closure(nested_idx) => {
                            let inner = nested
                                .get(*nested_idx)
                                .ok_or_else(|| "Nested chunk index out of bounds".to_string())?;
                            // Numeric JIT fast path (native codegen, non-wasm): if this is a
                            // straight-line numeric function, compile it once (cached per chunk)
                            // and call native code when all args are numbers; else fall back to
                            // the interpreter below. Purely additive — can't change behaviour.
                            #[cfg(not(target_arch = "wasm32"))]
                            let jit_fn = crate::jit::try_compile_numeric(inner);
                            let inner_clone = inner.clone();
                            let globals = self.globals.clone();
                            // The closure captures its defining frame's scope PLUS that frame's own
                            // enclosing chain, so functions nested arbitrarily deep still resolve
                            // every ancestor's locals (innermost first).
                            // A closure must capture a real scope (even if empty) so that, post-creation,
                            // the parent's name-based locals are visible. Materialise local_scope here.
                            let captured_scope: ScopeMap = ls_get_or_init!().clone();
                            let enclosing_chain: SharedChain =
                                SharedChain::new(if active_loop_vars.is_empty() {
                                    let mut chain = Vec::with_capacity(self.enclosing.len() + 1);
                                    chain.push(captured_scope.clone());
                                    chain.extend(self.enclosing.iter().cloned());
                                    chain
                                } else {
                                    // Per-iteration `let`: freeze the loop var(s) into an overlay that
                                    // shadows the still-shared frame scope, then the inherited chain.
                                    let mut overlay = ObjectMap::default();
                                    {
                                        let ls = captured_scope.borrow();
                                        for n in &active_loop_vars {
                                            if let Some(v) = ls.get(n.as_ref()) {
                                                overlay.insert(Arc::clone(n), v.clone());
                                            }
                                        }
                                    }
                                    let mut chain = Vec::with_capacity(self.enclosing.len() + 2);
                                    chain.push(VmRef::new(overlay));
                                    chain.push(captured_scope.clone());
                                    chain.extend(self.enclosing.iter().cloned());
                                    chain
                                });
                            let capabilities = Arc::clone(&self.capabilities);
                            let native_modules = self.native_modules.clone();
                            // Frame-eligibility is an O(chunk) bytecode scan; gate it behind the
                            // (cached) frame-VM flag so the DEFAULT path skips it entirely — flag-off
                            // closure creation pays nothing.
                            let frameable = Vm::frame_vm_enabled() && {
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    jit_fn.is_none() && Vm::chunk_frame_eligible(&inner_clone)
                                }
                                #[cfg(target_arch = "wasm32")]
                                {
                                    Vm::chunk_frame_eligible(&inner_clone)
                                }
                            };
                            let vmclosure = VmClosure {
                                chunk: std::sync::Arc::new(inner_clone),
                                frameable,
                                #[cfg(not(target_arch = "wasm32"))]
                                jit_fn,
                                enclosing: enclosing_chain,
                                globals,
                                capabilities,
                                native_modules,
                            };
                            #[cfg(feature = "send-values")]
                            {
                                Value::Function(std::sync::Arc::new(vmclosure))
                            }
                            #[cfg(not(feature = "send-values"))]
                            {
                                Value::Function(std::rc::Rc::new(vmclosure))
                            }
                        }
                    };
                    self.stack.push(v);
                }
                Opcode::LoadVar => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = local_scope
                        .as_ref()
                        .and_then(|ls| ls.borrow().get(name.as_ref()).cloned())
                        .or_else(|| {
                            // Walk the captured lexical chain, innermost first.
                            self.enclosing
                                .iter()
                                .find_map(|e| e.borrow().get(name.as_ref()).cloned())
                        })
                        .or_else(|| self.scope.get(name.as_ref()).cloned())
                        .or_else(|| self.globals.borrow().get(name.as_ref()).cloned())
                        .ok_or_else(|| format!("Undefined variable: {}", name))?;
                    self.stack.push(v);
                }
                Opcode::StoreVar => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    // Update innermost scope that has the variable (matches interpreter Scope.assign)
                    if local_scope
                        .as_ref()
                        .is_some_and(|ls| ls.borrow().contains_key(name.as_ref()))
                    {
                        ls_get_or_init!().borrow_mut().insert(Arc::clone(name), v);
                    } else if let Some(e) = self
                        .enclosing
                        .iter()
                        .find(|e| e.borrow().contains_key(name.as_ref()))
                    {
                        // Innermost captured scope that already binds the name (matches the
                        // interpreter's Scope.assign walking the lexical chain).
                        e.borrow_mut().insert(Arc::clone(name), v);
                    } else if self.scope.contains_key(name.as_ref()) {
                        self.scope.insert(Arc::clone(name), v);
                    } else if self.globals.borrow().contains_key(name.as_ref()) {
                        self.globals.borrow_mut().insert(Arc::clone(name), v);
                    } else {
                        // New variable: at top level (no enclosing) store in globals so REPL persists across lines
                        if self.enclosing.is_empty() {
                            self.globals.borrow_mut().insert(Arc::clone(name), v);
                        } else {
                            ls_get_or_init!().borrow_mut().insert(Arc::clone(name), v);
                        }
                    }
                }
                Opcode::DeclareVar => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    if let Some(frame) = block_undo_stack.last_mut() {
                        let old = local_scope
                            .as_ref()
                            .and_then(|ls| ls.borrow().get(name.as_ref()).cloned());
                        frame.push((Arc::clone(name), old));
                    }
                    // REPL: persist top-level bindings only (not block-locals shadowing globals).
                    if repl_mode && self.enclosing.is_empty() && block_undo_stack.is_empty() {
                        self.globals
                            .borrow_mut()
                            .insert(Arc::clone(name), v.clone());
                    }
                    ls_get_or_init!().borrow_mut().insert(Arc::clone(name), v);
                }
                Opcode::DeclareVarPlain => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    if repl_mode && self.enclosing.is_empty() && block_undo_stack.is_empty() {
                        self.globals
                            .borrow_mut()
                            .insert(Arc::clone(name), v.clone());
                    }
                    ls_get_or_init!().borrow_mut().insert(Arc::clone(name), v);
                }
                Opcode::EnterBlock => {
                    block_undo_stack.push(Vec::new());
                }
                Opcode::ExitBlock => {
                    let frame = block_undo_stack
                        .pop()
                        .ok_or_else(|| "ExitBlock without matching EnterBlock".to_string())?;
                    for (name, old) in frame.into_iter().rev() {
                        let mut ls = ls_get_or_init!().borrow_mut();
                        match old {
                            Some(prev) => {
                                ls.insert(name, prev);
                            }
                            None => {
                                ls.remove(name.as_ref());
                            }
                        }
                    }
                }
                Opcode::LoopVarsBegin => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    active_loop_vars.push(Arc::clone(name));
                }
                Opcode::LoopVarsEnd => {
                    active_loop_vars.pop();
                }
                Opcode::ArgMissing => {
                    // True iff the positional arg at `idx` was not supplied → the function
                    // prologue applies the param's default. Matches the interpreter: an
                    // explicit `null` arg is "supplied" and keeps the `null`.
                    let idx = Self::read_u16(code, &mut ip) as usize;
                    self.stack.push(Value::Bool(idx >= args.len()));
                }
                Opcode::LoadGlobal => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = self
                        .globals
                        .borrow()
                        .get(name.as_ref())
                        .cloned()
                        .ok_or_else(|| format!("Undefined global: {}", name))?;
                    self.stack.push(v);
                }
                Opcode::StoreGlobal => {
                    let idx = Self::read_u16(code, &mut ip);
                    let name = names
                        .get(idx as usize)
                        .ok_or_else(|| format!("Name index out of bounds: {}", idx))?;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    self.globals.borrow_mut().insert(Arc::clone(name), v);
                }
                Opcode::Pop => {
                    self.stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                }
                Opcode::PopN => {
                    let n = Self::read_u16(code, &mut ip) as usize;
                    for _ in 0..n {
                        self.stack
                            .pop()
                            .ok_or_else(|| "Stack underflow".to_string())?;
                    }
                }
                Opcode::Dup => {
                    let v = self
                        .stack
                        .last()
                        .ok_or_else(|| "Stack underflow".to_string())?
                        .clone();
                    self.stack.push(v);
                }
                Opcode::IterNormalize => {
                    // `for…of`: turn a JS iterator object (callable `next()` → `{ value, done }`,
                    // e.g. a Map/Set `.values()` result) into an array so the index loop iterates
                    // it. Arrays/strings/everything else pass through unchanged.
                    let v = self
                        .stack
                        .last()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    if let Some(items) = tishlang_core::drain_iterator(v) {
                        self.stack.pop();
                        self.stack.push(Value::Array(VmRef::new(items)));
                    }
                }
                Opcode::Call => {
                    let argc = Self::read_u16(code, &mut ip) as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(
                            self.stack
                                .pop()
                                .ok_or_else(|| "Stack underflow in call".to_string())?,
                        );
                    }
                    args.reverse();
                    let callee = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow: no callee".to_string())?;
                    // Call the function in place — no `Arc` clone on the hot direct-call path. The
                    // immutable borrow of `callee` is held only across the call, which never touches it.
                    let result = match &callee {
                        Value::Function(f) => {
                            // Frame-VM (flag-on): a frameable VmClosure runs on the heap frame stack
                            // (iterative, no per-call Vm / native recursion). Else the normal path.
                            if Self::frame_vm_enabled() {
                                match f.as_any().downcast_ref::<VmClosure>() {
                                    Some(vc) if Self::vmclosure_frameable(vc) => {
                                        match self.run_framed(vc, &args) {
                                            // A pending throw is handled by the post-call check
                                            // below (issue #60); a real fatal error propagates.
                                            Some(Ok(v)) => v,
                                            Some(Err(e)) if e == PENDING_THROW_SENTINEL => {
                                                Value::Null
                                            }
                                            Some(Err(e)) => return Err(e),
                                            None => f.call(&args),
                                        }
                                    }
                                    _ => f.call(&args),
                                }
                            } else {
                                f.call(&args)
                            }
                        }
                        Value::Object(o) => {
                            let call_fn = match o.borrow().strings.get("__call") {
                                Some(Value::Function(cf)) => cf.clone(),
                                _ => raise!(construct_builtin::error_object(
                                    "TypeError",
                                    &format!("Call of non-function: {}", callee.type_name())
                                )),
                            };
                            call_fn.call(&args)
                        }
                        _ => raise!(construct_builtin::error_object(
                            "TypeError",
                            &format!("Call of non-function: {}", callee.type_name())
                        )),
                    };
                    // A throw that escaped the callee's own `catch` is parked in the thread-local;
                    // surface it here so this frame's `try` (if any) can catch it (issue #60).
                    if let Some(v) = take_pending_throw() {
                        raise!(v);
                    }
                    self.stack.push(result);
                }
                Opcode::SelfCall => {
                    // Direct recursive call to the CURRENT function (`chunk`). The compiler emits
                    // this only when the function's own name is provably stable, so the callee is
                    // implicitly `chunk` — no callee on the stack, no name lookup, no closure
                    // dispatch. Behaviour matches `LoadVar name; Call argc` (a closure call that
                    // swallows errors to Null), and uses the SAME captured `enclosing`.
                    let argc = Self::read_u16(code, &mut ip) as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(
                            self.stack
                                .pop()
                                .ok_or_else(|| "Stack underflow in self-call".to_string())?,
                        );
                    }
                    args.reverse();
                    // #381: SelfCall is a second native recursive re-entry (a self-recursive function
                    // re-enters run_chunk directly). Bound it with the same shared counter; `raise!` is
                    // in scope here, so throw the catchable RangeError directly.
                    let depth = tishlang_core::inc_call_depth();
                    if depth > max_call_depth() {
                        tishlang_core::dec_call_depth();
                        raise!(stack_overflow_error());
                    }
                    let mut vm = Vm {
                        stack: Vec::new(),
                        scope: ObjectMap::default(),
                        enclosing: self.enclosing.clone(),
                        globals: self.globals.clone(),
                        capabilities: Arc::clone(&self.capabilities),
                        native_modules: self.native_modules.clone(),
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    let result = stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
                        vm.run_chunk(chunk, nested, &args, false)
                            .unwrap_or(Value::Null)
                    });
                    #[cfg(target_arch = "wasm32")]
                    let result = vm
                        .run_chunk(chunk, nested, &args, false)
                        .unwrap_or(Value::Null);
                    tishlang_core::dec_call_depth();
                    if let Some(v) = take_pending_throw() {
                        raise!(v);
                    }
                    self.stack.push(result);
                }
                Opcode::CallSpread => {
                    let callee = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow: no callee in CallSpread".to_string())?;
                    let args_array = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow in CallSpread".to_string())?;
                    // A lone iterator spread (`f(...m.values())`) — drain to an array.
                    let args_array = match tishlang_core::drain_iterator(&args_array) {
                        Some(items) => Value::Array(VmRef::new(items)),
                        None => args_array,
                    };
                    let args: Vec<Value> = match &args_array {
                        Value::Array(a) => a.borrow().clone(),
                        _ => {
                            return Err(format!(
                                "CallSpread: args must be array, got {}",
                                args_array.to_display_string()
                            ));
                        }
                    };
                    let f = match &callee {
                        Value::Function(f) => f.clone(),
                        Value::Object(o) => {
                            if let Some(Value::Function(call_fn)) = o.borrow().strings.get("__call")
                            {
                                call_fn.clone()
                            } else {
                                return Err(format!(
                                    "Call of non-function: {}",
                                    callee.type_name()
                                ));
                            }
                        }
                        _ => {
                            return Err(format!("Call of non-function: {}", callee.type_name()));
                        }
                    };
                    let result = f.call(&args);
                    if let Some(v) = take_pending_throw() {
                        raise!(v);
                    }
                    self.stack.push(result);
                }
                Opcode::Construct => {
                    let argc = Self::read_u16(code, &mut ip) as usize;
                    let mut args = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        args.push(
                            self.stack
                                .pop()
                                .ok_or_else(|| "Stack underflow in construct".to_string())?,
                        );
                    }
                    args.reverse();
                    let callee = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow: no callee for construct".to_string())?;
                    let result = construct_builtin::construct(&callee, &args);
                    if let Some(v) = take_pending_throw() {
                        raise!(v);
                    }
                    self.stack.push(result);
                }
                Opcode::ConstructSpread => {
                    let callee = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow: callee in ConstructSpread".to_string())?;
                    let args_array = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow in ConstructSpread".to_string())?;
                    // A lone iterator spread (`new X(...m.values())`) — drain to an array.
                    let args_array = match tishlang_core::drain_iterator(&args_array) {
                        Some(items) => Value::Array(VmRef::new(items)),
                        None => args_array,
                    };
                    let args: Vec<Value> = match &args_array {
                        Value::Array(a) => a.borrow().clone(),
                        _ => {
                            return Err(format!(
                                "ConstructSpread: args must be array, got {}",
                                args_array.to_display_string()
                            ));
                        }
                    };
                    let result = construct_builtin::construct(&callee, &args);
                    if let Some(v) = take_pending_throw() {
                        raise!(v);
                    }
                    self.stack.push(result);
                }
                Opcode::Return => {
                    let v = self.stack.pop().unwrap_or(Value::Null);
                    return Ok(v);
                }
                Opcode::Jump => {
                    let offset = Self::read_i16(code, &mut ip) as isize;
                    ip = (ip as isize + offset).max(0) as usize;
                }
                Opcode::JumpIfFalse => {
                    let offset = Self::read_i16(code, &mut ip) as isize;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    if !v.is_truthy() {
                        ip = (ip as isize + offset).max(0) as usize;
                    }
                }
                Opcode::JumpBack => {
                    let dist = Self::read_u16(code, &mut ip) as usize;
                    // #190 OSR: on a hot back-edge, try to finish the loop natively. Slot-based frames
                    // only (`slot_locals` is the live frame); non-slot / non-numeric loops fail the
                    // whitelist or the live-in check and fall straight through to the interpreter.
                    #[cfg(not(target_arch = "wasm32"))]
                    if chunk.slot_based && Self::osr_enabled() {
                        let region_end = ip;
                        let header_ip = ip.saturating_sub(dist);
                        // Switch the fast slot only when the active loop changes: stash the old count,
                        // load this header's (default 0). The hot loop keeps `header_ip == osr_hot.0`.
                        if header_ip != osr_hot.0 {
                            if osr_hot.0 != usize::MAX {
                                osr_counters.insert(osr_hot.0, osr_hot.1);
                            }
                            osr_hot = (
                                header_ip,
                                osr_counters.get(&header_ip).copied().unwrap_or(0),
                            );
                        }
                        if osr_hot.1 != u32::MAX {
                            osr_hot.1 = osr_hot.1.saturating_add(1);
                            if osr_hot.1 >= OSR_THRESHOLD
                                && osr_hot
                                    .1
                                    .saturating_sub(OSR_THRESHOLD)
                                    .is_multiple_of(OSR_RETRY)
                            {
                                match Self::run_osr(
                                    chunk,
                                    header_ip,
                                    region_end,
                                    &mut slot_locals,
                                    0,
                                ) {
                                    OsrResult::Compiled(exit_ip) => {
                                        ip = exit_ip;
                                        continue;
                                    }
                                    OsrResult::LiveInMiss => {}
                                    OsrResult::NotCompilable => osr_hot.1 = u32::MAX,
                                }
                            }
                        }
                    }
                    ip = ip.saturating_sub(dist);
                }
                Opcode::BinOp => {
                    let op_u8 = Self::read_u16(code, &mut ip) as u8;
                    let r = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let l = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let op =
                        u8_to_binop(op_u8).ok_or_else(|| format!("Unknown binop: {}", op_u8))?;
                    let result = eval_binop(op, &l, &r)?;
                    self.stack.push(result);
                }
                Opcode::UnaryOp => {
                    let op_u8 = Self::read_u16(code, &mut ip) as u8;
                    let o = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let op = u8_to_unaryop(op_u8)
                        .ok_or_else(|| format!("Unknown unary op: {}", op_u8))?;
                    let result = eval_unary(op, &o)?;
                    self.stack.push(result);
                }
                Opcode::GetMember => {
                    let idx = Self::read_u16(code, &mut ip);
                    let key = names
                        .get(idx as usize)
                        .ok_or_else(|| "Name index out of bounds".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let v = catchable!(ic_get_member(chunk, idx, &obj, key));
                    self.stack.push(v);
                }
                Opcode::GetMemberOptional => {
                    let idx = Self::read_u16(code, &mut ip);
                    let key = names
                        .get(idx as usize)
                        .ok_or_else(|| "Name index out of bounds".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let v = ic_get_member(chunk, idx, &obj, key).unwrap_or(Value::Null);
                    self.stack.push(v);
                }
                Opcode::SetMember => {
                    let idx = Self::read_u16(code, &mut ip);
                    let key = names
                        .get(idx as usize)
                        .ok_or_else(|| "Name index out of bounds".to_string())?;
                    let val = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    catchable!(ic_set_member(chunk, idx, &obj, key, val.clone()));
                    self.stack.push(val); // assignment yields value
                }
                Opcode::GetIndex => {
                    let idx_val = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let v = catchable!(get_index(&obj, &idx_val));
                    self.stack.push(v);
                }
                Opcode::SetIndex => {
                    // Stack: [obj, idx, val, val] (Dup of val for expression result).
                    // Pop val (dup), val, idx, obj; use (obj, idx, val) for set_index; leave val on stack.
                    let dup_val = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let val = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let idx_val = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    catchable!(set_index(&obj, &idx_val, val.clone()));
                    self.stack.push(dup_val); // assignment yields the assigned value
                }
                Opcode::DeleteIndex => {
                    // `delete obj[key]` / `delete obj.prop`: pop [obj, key], remove, push true.
                    let key = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let obj = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    delete_index(&obj, &key);
                    self.stack.push(Value::Bool(true));
                }
                Opcode::NewArray => {
                    let n = Self::read_u16(code, &mut ip) as usize;
                    let mut elems = Vec::with_capacity(n);
                    for _ in 0..n {
                        elems.push(
                            self.stack
                                .pop()
                                .ok_or_else(|| "Stack underflow".to_string())?,
                        );
                    }
                    elems.reverse();
                    // Packed-array fast path: if every element is a number AND there is at
                    // least one element, store as Vec<f64>. Empty arrays stay as Value::Array
                    // because they are commonly used as general-purpose containers (the type
                    // can't be inferred from zero elements).
                    if Value::packed_arrays_enabled() && !elems.is_empty() {
                        if let Some(nums) = elems.iter().try_fold(
                            Vec::<f64>::with_capacity(elems.len()),
                            |mut acc, v| {
                                if let Value::Number(n) = v {
                                    acc.push(*n);
                                    Some(acc)
                                } else {
                                    None
                                }
                            },
                        ) {
                            self.stack.push(Value::number_array(nums));
                        } else {
                            self.stack.push(Value::Array(VmRef::new(elems)));
                        }
                    } else {
                        self.stack.push(Value::Array(VmRef::new(elems)));
                    }
                }
                Opcode::NewObject => {
                    let n = Self::read_u16(code, &mut ip) as usize;
                    if self.stack.len() < 2 * n {
                        return Err("Stack underflow".to_string());
                    }
                    // Pairs sit on the stack in source order: key1,val1,…,keyN,valN. Read them
                    // in place into the PropMap (insertion order = JS order) and drop them in
                    // one truncate — no intermediate Vec per object literal (a hot path: every
                    // `{...}` and every HTTP JSON response).
                    let base = self.stack.len() - 2 * n;
                    let mut map = PropMap::with_capacity(n);
                    for i in 0..n {
                        let key_val = std::mem::replace(&mut self.stack[base + 2 * i], Value::Null);
                        let val = std::mem::replace(&mut self.stack[base + 2 * i + 1], Value::Null);
                        let key: Arc<str> = key_val.to_display_string().into();
                        map.insert(key, val);
                    }
                    self.stack.truncate(base);
                    self.stack.push(Value::Object(VmRef::new(ObjectData {
                        strings: map,
                        symbols: None,
                    })));
                }
                Opcode::EnterTry => {
                    let offset = Self::read_u16(code, &mut ip) as usize;
                    let catch_ip = ip + offset;
                    try_handlers.push((catch_ip, self.stack.len()));
                }
                Opcode::ExitTry => {
                    try_handlers.pop();
                }
                Opcode::ConcatArray => {
                    let right = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let left = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    // Materialise NumberArray on either side before concatenation.
                    let left = left.coerce_number_array();
                    let right = right.coerce_number_array();
                    // Spread of a Map/Set iterator (`[...m.values()]`): drain to an array.
                    let left = match tishlang_core::drain_iterator(&left) {
                        Some(items) => Value::Array(VmRef::new(items)),
                        None => left,
                    };
                    let right = match tishlang_core::drain_iterator(&right) {
                        Some(items) => Value::Array(VmRef::new(items)),
                        None => right,
                    };
                    let (mut a, b) = (
                        match &left {
                            Value::Array(arr) => arr.borrow().clone(),
                            _ => {
                                return Err(format!(
                                    "ConcatArray: left must be array, got {}",
                                    left.to_display_string()
                                ));
                            }
                        },
                        match &right {
                            Value::Array(arr) => arr.borrow().clone(),
                            _ => {
                                return Err(format!(
                                    "ConcatArray: right must be array, got {}",
                                    right.to_display_string()
                                ));
                            }
                        },
                    );
                    a.extend(b);
                    self.stack.push(Value::Array(VmRef::new(a)));
                }
                Opcode::MergeObject => {
                    let right = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let left = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    match (&left, &right) {
                        (Value::Object(l), Value::Object(r)) => {
                            let merged = merge_object_data(l, r);
                            self.stack.push(Value::Object(VmRef::new(merged)));
                        }
                        _ => {
                            return Err(format!(
                                "MergeObject: expected two objects, got {} and {}",
                                left.to_display_string(),
                                right.to_display_string()
                            ));
                        }
                    }
                }
                Opcode::ArraySortNumeric => {
                    let operand = Self::read_u16(code, &mut ip);
                    let asc = operand == 0;
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let result = if asc {
                        arr_builtins::sort_numeric_asc(&arr)
                    } else {
                        arr_builtins::sort_numeric_desc(&arr)
                    };
                    self.stack.push(result);
                }
                Opcode::ArraySortByProperty => {
                    let prop_idx = Self::read_u16(code, &mut ip);
                    let asc = Self::read_u16(code, &mut ip) == 0;
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let prop = constants
                        .get(prop_idx as usize)
                        .and_then(|c| {
                            if let Constant::String(s) = c {
                                Some(s.as_ref())
                            } else {
                                None
                            }
                        })
                        .unwrap_or("");
                    let result = arr_builtins::sort_by_property_numeric(&arr, prop, asc);
                    self.stack.push(result);
                }
                Opcode::ArrayMapIdentity => {
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let result = match &arr {
                        Value::Array(a) => Value::Array(VmRef::new(a.borrow().clone())),
                        // Identity map on a NumberArray = clone the packed vec (stays packed).
                        Value::NumberArray(a) => Value::NumberArray(VmRef::new(a.borrow().clone())),
                        _ => Value::Null,
                    };
                    self.stack.push(result);
                }
                Opcode::ArrayMapBinOp => {
                    let binop_u8 = code[ip];
                    ip += 1;
                    let const_idx = Self::read_u16(code, &mut ip);
                    let param_left = code[ip] == 0; // 0 = param on left (x op const), 1 = param on right (const op x)
                    ip += 1;
                    let binop = u8_to_binop(binop_u8)
                        .ok_or_else(|| format!("Unknown binop in ArrayMapBinOp: {}", binop_u8))?;
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let const_val = constants
                        .get(const_idx as usize)
                        .map(|c| c.to_value())
                        .unwrap_or(Value::Null);
                    let result = match &arr {
                        Value::NumberArray(a) => {
                            // All-numeric fast path: operate on raw f64, no boxing/unboxing.
                            let arr_borrow = a.borrow();
                            let mapped: Vec<Value> = arr_borrow
                                .iter()
                                .map(|&n| {
                                    let elem = Value::Number(n);
                                    let (l, r) = if param_left {
                                        (elem, const_val.clone())
                                    } else {
                                        (const_val.clone(), elem)
                                    };
                                    eval_binop(binop, &l, &r).unwrap_or(Value::Null)
                                })
                                .collect();
                            // If every result is numeric, stay packed (the common case for x*2, x+1, etc).
                            if mapped.iter().all(|v| matches!(v, Value::Number(_))) {
                                Value::number_array(
                                    mapped
                                        .into_iter()
                                        .map(|v| match v {
                                            Value::Number(n) => n,
                                            _ => unreachable!(),
                                        })
                                        .collect(),
                                )
                            } else {
                                Value::Array(VmRef::new(mapped))
                            }
                        }
                        Value::Array(a) => {
                            let arr_borrow = a.borrow();
                            let mapped: Vec<Value> = arr_borrow
                                .iter()
                                .map(|v| {
                                    let l: Value = if param_left {
                                        (*v).clone()
                                    } else {
                                        const_val.clone()
                                    };
                                    let r: Value = if param_left {
                                        const_val.clone()
                                    } else {
                                        (*v).clone()
                                    };
                                    eval_binop(binop, &l, &r).unwrap_or(Value::Null)
                                })
                                .collect();
                            Value::Array(VmRef::new(mapped))
                        }
                        _ => Value::Null,
                    };
                    self.stack.push(result);
                }
                Opcode::ArrayFilterBinOp => {
                    let binop_u8 = code[ip];
                    ip += 1;
                    let const_idx = Self::read_u16(code, &mut ip);
                    let param_left = code[ip] == 0; // 0 = param on left (x op const), 1 = param on right (const op x)
                    ip += 1;
                    let binop = u8_to_binop(binop_u8).ok_or_else(|| {
                        format!("Unknown binop in ArrayFilterBinOp: {}", binop_u8)
                    })?;
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let const_val = constants
                        .get(const_idx as usize)
                        .map(|c| c.to_value())
                        .unwrap_or(Value::Null);
                    let result = match &arr {
                        Value::NumberArray(a) => {
                            let arr_borrow = a.borrow();
                            let filtered: Vec<f64> = arr_borrow
                                .iter()
                                .filter(|&&n| {
                                    let elem = Value::Number(n);
                                    let (l, r) = if param_left {
                                        (elem, const_val.clone())
                                    } else {
                                        (const_val.clone(), elem)
                                    };
                                    eval_binop(binop, &l, &r).unwrap_or(Value::Null).is_truthy()
                                })
                                .copied()
                                .collect();
                            Value::number_array(filtered)
                        }
                        Value::Array(a) => {
                            let arr_borrow = a.borrow();
                            let filtered: Vec<Value> = arr_borrow
                                .iter()
                                .filter(|v| {
                                    let (l, r) = if param_left {
                                        ((*v).clone(), const_val.clone())
                                    } else {
                                        (const_val.clone(), (*v).clone())
                                    };
                                    eval_binop(binop, &l, &r).unwrap_or(Value::Null).is_truthy()
                                })
                                .cloned()
                                .collect();
                            Value::Array(VmRef::new(filtered))
                        }
                        _ => Value::Null,
                    };
                    self.stack.push(result);
                }
                Opcode::Throw => {
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    raise!(v);
                }
                Opcode::AwaitPromise => {
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow in AwaitPromise".to_string())?;
                    #[cfg(any(feature = "http", feature = "promise"))]
                    {
                        use tishlang_core::Value as V;
                        match v {
                            V::Promise(p) => match p.block_until_settled() {
                                Ok(val) => self.stack.push(val),
                                Err(rej) => {
                                    Self::unwind_throw(
                                        &mut try_handlers,
                                        &mut self.stack,
                                        &mut ip,
                                        rej,
                                    )?;
                                }
                            },
                            other => self.stack.push(tishlang_runtime::await_promise(other)),
                        }
                    }
                    #[cfg(not(any(feature = "http", feature = "promise")))]
                    {
                        self.stack.push(v);
                    }
                }
                Opcode::LoadNativeExport => {
                    let spec_idx = Self::read_u16(code, &mut ip);
                    let export_idx = Self::read_u16(code, &mut ip);
                    let spec = match constants.get(spec_idx as usize) {
                        Some(Constant::String(s)) => s.as_ref(),
                        _ => {
                            return Err(
                                "LoadNativeExport: spec constant out of bounds or not string"
                                    .to_string(),
                            );
                        }
                    };
                    let export_name = match constants.get(export_idx as usize) {
                        Some(Constant::String(s)) => s.as_ref(),
                        _ => {
                            return Err("LoadNativeExport: export_name constant out of bounds or not string".to_string());
                        }
                    };
                    // Phase-2 item 11: consult externally registered native
                    // modules (populated via `Vm::register_native_module`)
                    // before falling through to the built-in lookup. Embedders
                    // on the cranelift / llvm backends that want to expose
                    // `cargo:…` Rust crates should register the module's
                    // exports map before calling `vm.run(chunk)`.
                    let from_registry: Option<Value> =
                        if spec.starts_with("cargo:") || spec.starts_with("ffi:") {
                            let regs = self.native_modules.borrow();
                            regs.get(spec)
                                .and_then(|m| m.borrow().get(&Arc::from(export_name)).cloned())
                        } else {
                            None
                        };
                    let v = from_registry
                        .or_else(|| get_builtin_export(self.capabilities.as_ref(), spec, export_name))
                        .ok_or_else(|| {
                            if spec.starts_with("cargo:") {
                                format!(
                                    "cargo:{} is not registered on the bytecode VM. Embedders must call Vm::register_native_module before run(). Spec: {} export: {}",
                                    spec.trim_start_matches("cargo:"),
                                    spec,
                                    export_name,
                                )
                            } else {
                                format!(
                                    "Built-in module '{}' does not export '{}' or capability not enabled for this run. Use e.g. tish run --feature fs (or full). The tish binary must also be built with that capability linked in.",
                                    spec, export_name
                                )
                            }
                        })?;
                    self.stack.push(v);
                }
                Opcode::Closure | Opcode::LoadThis => {
                    return Err(format!("Unhandled opcode: {:?}", opcode));
                }
            }
        }

        #[cfg(feature = "timers")]
        if cap_allows(self.capabilities.as_ref(), "timers") {
            tishlang_runtime::drain_timers();
        }

        Ok(self.stack.pop().unwrap_or(Value::Null))
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// Rough byte capacity for string coercion (matches hot paths like `"x" + n + "ms"`).
fn estimate_string_concat_len(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Number(_) => 24,
        Value::Bool(_) => 5,
        Value::Null => 4,
        _ => 32,
    }
}

/// Append JS-style string conversion without an intermediate `String` per operand (unlike
/// `format!("{}{}", a.to_display_string(), b.to_display_string())`, which triple-allocates).
fn append_value_for_string_concat(out: &mut String, v: &Value) {
    match v {
        // JS `Number.prototype.toString` (exponential past digit 21 / before −6), shared
        // with `console.log` so `"" + n` and `` `${n}` `` match Node exactly.
        Value::Number(n) => out.push_str(&tishlang_core::js_number_to_string(*n)),
        Value::String(s) => out.push_str(s.as_ref()),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Null => out.push_str("null"),
        // Arrays/objects use JS `ToString` (recursive comma-join / "[object Object]"),
        // not the inspect form, so `"" + [1,[2,3]]` and templates match Node.
        _ => out.push_str(&v.to_js_string()),
    }
}

fn eval_binop(op: BinOp, l: &Value, r: &Value) -> Result<Value, String> {
    use tishlang_ast::BinOp::*;
    use tishlang_core::Value::*;
    let ln = l.as_number().unwrap_or(f64::NAN);
    let rn = r.as_number().unwrap_or(f64::NAN);
    match op {
        Add => {
            if matches!(l, Value::String(_)) || matches!(r, Value::String(_)) {
                let cap = estimate_string_concat_len(l) + estimate_string_concat_len(r);
                let mut buf = std::string::String::with_capacity(cap);
                append_value_for_string_concat(&mut buf, l);
                append_value_for_string_concat(&mut buf, r);
                Ok(String(buf.into()))
            } else {
                Ok(Number(ln + rn))
            }
        }
        Sub => Ok(Number(ln - rn)),
        Mul => Ok(Number(ln * rn)),
        // IEEE division/remainder, matching JS (and the interp + rust-AOT backends): `5/0` → Infinity,
        // `-5/0` → -Infinity, `0/0` → NaN, `5%0` → NaN. The former `if rn==0 { NaN }` special-case made
        // the VM the only backend that returned NaN for `n/0` at runtime (literals were masked by
        // constant-folding) — a cross-backend divergence. Null/non-number operands already coerce to
        // NaN via `as_number().unwrap_or(NaN)` above, so `5/null` stays NaN (tish's null-coercion).
        Div => Ok(Number(ln / rn)),
        Mod => Ok(Number(ln % rn)),
        Pow => Ok(Number(ln.powf(rn))),
        Eq => Ok(Bool(l.strict_eq(r))),
        Ne => Ok(Bool(!l.strict_eq(r))),
        StrictEq => Ok(Bool(l.strict_eq(r))),
        StrictNe => Ok(Bool(!l.strict_eq(r))),
        // Relational operators: when BOTH operands are strings, compare them
        // lexicographically (JS semantics). Otherwise coerce to numbers — a string
        // mixed with a number still falls through to numeric coercion (NaN → false).
        Lt => Ok(Bool(match (l, r) {
            (String(a), String(b)) => a.as_str() < b.as_str(),
            _ => ln < rn,
        })),
        Le => Ok(Bool(match (l, r) {
            (String(a), String(b)) => a.as_str() <= b.as_str(),
            _ => ln <= rn,
        })),
        Gt => Ok(Bool(match (l, r) {
            (String(a), String(b)) => a.as_str() > b.as_str(),
            _ => ln > rn,
        })),
        Ge => Ok(Bool(match (l, r) {
            (String(a), String(b)) => a.as_str() >= b.as_str(),
            _ => ln >= rn,
        })),
        And => Ok(Bool(l.is_truthy() && r.is_truthy())),
        Or => Ok(Bool(l.is_truthy() || r.is_truthy())),
        // `to_int32`/`to_uint32` = JS ToInt32/ToUint32 (modulo 2³², NaN/±Infinity → 0); not a
        // saturating cast, so out-of-range operands wrap exactly like JS instead of clamping.
        BitAnd => Ok(Number((to_int32(ln) & to_int32(rn)) as f64)),
        BitOr => Ok(Number((to_int32(ln) | to_int32(rn)) as f64)),
        BitXor => Ok(Number((to_int32(ln) ^ to_int32(rn)) as f64)),
        // JS shifts mask the count to 5 bits; `wrapping_sh*` matches that and avoids
        // the debug-mode panic that plain `<<`/`>>` raise for a count of 32+.
        Shl => Ok(Number(to_int32(ln).wrapping_shl(to_uint32(rn)) as f64)),
        Shr => Ok(Number(to_int32(ln).wrapping_shr(to_uint32(rn)) as f64)),
        UShr => Ok(Number(to_uint32(ln).wrapping_shr(to_uint32(rn)) as f64)),
        In => Ok(Bool(match r {
            Value::Object(_) => object_has(r, l),
            Value::Array(a) => {
                let key_s: Arc<str> = match l {
                    Value::String(s) => Arc::from(s.as_str()),
                    Value::Number(n) => n.to_string().into(),
                    _ => l.to_display_string().into(),
                };
                if key_s.as_ref() == "length" {
                    true
                } else if let Ok(idx) = key_s.parse::<usize>() {
                    idx < a.borrow().len()
                } else {
                    false
                }
            }
            Value::NumberArray(a) => {
                let key_s: Arc<str> = match l {
                    Value::String(s) => Arc::from(s.as_str()),
                    Value::Number(n) => n.to_string().into(),
                    _ => l.to_display_string().into(),
                };
                if key_s.as_ref() == "length" {
                    true
                } else if let Ok(idx) = key_s.parse::<usize>() {
                    idx < a.borrow().len()
                } else {
                    false
                }
            }
            _ => false,
        })),
    }
}

fn eval_unary(op: UnaryOp, o: &Value) -> Result<Value, String> {
    use tishlang_ast::UnaryOp::*;
    use tishlang_core::Value::*;
    match op {
        Not => Ok(Bool(!o.is_truthy())),
        Neg => Ok(Number(-o.as_number().unwrap_or(f64::NAN))),
        Pos => Ok(Number(o.as_number().unwrap_or(f64::NAN))),
        BitNot => Ok(Number(!to_int32(o.as_number().unwrap_or(0.0)) as f64)),
        Void => Ok(Null),
    }
}

/// `GetMember` with the per-name inline cache (JSC-style, Phase 1a). On a shape hit the property is at
/// a cached slot index → a direct load, no key hash/compare. A miss (or a non-plain-object, or a
/// `DICT_SHAPE` object) falls to [`get_member`] (arrays/strings/`length`/methods/missing-property
/// error), refilling the cache when the object *does* have the property. Result-equivalent to
/// `get_member` — the cache only skips the lookup; the shape uniquely fixes the slot for a property.
#[inline]
fn ic_get_member(
    chunk: &Chunk,
    name_idx: u16,
    obj: &Value,
    key: &Arc<str>,
) -> Result<Value, String> {
    use std::sync::atomic::Ordering::Relaxed;
    if let Value::Object(od) = obj {
        let b = od.borrow();
        let shape = b.strings.shape();
        if shape != tishlang_core::DICT_SHAPE {
            if let Some(cell) = chunk.inline_caches.0.get(name_idx as usize) {
                let ic = cell.load(Relaxed);
                let cached_shape = (ic >> 32) as u32; // 0 == uncached
                if cached_shape != 0 && cached_shape == shape {
                    if let Some(v) = b.strings.value_at_index((ic & 0xffff_ffff) as usize) {
                        return Ok(v.clone());
                    }
                }
                // Miss: do the real lookup once, and if the property exists, cache its slot.
                if let Some((v, i)) = b.strings.get_with_index(key.as_ref()) {
                    cell.store(((shape as u64) << 32) | i as u64, Relaxed);
                    return Ok(v.clone());
                }
            }
        }
        // `b` drops at the end of this block → safe to re-borrow `obj` in `get_member` below.
    }
    get_member(obj, key)
}

/// `SetMember` with the per-name inline cache. On a shape hit for an existing property → an in-place
/// store at the cached slot (no key lookup, no shape change). Otherwise the slow path inserts (a new
/// key transitions the shape) and refills the cache. Non-objects fall to [`set_member`].
#[inline]
fn ic_set_member(
    chunk: &Chunk,
    name_idx: u16,
    obj: &Value,
    key: &Arc<str>,
    val: Value,
) -> Result<(), String> {
    use std::sync::atomic::Ordering::Relaxed;
    if let Value::Object(od) = obj {
        let mut b = od.borrow_mut();
        let shape = b.strings.shape();
        let cell = chunk.inline_caches.0.get(name_idx as usize);
        if shape != tishlang_core::DICT_SHAPE {
            if let Some(c) = cell {
                let ic = c.load(Relaxed);
                let cached_shape = (ic >> 32) as u32;
                if cached_shape != 0 && cached_shape == shape {
                    if let Some(slot) = b.strings.value_at_index_mut((ic & 0xffff_ffff) as usize) {
                        *slot = val; // existing property, same shape → in-place update
                        return Ok(());
                    }
                }
            }
        }
        // Slow path: insert (a new key transitions the shape) + refill the cache for next time.
        b.strings.insert(Arc::clone(key), val);
        if let Some(c) = cell {
            let ns = b.strings.shape();
            if ns != tishlang_core::DICT_SHAPE {
                if let Some((_, i)) = b.strings.get_with_index(key.as_ref()) {
                    c.store(((ns as u64) << 32) | i as u64, Relaxed);
                }
            }
        }
        return Ok(());
    }
    set_member(obj, key, val)
}

fn get_member(obj: &Value, key: &Arc<str>) -> Result<Value, String> {
    match obj {
        Value::Object(m) => {
            // `Set`/`Map` instances expose a computed `.size` (via a hidden `SizeProbe` opaque).
            if key.as_ref() == "size" {
                if let Some(n) = tishlang_builtins::collections::collection_size(obj) {
                    return Ok(Value::Number(n));
                }
            }
            let map = m.borrow();
            // Reading a missing own property returns `null` (tish's nullish value), matching
            // JS object semantics and the tree-walk interpreter — not a thrown error (#66).
            Ok(map
                .strings
                .get(key.as_ref())
                .cloned()
                .unwrap_or(Value::Null))
        }
        Value::NumberArray(a) => {
            let key_s = key.as_ref();
            // Numeric index fast path.
            if let Ok(idx) = key_s.parse::<usize>() {
                return Ok(a
                    .borrow()
                    .get(idx)
                    .map(|&n| Value::Number(n))
                    .unwrap_or(Value::Null));
            }
            if key_s == "length" {
                return Ok(Value::Number(a.borrow().len() as f64));
            }
            // push/pop/sort — stay packed; everything else materialise + delegate.
            let a_clone = a.clone();
            let method: ArrayMethodFn = match key_s {
                "push" => make_native_fn(move |args: &[Value]| {
                    let mut arr = a_clone.borrow_mut();
                    for v in args {
                        match v {
                            Value::Number(n) => arr.push(*n),
                            _ => {
                                arr.push(f64::NAN); // hole-marker for non-numeric
                            }
                        }
                    }
                    Value::Number(arr.len() as f64)
                }),
                "pop" => make_native_fn(move |_: &[Value]| {
                    a_clone
                        .borrow_mut()
                        .pop()
                        .map(|n| {
                            if n.is_nan() {
                                Value::Null
                            } else {
                                Value::Number(n)
                            }
                        })
                        .unwrap_or(Value::Null)
                }),
                "shift" => make_native_fn(move |_: &[Value]| {
                    let mut arr = a_clone.borrow_mut();
                    if arr.is_empty() {
                        Value::Null
                    } else {
                        let n = arr.remove(0);
                        if n.is_nan() {
                            Value::Null
                        } else {
                            Value::Number(n)
                        }
                    }
                }),
                "unshift" => make_native_fn(move |args: &[Value]| {
                    let mut arr = a_clone.borrow_mut();
                    for (i, v) in args.iter().enumerate() {
                        let n = match v {
                            Value::Number(n) => *n,
                            _ => f64::NAN,
                        };
                        arr.insert(i, n);
                    }
                    Value::Number(arr.len() as f64)
                }),
                "reverse" => make_native_fn(move |_: &[Value]| {
                    a_clone.borrow_mut().reverse();
                    Value::NumberArray(a_clone.clone())
                }),
                "splice" => {
                    let a2 = a_clone.clone();
                    make_native_fn(move |args: &[Value]| {
                        // Check if there are non-numeric items to insert (args[2..]).
                        let has_non_numeric = args
                            .get(2..)
                            .unwrap_or(&[])
                            .iter()
                            .any(|v| !matches!(v, Value::Number(_)));
                        if has_non_numeric {
                            // Deopt: materialise, splice on the boxed array, then write numeric
                            // elements back to the original Vec<f64>. This preserves the VmRef
                            // identity for subsequent accesses. The array may have non-numeric
                            // elements after this splice — they become NaN holes in the VmRef.
                            let boxed = Value::materialize_number_array(&a2);
                            let result = arr_builtins::splice(
                                &boxed,
                                args.first().unwrap_or(&Value::Null),
                                args.get(1),
                                args.get(2..).unwrap_or(&[]),
                            );
                            // Sync the modified boxed Vec back into the original VmRef.
                            if let Value::Array(boxed_vmref) = &boxed {
                                let mut packed = a2.borrow_mut();
                                *packed = boxed_vmref
                                    .borrow()
                                    .iter()
                                    .map(|v| match v {
                                        Value::Number(n) => *n,
                                        _ => f64::NAN,
                                    })
                                    .collect();
                            }
                            result
                        } else {
                            let mut arr = a2.borrow_mut();
                            let len = arr.len() as i64;
                            let start = match args.first() {
                                Some(Value::Number(n)) => {
                                    let s = *n as i64;
                                    if s < 0 {
                                        (len + s).max(0) as usize
                                    } else {
                                        (s as usize).min(arr.len())
                                    }
                                }
                                _ => 0,
                            };
                            let del = match args.get(1) {
                                Some(Value::Number(n)) => (*n as i64).max(0) as usize,
                                _ => arr.len().saturating_sub(start),
                            };
                            let del = del.min(arr.len().saturating_sub(start));
                            let new_nums: Vec<f64> = args
                                .get(2..)
                                .unwrap_or(&[])
                                .iter()
                                .map(|v| match v {
                                    Value::Number(n) => *n,
                                    _ => f64::NAN,
                                })
                                .collect();
                            let removed: Vec<f64> =
                                arr.splice(start..start + del, new_nums).collect();
                            Value::number_array(removed)
                        }
                    })
                }
                "sort" => make_native_fn(move |args: &[Value]| {
                    let arr_val = Value::NumberArray(a_clone.clone());
                    let cmp = args.first();
                    if let Some(Value::Function(_)) = cmp {
                        // Comparator sort: materialise first (comparator may return non-numeric).
                        let boxed = Value::materialize_number_array(&a_clone);
                        arr_builtins::sort_with_comparator(&boxed, cmp.unwrap())
                    } else {
                        arr_builtins::sort_numeric_asc(&arr_val)
                    }
                }),
                _ => {
                    // All other methods: materialise to a boxed Array and delegate.
                    // The a_clone is the original NumberArray VmRef; we materialise once per
                    // method lookup (not per call) so the closure captures a stable boxed Array.
                    let boxed = Value::materialize_number_array(&a_clone);
                    let bv = boxed.clone();
                    match key_s {
                        "map" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            // #187 fusion over the SAME boxed snapshot `bv` the generic path uses, so a
                            // fused result is byte-identical (a boxed all-number `Array` → boxed output).
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(v) = hof_fusion::map(&bv, &cb) {
                                return v;
                            }
                            arr_builtins::map(&bv, &cb)
                        }),
                        "filter" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(v) = hof_fusion::filter(&bv, &cb) {
                                return v;
                            }
                            arr_builtins::filter(&bv, &cb)
                        }),
                        "reduce" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            let init = args.get(1).cloned(); // None = no initial value (empty array → throw)
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(v) = hof_fusion::reduce(&bv, &cb, init.as_ref()) {
                                return v;
                            }
                            arr_builtins::reduce(&bv, &cb, init.as_ref())
                        }),
                        "forEach" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(v) = hof_fusion::for_each(&bv, &cb) {
                                return v;
                            }
                            arr_builtins::for_each(&bv, &cb)
                        }),
                        "find" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::find(&bv, &cb)
                        }),
                        "findIndex" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::find_index(&bv, &cb)
                        }),
                        "findLast" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::find_last(&bv, &cb)
                        }),
                        "findLastIndex" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::find_last_index(&bv, &cb)
                        }),
                        "at" => make_native_fn(move |args| {
                            let i = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::at(&bv, &i)
                        }),
                        "some" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::some(&bv, &cb)
                        }),
                        "every" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::every(&bv, &cb)
                        }),
                        "join" => make_native_fn(move |args| {
                            let sep = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::join(&bv, &sep)
                        }),
                        "flat" => make_native_fn(move |args| {
                            let d = args.first().cloned().unwrap_or(Value::Number(1.0));
                            arr_builtins::flat(&bv, &d)
                        }),
                        "flatMap" => make_native_fn(move |args| {
                            let cb = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::flat_map(&bv, &cb)
                        }),
                        "reverse" => make_native_fn(move |_| arr_builtins::reverse(&bv)),
                        "fill" => make_native_fn(move |args| {
                            let v = args.first().cloned().unwrap_or(Value::Null);
                            let s = args.get(1).cloned().unwrap_or(Value::Null);
                            let e = args.get(2).cloned().unwrap_or(Value::Null);
                            arr_builtins::fill(&bv, &v, &s, &e)
                        }),
                        "slice" => make_native_fn(move |args| {
                            let s = args.first().cloned().unwrap_or(Value::Null);
                            let e = args.get(1).cloned().unwrap_or(Value::Null);
                            arr_builtins::slice(&bv, &s, &e)
                        }),
                        "concat" => make_native_fn(move |args| arr_builtins::concat(&bv, args)),
                        "indexOf" => make_native_fn(move |args| {
                            let s = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::index_of(&bv, &s, args.get(1))
                        }),
                        "lastIndexOf" => make_native_fn(move |args| {
                            let s = args.first().cloned().unwrap_or(Value::Null);
                            arr_builtins::last_index_of(&bv, &s, args.get(1))
                        }),
                        "copyWithin" => make_native_fn(move |args| {
                            let t = args.first().cloned().unwrap_or(Value::Null);
                            let s = args.get(1).cloned().unwrap_or(Value::Null);
                            let e = args.get(2).cloned().unwrap_or(Value::Null);
                            arr_builtins::copy_within(&bv, &t, &s, &e)
                        }),
                        "includes" => make_native_fn(move |args| {
                            let s = args.first().cloned().unwrap_or(Value::Null);
                            let f = args.get(1).cloned();
                            arr_builtins::includes(&bv, &s, f.as_ref())
                        }),
                        "unshift" => make_native_fn(move |args| arr_builtins::unshift(&bv, args)),
                        "shift" => make_native_fn(move |_| arr_builtins::shift(&bv)),
                        "splice" => make_native_fn(move |args| {
                            let s = args.first().cloned().unwrap_or(Value::Null);
                            let dc = args.get(1).cloned();
                            let items: Vec<Value> = args.get(2..).unwrap_or(&[]).to_vec();
                            arr_builtins::splice(&bv, &s, dc.as_ref(), &items)
                        }),
                        _ => return Err(format!("Property '{}' not found", key)),
                    }
                }
            };
            Ok(Value::Function(method))
        }
        Value::Array(a) => {
            let key_s = key.as_ref();
            if let Ok(idx) = key_s.parse::<usize>() {
                let arr = a.borrow();
                return arr
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| "Index out of bounds".to_string());
            }
            if key_s == "length" {
                return Ok(Value::Number(a.borrow().len() as f64));
            }
            let a_clone = a.clone();
            let method: ArrayMethodFn = match key_s {
                "push" => make_native_fn(move |args: &[Value]| {
                    arr_builtins::push(&Value::Array(a_clone.clone()), args)
                }),
                "pop" => make_native_fn(move |_args: &[Value]| {
                    arr_builtins::pop(&Value::Array(a_clone.clone()))
                }),
                "shift" => make_native_fn(move |_args: &[Value]| {
                    arr_builtins::shift(&Value::Array(a_clone.clone()))
                }),
                "unshift" => make_native_fn(move |args: &[Value]| {
                    arr_builtins::unshift(&Value::Array(a_clone.clone()), args)
                }),
                "reverse" => make_native_fn(move |_args: &[Value]| {
                    arr_builtins::reverse(&Value::Array(a_clone.clone()))
                }),
                "fill" => make_native_fn(move |args: &[Value]| {
                    let value = args.first().unwrap_or(&Value::Null);
                    let start = args.get(1).unwrap_or(&Value::Null);
                    let end = args.get(2).unwrap_or(&Value::Null);
                    arr_builtins::fill(&Value::Array(a_clone.clone()), value, start, end)
                }),
                "shuffle" => make_native_fn(move |_args: &[Value]| {
                    arr_builtins::shuffle(&Value::Array(a_clone.clone()))
                }),
                "slice" => make_native_fn(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let end = args.get(1).unwrap_or(&Value::Null);
                    arr_builtins::slice(&Value::Array(a_clone.clone()), start, end)
                }),
                "concat" => make_native_fn(move |args: &[Value]| {
                    arr_builtins::concat(&Value::Array(a_clone.clone()), args)
                }),
                "join" => make_native_fn(move |args: &[Value]| {
                    let sep = args.first().unwrap_or(&Value::Null);
                    arr_builtins::join(&Value::Array(a_clone.clone()), sep)
                }),
                "indexOf" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    arr_builtins::index_of(&Value::Array(a_clone.clone()), search, args.get(1))
                }),
                "lastIndexOf" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    arr_builtins::last_index_of(&Value::Array(a_clone.clone()), search, args.get(1))
                }),
                "copyWithin" => make_native_fn(move |args: &[Value]| {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    let s = args.get(1).cloned().unwrap_or(Value::Null);
                    let e = args.get(2).cloned().unwrap_or(Value::Null);
                    arr_builtins::copy_within(&Value::Array(a_clone.clone()), &t, &s, &e)
                }),
                "includes" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let from = args.get(1);
                    arr_builtins::includes(&Value::Array(a_clone.clone()), search, from)
                }),
                "map" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    let arr = Value::Array(a_clone.clone());
                    // #187 native HOF fusion — tight native loop over a JIT'd numeric callback; bails
                    // (None) to the byte-identical generic path for any non-fusable callback/array.
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(v) = hof_fusion::map(&arr, &cb) {
                        return v;
                    }
                    arr_builtins::map(&arr, &cb)
                }),
                "filter" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    let arr = Value::Array(a_clone.clone());
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(v) = hof_fusion::filter(&arr, &cb) {
                        return v;
                    }
                    arr_builtins::filter(&arr, &cb)
                }),
                "reduce" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    let init = args.get(1).cloned(); // None = no initial value (empty array → throw)
                    let arr = Value::Array(a_clone.clone());
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(v) = hof_fusion::reduce(&arr, &cb, init.as_ref()) {
                        return v;
                    }
                    arr_builtins::reduce(&arr, &cb, init.as_ref())
                }),
                "forEach" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    let arr = Value::Array(a_clone.clone());
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(v) = hof_fusion::for_each(&arr, &cb) {
                        return v;
                    }
                    arr_builtins::for_each(&arr, &cb)
                }),
                "find" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find(&Value::Array(a_clone.clone()), &cb)
                }),
                "findIndex" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find_index(&Value::Array(a_clone.clone()), &cb)
                }),
                "findLast" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find_last(&Value::Array(a_clone.clone()), &cb)
                }),
                "findLastIndex" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find_last_index(&Value::Array(a_clone.clone()), &cb)
                }),
                "at" => make_native_fn(move |args: &[Value]| {
                    let i = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::at(&Value::Array(a_clone.clone()), &i)
                }),
                "some" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::some(&Value::Array(a_clone.clone()), &cb)
                }),
                "every" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::every(&Value::Array(a_clone.clone()), &cb)
                }),
                "flat" => make_native_fn(move |args: &[Value]| {
                    let depth = args.first().unwrap_or(&Value::Number(1.0));
                    arr_builtins::flat(&Value::Array(a_clone.clone()), depth)
                }),
                "flatMap" => make_native_fn(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::flat_map(&Value::Array(a_clone.clone()), &cb)
                }),
                "sort" => make_native_fn(move |args: &[Value]| {
                    let cmp = args.first();
                    if let Some(Value::Function(_)) = cmp {
                        arr_builtins::sort_with_comparator(
                            &Value::Array(a_clone.clone()),
                            cmp.unwrap(),
                        )
                    } else {
                        arr_builtins::sort_default(&Value::Array(a_clone.clone()))
                    }
                }),
                "splice" => make_native_fn(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let delete_count = args.get(1).map(|v| v as &Value);
                    let items: Vec<Value> = args.get(2..).unwrap_or(&[]).to_vec();
                    arr_builtins::splice(
                        &Value::Array(a_clone.clone()),
                        start,
                        delete_count,
                        &items,
                    )
                }),
                _ => return Err(format!("Property '{}' not found", key)),
            };
            Ok(Value::Function(method))
        }
        Value::String(s) => {
            let key_s = key.as_ref();
            if let Ok(idx) = key_s.parse::<usize>() {
                return match str_builtins::nth_char(s, idx) {
                    Some(c) => Ok(Value::String(tishlang_core::ArcStr::from(c.to_string()))),
                    None => Err("Index out of bounds".to_string()),
                };
            }
            if key_s == "length" {
                return Ok(Value::Number(str_builtins::char_count(s) as f64));
            }
            let s_clone: tishlang_core::ArcStr = s.clone();
            let method: ArrayMethodFn = match key_s {
                "indexOf" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let from = args.get(1);
                    str_builtins::index_of(&Value::String(s_clone.clone()), search, from)
                }),
                "lastIndexOf" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let position = args.get(1).cloned().unwrap_or(Value::Number(f64::INFINITY));
                    str_builtins::last_index_of(&Value::String(s_clone.clone()), search, &position)
                }),
                "includes" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let from = args.get(1);
                    str_builtins::includes(&Value::String(s_clone.clone()), search, from)
                }),
                "slice" => make_native_fn(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let end = args.get(1).unwrap_or(&Value::Null);
                    str_builtins::slice(&Value::String(s_clone.clone()), start, end)
                }),
                "substring" => make_native_fn(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let end = args.get(1).unwrap_or(&Value::Null);
                    str_builtins::substring(&Value::String(s_clone.clone()), start, end)
                }),
                "split" => make_native_fn(move |args: &[Value]| {
                    let sep = args.first().unwrap_or(&Value::Null);
                    let limit = args.get(1).unwrap_or(&Value::Null);
                    let max = match limit {
                        Value::Number(n) if *n >= 0.0 => Some(*n as usize),
                        _ => None,
                    };
                    // A RegExp separator needs the runtime's regex path — but a `Value::RegExp` can
                    // only exist with the `regex` feature, which is also what pulls in the optional
                    // `tishlang_runtime`. String separators use the always-available builtin, so
                    // `tish_vm` still compiles (and tests) without the optional runtime crate.
                    #[cfg(feature = "regex")]
                    if matches!(sep, Value::RegExp(_)) {
                        return tishlang_runtime::string_split_limit(
                            &Value::String(s_clone.clone()),
                            sep,
                            limit,
                        );
                    }
                    str_builtins::split_limit(&Value::String(s_clone.clone()), sep, max)
                }),
                "trim" => make_native_fn(move |_args: &[Value]| {
                    str_builtins::trim(&Value::String(s_clone.clone()))
                }),
                "trimStart" => make_native_fn(move |_args: &[Value]| {
                    str_builtins::trim_start(&Value::String(s_clone.clone()))
                }),
                "trimEnd" => make_native_fn(move |_args: &[Value]| {
                    str_builtins::trim_end(&Value::String(s_clone.clone()))
                }),
                "toUpperCase" => make_native_fn(move |_args: &[Value]| {
                    str_builtins::to_upper_case(&Value::String(s_clone.clone()))
                }),
                "toLowerCase" => make_native_fn(move |_args: &[Value]| {
                    str_builtins::to_lower_case(&Value::String(s_clone.clone()))
                }),
                "startsWith" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    str_builtins::starts_with(&Value::String(s_clone.clone()), search, args.get(1))
                }),
                "endsWith" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    str_builtins::ends_with(&Value::String(s_clone.clone()), search, args.get(1))
                }),
                "replace" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let replacement = args.get(1).unwrap_or(&Value::Null);
                    // RegExp search (incl. global flag + function replacer) routes to the runtime's
                    // regex-aware string_replace, identical to the rust backend.
                    #[cfg(feature = "regex")]
                    if matches!(search, Value::RegExp(_)) {
                        return tishlang_runtime::string_replace(
                            &Value::String(s_clone.clone()),
                            search,
                            replacement,
                        );
                    }
                    str_builtins::replace(&Value::String(s_clone.clone()), search, replacement)
                }),
                "replaceAll" => make_native_fn(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    let replacement = args.get(1).unwrap_or(&Value::Null);
                    str_builtins::replace_all(&Value::String(s_clone.clone()), search, replacement)
                }),
                #[cfg(feature = "regex")]
                "match" => make_native_fn(move |args: &[Value]| {
                    let re = args.first().unwrap_or(&Value::Null);
                    tishlang_runtime::string_match_regex(&Value::String(s_clone.clone()), re)
                }),
                #[cfg(feature = "regex")]
                "search" => make_native_fn(move |args: &[Value]| {
                    let re = args.first().unwrap_or(&Value::Null);
                    tishlang_runtime::string_search_regex(&Value::String(s_clone.clone()), re)
                }),
                "charAt" => make_native_fn(move |args: &[Value]| {
                    let idx = args.first().unwrap_or(&Value::Null);
                    str_builtins::char_at(&Value::String(s_clone.clone()), idx)
                }),
                "at" => make_native_fn(move |args: &[Value]| {
                    let idx = args.first().unwrap_or(&Value::Null);
                    str_builtins::at(&Value::String(s_clone.clone()), idx)
                }),
                "charCodeAt" => make_native_fn(move |args: &[Value]| {
                    let idx = args.first().unwrap_or(&Value::Null);
                    str_builtins::char_code_at(&Value::String(s_clone.clone()), idx)
                }),
                "repeat" => make_native_fn(move |args: &[Value]| {
                    let count = args.first().unwrap_or(&Value::Null);
                    str_builtins::repeat(&Value::String(s_clone.clone()), count)
                }),
                "padStart" => make_native_fn(move |args: &[Value]| {
                    let target_len = args.first().unwrap_or(&Value::Null);
                    let pad = args.get(1).unwrap_or(&Value::Null);
                    str_builtins::pad_start(&Value::String(s_clone.clone()), target_len, pad)
                }),
                "padEnd" => make_native_fn(move |args: &[Value]| {
                    let target_len = args.first().unwrap_or(&Value::Null);
                    let pad = args.get(1).unwrap_or(&Value::Null);
                    str_builtins::pad_end(&Value::String(s_clone.clone()), target_len, pad)
                }),
                _ => return Err(format!("Property '{}' not found", key)),
            };
            Ok(Value::Function(method))
        }
        Value::Number(n) => {
            // Number.prototype methods. Shared impls live in tishlang_builtins::number so
            // the VM, rust runtime, and interpreter stay byte-identical (full-backend-parity-plan.md).
            let n_val = *n;
            let method: ArrayMethodFn = match key.as_ref() {
                "toFixed" => make_native_fn(move |args: &[Value]| {
                    let digits = args.first().unwrap_or(&Value::Null);
                    num_builtins::to_fixed(&Value::Number(n_val), digits)
                }),
                "toString" => make_native_fn(move |args: &[Value]| {
                    let radix = args.first().unwrap_or(&Value::Null);
                    num_builtins::to_string(&Value::Number(n_val), radix)
                }),
                _ => return Err(format!("Property '{}' not found", key)),
            };
            Ok(Value::Function(method))
        }
        #[cfg(feature = "regex")]
        Value::RegExp(re) => match key.as_ref() {
            // `test`/`exec` route to the same runtime impls the rust backend uses, so the match
            // object shape (keys "0".."n" + "index") and lastIndex advancement are identical.
            "test" => {
                let rc = re.clone();
                Ok(Value::native(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    tishlang_runtime::regexp_test(&Value::RegExp(rc.clone()), input)
                }))
            }
            "exec" => {
                let rc = re.clone();
                Ok(Value::native(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    tishlang_runtime::regexp_exec(&Value::RegExp(rc.clone()), input)
                }))
            }
            // Properties mirror the interpreter (eval.rs get_prop RegExp arm) exactly.
            "source" => Ok(Value::String(re.borrow().source.clone().into())),
            "flags" => Ok(Value::String(re.borrow().flags_string().into())),
            "lastIndex" => Ok(Value::Number(re.borrow().last_index as f64)),
            "global" => Ok(Value::Bool(re.borrow().flags.global)),
            "ignoreCase" => Ok(Value::Bool(re.borrow().flags.ignore_case)),
            "multiline" => Ok(Value::Bool(re.borrow().flags.multiline)),
            "dotAll" => Ok(Value::Bool(re.borrow().flags.dot_all)),
            "unicode" => Ok(Value::Bool(re.borrow().flags.unicode)),
            "sticky" => Ok(Value::Bool(re.borrow().flags.sticky)),
            _ => Err(format!("Property '{}' not found", key)),
        },
        #[cfg(any(feature = "http", feature = "promise"))]
        Value::Promise(p) => match key.as_ref() {
            "then" => {
                let pc = Arc::clone(p);
                Ok(Value::native(move |args| {
                    tishlang_runtime::promise_instance_then(&pc, args)
                }))
            }
            "catch" => {
                let pc = Arc::clone(p);
                Ok(Value::native(move |args| {
                    tishlang_runtime::promise_instance_catch(&pc, args)
                }))
            }
            _ => Err(format!("Property '{}' not found", key)),
        },
        _ => Err(format!(
            "Cannot read property '{}' of {}",
            key,
            obj.type_name()
        )),
    }
}

fn set_member(obj: &Value, key: &Arc<str>, val: Value) -> Result<(), String> {
    match obj {
        Value::Object(m) => {
            m.borrow_mut().strings.insert(Arc::clone(key), val);
            Ok(())
        }
        Value::Array(a) => {
            if key.as_ref() == "length" {
                // `arr.length = k` truncates or grows (holes read back as Null), JS-style.
                let new_len = array_length_arg(&val)?;
                let mut arr = a.borrow_mut();
                arr.resize(new_len, Value::Null);
                return Ok(());
            }
            let idx: usize = key.as_ref().parse().unwrap_or(0);
            let mut arr = a.borrow_mut();
            if idx < arr.len() {
                arr[idx] = val;
            } else {
                arr.resize(idx + 1, Value::Null);
                arr[idx] = val;
            }
            Ok(())
        }
        Value::NumberArray(a) => {
            if key.as_ref() == "length" {
                let new_len = array_length_arg(&val)?;
                // NaN is the packed-array hole marker (read back as Null), matching get_index.
                a.borrow_mut().resize(new_len, f64::NAN);
                return Ok(());
            }
            Err(format!("Cannot set property of {}", obj.type_name()))
        }
        _ => Err(format!("Cannot set property of {}", obj.type_name())),
    }
}

/// JS `arr.length = v`: `v` is coerced to a number and must be a valid array length —
/// a non-negative integer below 2³². Anything else is a RangeError ("Invalid array length").
fn array_length_arg(val: &Value) -> Result<usize, String> {
    let n = val.as_number().unwrap_or(f64::NAN);
    if n.is_nan() || n < 0.0 || n.fract() != 0.0 || n > 4_294_967_295.0 {
        return Err("Invalid array length".to_string());
    }
    Ok(n as usize)
}

fn get_index(obj: &Value, idx: &Value) -> Result<Value, String> {
    match obj {
        Value::NumberArray(a) => {
            let i = match idx {
                Value::Number(n) => *n as usize,
                _ => {
                    return Err(format!(
                        "Array index must be number, got {}",
                        idx.type_name()
                    ))
                }
            };
            // NaN is used as the hole marker (sparse-array positions); reads return Null.
            Ok(a.borrow()
                .get(i)
                .map(|&n| {
                    if n.is_nan() {
                        Value::Null
                    } else {
                        Value::Number(n)
                    }
                })
                .unwrap_or(Value::Null))
        }
        Value::Array(a) => {
            let i = match idx {
                Value::Number(n) => *n as usize,
                _ => {
                    return Err(format!(
                        "Array index must be number, got {}",
                        idx.type_name()
                    ));
                }
            };
            Ok(a.borrow().get(i).cloned().unwrap_or(Value::Null))
        }
        Value::String(s) => {
            let i = match idx {
                Value::Number(n) => {
                    let n = *n;
                    if n < 0.0 || n.fract() != 0.0 {
                        // A negative / non-integer string index reads `null` (JS `undefined`), NOT a
                        // throw — interp and native already return null; the vm must agree. (#437)
                        return Ok(Value::Null);
                    }
                    n as usize
                }
                _ => {
                    return Err(format!(
                        "String index must be number, got {}",
                        idx.type_name()
                    ));
                }
            };
            // `nth_char` returns None past the end, so the cursor cache handles bounds — no separate
            // O(n) `chars().count()` pre-check (#203).
            match str_builtins::nth_char(s, i) {
                Some(c) => Ok(Value::String(tishlang_core::ArcStr::from(c.to_string()))),
                // Past the end → `null` (JS `undefined`), matching interp/native, not a throw. (#437)
                None => Ok(Value::Null),
            }
        }
        // A missing own property returns `null`, not a thrown error — matching dot reads
        // (#66) and JS object semantics. Keeps `obj[key]` and `obj.key` in lockstep (#113).
        Value::Object(_) => Ok(object_get(obj, idx).unwrap_or(Value::Null)),
        #[cfg(any(feature = "http", feature = "promise"))]
        Value::Promise(_) => {
            let key_arc: std::sync::Arc<str> = match idx {
                Value::String(s) => std::sync::Arc::from(s.as_str()),
                _ => {
                    return Err(format!(
                        "Promise bracket access requires a string key, got {}",
                        idx.type_name()
                    ));
                }
            };
            get_member(obj, &key_arc)
        }
        _ => Err(format!(
            "Cannot read property '{}' of {}",
            idx.to_display_string(),
            obj.type_name()
        )),
    }
}

/// `delete obj[key]` semantics (issue #40). Objects drop the string key; arrays clear the
/// element at a numeric index to a `null` hole (length is preserved, JS-style). Anything else
/// is a no-op. The operator always evaluates to `true` (handled by the caller).
fn delete_index(obj: &Value, key: &Value) {
    match obj {
        Value::Object(m) => {
            let key_s: Arc<str> = match key {
                Value::String(s) => Arc::from(s.as_str()),
                other => Arc::from(other.to_display_string().as_str()),
            };
            m.borrow_mut().strings.remove(key_s.as_ref());
        }
        Value::Array(a) => {
            if let Value::Number(n) = key {
                let n = *n;
                if n >= 0.0 && n.fract() == 0.0 {
                    let i = n as usize;
                    let mut arr = a.borrow_mut();
                    if i < arr.len() {
                        arr[i] = Value::Null;
                    }
                }
            }
        }
        _ => {}
    }
}

fn set_index(obj: &Value, idx: &Value, val: Value) -> Result<(), String> {
    match obj {
        Value::NumberArray(a) => {
            let i = match idx {
                Value::Number(n) => *n as usize,
                _ => {
                    return Err(format!(
                        "Array index must be number, got {}",
                        idx.type_name()
                    ))
                }
            };
            // In-bounds numeric assignment stays packed.
            // Out-of-bounds or non-numeric falls through to the Array path by returning
            // a sentinel error — the caller (SetIndex opcode) does NOT handle deopt.
            // Instead we only do in-bounds-or-next-element numeric assignments here;
            // anything that creates holes (i > len) or sets a non-number is unsupported.
            match val {
                Value::Number(n) => {
                    let mut arr = a.borrow_mut();
                    // Extend with NaN "holes" if needed (NaN = sparse hole; read back as Null).
                    while arr.len() <= i {
                        arr.push(f64::NAN);
                    }
                    arr[i] = n;
                }
                // Non-numeric set: the Vec<f64> can't represent this type. Extend with NaN holes
                // up to the index, then leave the slot as NaN (the value is lost). This is a
                // known limitation of NumberArray; the uncommon mixed-type path should not produce
                // a NumberArray in the first place. The caller will see the correct index reads for
                // numeric elements and Null for the NaN holes.
                _ => {
                    let mut arr = a.borrow_mut();
                    while arr.len() <= i {
                        arr.push(f64::NAN);
                    }
                    // arr[i] is already NaN (hole); we can't store the non-numeric value — acceptable
                    // for the experimental TISH_PACKED_ARRAYS path.
                }
            }
            Ok(())
        }
        Value::Array(a) => {
            let i = match idx {
                Value::Number(n) => *n as usize,
                _ => {
                    return Err(format!(
                        "Array index must be number, got {}",
                        idx.type_name()
                    ));
                }
            };
            let mut arr = a.borrow_mut();
            while arr.len() <= i {
                arr.push(Value::Null);
            }
            arr[i] = val;
            Ok(())
        }
        Value::Object(_) => object_set(obj, idx, val),
        _ => Err(format!("Cannot set property of {}", obj.type_name())),
    }
}

/// Run a chunk with every capability linked into this `tishlang_vm` build (tests, embedders).
pub fn run(chunk: &Chunk) -> Result<Value, String> {
    let mut vm = Vm::new();
    vm.run_with_options(chunk, false)
}

/// Run a chunk with options (e.g. REPL persistence for top-level declarations).
pub fn run_with_options(chunk: &Chunk, opts: VmRunOptions) -> Result<Value, String> {
    let mut vm = Vm::with_capabilities(opts.capabilities);
    vm.run_with_options(chunk, opts.repl_mode)
}

#[cfg(test)]
mod recursion_limit_tests_381 {
    use tishlang_core::{set_max_call_depth_for_test, Value, DEFAULT_MAX_CALL_DEPTH};

    fn run_src(src: &str) -> Result<Value, String> {
        let program = tishlang_parser::parse(src).expect("parse");
        let chunk = tishlang_bytecode::compile(&program).expect("compile");
        super::run(&chunk)
    }

    #[test]
    fn deep_recursion_is_catchable_not_abort() {
        // Without the guard this infinite recursion aborts the process (SIGABRT). With it, the throw
        // is a catchable RangeError: try/catch recovers and the program returns normally.
        set_max_call_depth_for_test(300);
        // Object-returning recursion is NOT JIT-eligible, so it takes the guarded VM call path.
        let src = "let ok = false\n\
                   fn rec(n) { return { v: rec(n + 1) } }\n\
                   try { rec(0) } catch (e) { ok = true }\n\
                   ok";
        let out = run_src(src);
        set_max_call_depth_for_test(DEFAULT_MAX_CALL_DEPTH);
        assert!(
            out.is_ok(),
            "deep recursion must be catchable, not abort/error: {out:?}"
        );
    }

    #[test]
    fn uncaught_deep_recursion_surfaces_error_not_abort() {
        // An UNCAUGHT infinite recursion must surface as a returned error, never a SIGABRT.
        set_max_call_depth_for_test(300);
        let out = run_src("fn rec(n) { return { v: rec(n + 1) } }\nrec(0)");
        set_max_call_depth_for_test(DEFAULT_MAX_CALL_DEPTH);
        assert!(
            out.is_err(),
            "uncaught deep recursion must return an error, got {out:?}"
        );
    }

    #[test]
    fn normal_recursion_is_unaffected() {
        set_max_call_depth_for_test(20_000);
        let out = run_src(
            "fn fib(n) { if (n < 2) { return n } return fib(n - 1) + fib(n - 2) }\nfib(15)",
        );
        set_max_call_depth_for_test(DEFAULT_MAX_CALL_DEPTH);
        assert!(
            out.is_ok(),
            "normal recursion must not be affected: {out:?}"
        );
    }

    // The core of #381 for the JIT tier: a pure-numeric self-recursive function JIT-compiles and
    // recurses on the native stack via SelfCall, BELOW the VM's depth counter (so `set_max_call_depth`
    // can't bound it). Without the guard this overflows the native stack — an uncatchable SIGSEGV/abort
    // that would kill the whole process. With it, the entry SP check turns the overflow into a
    // RangeError which, uncaught, must surface as a returned `Err`. Run on a bounded-stack thread so
    // the guard (or, on a regression, the overflow) is reached fast: a working guard lets the thread
    // `join` cleanly with the error flag; a broken one aborts the process (the loud regression signal).
    #[test]
    fn jit_deep_recursion_surfaces_error_not_abort() {
        // `Value` is `!Send` (holds `Rc`), so reduce to a Send bool inside the thread: true iff the
        // overflow surfaced as the catchable stack-overflow error rather than aborting.
        let handle = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                matches!(
                    run_src(
                        "fn dive(n) { if (n <= 0.0) { return 0.0 } return dive(n - 1.0) + 1.0 }\n\
                         dive(50000000.0)",
                    ),
                    Err(ref e) if e.contains("call stack")
                )
            })
            .expect("spawn");
        let surfaced = handle
            .join()
            .expect("thread must not abort — the guard must catch the overflow");
        assert!(
            surfaced,
            "uncaught JIT deep recursion must surface as a RangeError, not abort"
        );
    }
}
