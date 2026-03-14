//! Stack-based bytecode VM.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_ast::{BinOp, UnaryOp};
use tish_builtins::array as arr_builtins;
use tish_builtins::globals as globals_builtins;
use tish_builtins::math as math_builtins;
use tish_bytecode::{u8_to_binop, u8_to_unaryop, Chunk, Constant, Opcode, NO_REST_PARAM};
use tish_core::Value;

type ArrayMethodFn = Rc<dyn Fn(&[Value]) -> Value>;

/// Look up built-in module export for LoadNativeExport. Returns None if unknown or feature disabled.
fn get_builtin_export(spec: &str, export_name: &str) -> Option<Value> {
    #[cfg(feature = "fs")]
    if spec == "tish:fs" {
        return match export_name {
            "readFile" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::read_file(args)))),
            "writeFile" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::write_file(args)))),
            "fileExists" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::file_exists(args)))),
            "readDir" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::read_dir(args)))),
            "mkdir" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::mkdir(args)))),
            _ => None,
        };
    }
    #[cfg(feature = "http")]
    if spec == "tish:http" {
        return match export_name {
            "fetch" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::http_fetch(args)))),
            "fetchAll" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::http_fetch_all(args)))),
            "serve" => Some(Value::Function(Rc::new(|args: &[Value]| {
                let port = args.first().cloned().unwrap_or(Value::Null);
                let handler = args.get(1).cloned().unwrap_or(Value::Null);
                if let Value::Function(f) = handler {
                    tish_runtime::http_serve(&[port], move |req_args| f(req_args))
                } else {
                    Value::Null
                }
            }))),
            _ => None,
        };
    }
    #[cfg(feature = "process")]
    if spec == "tish:process" {
        return match export_name {
            "exit" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_exit(args)))),
            "cwd" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_cwd(args)))),
            "exec" => Some(Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_exec(args)))),
            "argv" => Some(Value::Array(Rc::new(RefCell::new(
                std::env::args().map(|s| Value::String(s.into())).collect(),
            )))),
            "env" => Some(Value::Object(Rc::new(RefCell::new(
                std::env::vars()
                    .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                    .collect(),
            )))),
            "process" => {
                let mut m = HashMap::new();
                m.insert("exit".into(), Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_exit(args))));
                m.insert("cwd".into(), Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_cwd(args))));
                m.insert("exec".into(), Value::Function(Rc::new(|args: &[Value]| tish_runtime::process_exec(args))));
                m.insert(
                    "argv".into(),
                    Value::Array(Rc::new(RefCell::new(
                        std::env::args().map(|s| Value::String(s.into())).collect(),
                    ))),
                );
                m.insert(
                    "env".into(),
                    Value::Object(Rc::new(RefCell::new(
                        std::env::vars()
                            .map(|(k, v)| (Arc::from(k.as_str()), Value::String(v.into())))
                            .collect(),
                    ))),
                );
                Some(Value::Object(Rc::new(RefCell::new(m))))
            }
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
fn init_globals() -> HashMap<Arc<str>, Value> {
    let mut g = HashMap::new();

    let mut console = HashMap::new();
    console.insert(
        "debug".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            vm_log(&s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "log".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            vm_log(&s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "info".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            vm_log(&s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "warn".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            vm_log_err(&s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "error".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            vm_log_err(&s.join(" "));
            Value::Null
        })),
    );
    g.insert("console".into(), Value::Object(Rc::new(RefCell::new(console))));

    let mut math = HashMap::new();
    math.insert(
        "abs".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.abs())
        })),
    );
    math.insert(
        "sqrt".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.sqrt())
        })),
    );
    math.insert(
        "floor".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.floor())
        })),
    );
    math.insert(
        "ceil".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.ceil())
        })),
    );
    math.insert(
        "round".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
            Value::Number(n.round())
        })),
    );
    math.insert(
        "random".into(),
        Value::Function(Rc::new(|_| Value::Number(rand::random::<f64>()))),
    );
    math.insert(
        "min".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let nums: Vec<f64> = args.iter().filter_map(|v| v.as_number()).collect();
            Value::Number(nums.into_iter().fold(f64::NAN, |a, b| a.min(b)))
        })),
    );
    math.insert(
        "max".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let nums: Vec<f64> = args.iter().filter_map(|v| v.as_number()).collect();
            Value::Number(nums.into_iter().fold(f64::NAN, |a, b| a.max(b)))
        })),
    );
    math.insert("pow".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::pow(args))));
    math.insert("sin".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::sin(args))));
    math.insert("cos".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::cos(args))));
    math.insert("tan".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::tan(args))));
    math.insert("log".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::log(args))));
    math.insert("exp".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::exp(args))));
    math.insert("sign".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::sign(args))));
    math.insert("trunc".into(), Value::Function(Rc::new(|args: &[Value]| math_builtins::trunc(args))));
    math.insert("PI".into(), Value::Number(std::f64::consts::PI));
    math.insert("E".into(), Value::Number(std::f64::consts::E));
    g.insert("Math".into(), Value::Object(Rc::new(RefCell::new(math))));

    let mut json = HashMap::new();
    json.insert(
        "parse".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
            tish_core::json_parse(&s).unwrap_or(Value::Null)
        })),
    );
    json.insert(
        "stringify".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let v = args.first().unwrap_or(&Value::Null);
            Value::String(tish_core::json_stringify(v).into())
        })),
    );
    g.insert("JSON".into(), Value::Object(Rc::new(RefCell::new(json))));

    g.insert("parseInt".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::parse_int(args))));
    g.insert("parseFloat".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::parse_float(args))));
    g.insert("encodeURI".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::encode_uri(args))));
    g.insert("decodeURI".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::decode_uri(args))));
    g.insert("Boolean".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::boolean(args))));
    g.insert("isFinite".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::is_finite(args))));
    g.insert("isNaN".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::is_nan(args))));
    g.insert("Infinity".into(), Value::Number(f64::INFINITY));
    g.insert("NaN".into(), Value::Number(f64::NAN));
    g.insert(
        "typeof".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let v = args.first().unwrap_or(&Value::Null);
            Value::String(v.type_name().into())
        })),
    );

    // Date - at minimum Date.now() for timing
    let mut date = HashMap::new();
    date.insert(
        "now".into(),
        Value::Function(Rc::new(|_args: &[Value]| {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as f64;
            Value::Number(ms)
        })),
    );
    g.insert("Date".into(), Value::Object(Rc::new(RefCell::new(date))));

    // Object methods - delegate to tish_builtins::globals
    let mut object_methods = HashMap::new();
    object_methods.insert(
        "assign".into(),
        Value::Function(Rc::new(|args: &[Value]| globals_builtins::object_assign(args))),
    );
    object_methods.insert(
        "fromEntries".into(),
        Value::Function(Rc::new(|args: &[Value]| globals_builtins::object_from_entries(args))),
    );
    object_methods.insert("keys".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::object_keys(args))));
    object_methods.insert("values".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::object_values(args))));
    object_methods.insert("entries".into(), Value::Function(Rc::new(|args: &[Value]| globals_builtins::object_entries(args))));
    g.insert("Object".into(), Value::Object(Rc::new(RefCell::new(object_methods))));

    // Array.isArray
    let mut array_static = HashMap::new();
    array_static.insert(
        "isArray".into(),
        Value::Function(Rc::new(|args: &[Value]| globals_builtins::array_is_array(args))),
    );
    g.insert("Array".into(), Value::Object(Rc::new(RefCell::new(array_static))));

    // String.fromCharCode
    let mut string_static = HashMap::new();
    string_static.insert(
        "fromCharCode".into(),
        Value::Function(Rc::new(|args: &[Value]| globals_builtins::string_from_char_code(args))),
    );
    g.insert("String".into(), Value::Object(Rc::new(RefCell::new(string_static))));

    g
}

/// Shared scope for closure capture (parent frame's locals).
type ScopeMap = Rc<RefCell<HashMap<Arc<str>, Value>>>;

pub struct Vm {
    stack: Vec<Value>,
    scope: HashMap<Arc<str>, Value>,
    /// Enclosing scope for closures (captured parent frame locals).
    enclosing: Option<ScopeMap>,
    globals: Rc<RefCell<HashMap<Arc<str>, Value>>>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            scope: HashMap::new(),
            enclosing: None,
            globals: Rc::new(RefCell::new(init_globals())),
        }
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.globals.borrow().get(name).cloned()
    }

    pub fn set_global(&mut self, name: Arc<str>, value: Value) {
        self.globals.borrow_mut().insert(name, value);
    }

    fn read_u16(code: &[u8], ip: &mut usize) -> u16 {
        let a = code[*ip] as u16;
        let b = code[*ip + 1] as u16;
        *ip += 2;
        (a << 8) | b
    }

    pub fn run(&mut self, chunk: &Chunk) -> Result<Value, String> {
        self.run_chunk(chunk, &chunk.nested, &[])
    }

    fn run_chunk(
        &mut self,
        chunk: &Chunk,
        nested: &[Chunk],
        args: &[Value],
    ) -> Result<Value, String> {
        let code = &chunk.code;
        let constants = &chunk.constants;
        let names = &chunk.names;

        let mut ip = 0;
        let local_scope: ScopeMap = Rc::new(RefCell::new(HashMap::new()));
        {
            let mut ls = local_scope.borrow_mut();
            let param_count = chunk.param_count as usize;
            if chunk.rest_param_index != NO_REST_PARAM {
                let ri = chunk.rest_param_index as usize;
                for (i, name) in chunk.names.iter().take(param_count).enumerate() {
                    if i < ri {
                        let v = args.get(i).cloned().unwrap_or(Value::Null);
                        ls.insert(Arc::clone(name), v);
                    } else if i == ri {
                        let rest_arr: Vec<Value> = args.iter().skip(ri).cloned().collect();
                        ls.insert(
                            Arc::clone(name),
                            Value::Array(Rc::new(RefCell::new(rest_arr))),
                        );
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

        loop {
            if ip >= code.len() {
                break;
            }
            let op = code[ip];
            ip += 1;
            if op == Opcode::Nop as u8 {
                continue;
            }
            let opcode = Opcode::from_u8(op).ok_or_else(|| format!("Unknown opcode: {}", op))?;

            match opcode {
                Opcode::Nop => {}
                Opcode::LoadConst => {
                    let idx = Self::read_u16(code, &mut ip);
                    let c = constants
                        .get(idx as usize)
                        .ok_or_else(|| format!("Constant index out of bounds: {}", idx))?;
                    let v = match c {
                        Constant::Number(n) => Value::Number(*n),
                        Constant::String(s) => Value::String(Arc::clone(s)),
                        Constant::Bool(b) => Value::Bool(*b),
                        Constant::Null => Value::Null,
                        Constant::Closure(nested_idx) => {
                            let inner = nested
                                .get(*nested_idx)
                                .ok_or_else(|| "Nested chunk index out of bounds".to_string())?;
                            let inner_clone = inner.clone();
                            let globals = Rc::clone(&self.globals);
                            let enclosing = Some(Rc::clone(&local_scope));
                            Value::Function(Rc::new(move |args: &[Value]| {
                                let mut vm = Vm {
                                    stack: Vec::new(),
                                    scope: HashMap::new(),
                                    enclosing: enclosing.clone(),
                                    globals: Rc::clone(&globals),
                                };
                                vm.run_chunk(&inner_clone, &inner_clone.nested, args)
                                    .unwrap_or(Value::Null)
                            }))
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
                        .borrow()
                        .get(name.as_ref())
                        .cloned()
                        .or_else(|| {
                            self.enclosing
                                .as_ref()
                                .and_then(|e| e.borrow().get(name.as_ref()).cloned())
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
                    if local_scope.borrow().contains_key(name.as_ref()) {
                        local_scope.borrow_mut().insert(Arc::clone(name), v);
                    } else if self
                        .enclosing
                        .as_ref()
                        .map(|e| e.borrow().contains_key(name.as_ref()))
                        .unwrap_or(false)
                    {
                        let en = self.enclosing.as_ref().unwrap();
                        en.borrow_mut().insert(Arc::clone(name), v);
                    } else if self.scope.contains_key(name.as_ref()) {
                        self.scope.insert(Arc::clone(name), v);
                    } else if self.globals.borrow().contains_key(name.as_ref()) {
                        self.globals
                            .borrow_mut()
                            .insert(Arc::clone(name), v);
                    } else {
                        // New variable: store in current frame so closures can capture via enclosing
                        local_scope.borrow_mut().insert(Arc::clone(name), v);
                    }
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
                    self.globals
                        .borrow_mut()
                        .insert(Arc::clone(name), v);
                }
                Opcode::Pop => {
                    self.stack.pop().ok_or_else(|| "Stack underflow".to_string())?;
                }
                Opcode::Dup => {
                    let v = self
                        .stack
                        .last()
                        .ok_or_else(|| "Stack underflow".to_string())?
                        .clone();
                    self.stack.push(v);
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
                    match &callee {
                        Value::Function(f) => {
                            let result = f(&args);
                            self.stack.push(result);
                        }
                        _ => {
                            return Err(format!(
                                "Call of non-function: {}",
                                callee.type_name()
                            ));
                        }
                    }
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
                    let args: Vec<Value> = match &args_array {
                        Value::Array(a) => a.borrow().clone(),
                        _ => {
                            return Err(format!(
                                "CallSpread: args must be array, got {}",
                                args_array.to_display_string()
                            ));
                        }
                    };
                    match &callee {
                        Value::Function(f) => {
                            let result = f(&args);
                            self.stack.push(result);
                        }
                        _ => {
                            return Err(format!(
                                "Call of non-function: {}",
                                callee.type_name()
                            ));
                        }
                    }
                }
                Opcode::Return => {
                    let v = self.stack.pop().unwrap_or(Value::Null);
                    return Ok(v);
                }
                Opcode::Jump => {
                    let offset = Self::read_u16(code, &mut ip) as usize;
                    ip = ip.saturating_add(offset as isize as usize);
                }
                Opcode::JumpIfFalse => {
                    let offset = Self::read_u16(code, &mut ip) as usize;
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    if !v.is_truthy() {
                        ip = ip.saturating_add(offset as isize as usize);
                    }
                }
                Opcode::JumpBack => {
                    let dist = Self::read_u16(code, &mut ip) as usize;
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
                    let op = u8_to_binop(op_u8)
                        .ok_or_else(|| format!("Unknown binop: {}", op_u8))?;
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
                    let v = get_member(&obj, key)?;
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
                    let v = get_member(&obj, key).unwrap_or(Value::Null);
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
                    set_member(&obj, key, val.clone())?;
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
                    let v = get_index(&obj, &idx_val)?;
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
                    set_index(&obj, &idx_val, val.clone())?;
                    self.stack.push(dup_val); // assignment yields the assigned value
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
                    self.stack
                        .push(Value::Array(Rc::new(RefCell::new(elems))));
                }
                Opcode::NewObject => {
                    let n = Self::read_u16(code, &mut ip) as usize;
                    let mut map = HashMap::new();
                    for _ in 0..n {
                        let val = self
                            .stack
                            .pop()
                            .ok_or_else(|| "Stack underflow".to_string())?;
                        let key_val = self
                            .stack
                            .pop()
                            .ok_or_else(|| "Stack underflow".to_string())?;
                        let key = key_val.to_display_string().into();
                        map.insert(key, val);
                    }
                    self.stack
                        .push(Value::Object(Rc::new(RefCell::new(map))));
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
                    self.stack.push(Value::Array(Rc::new(RefCell::new(a))));
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
                    let mut merged: HashMap<Arc<str>, Value> = HashMap::new();
                    if let Value::Object(l) = &left {
                        merged.extend(l.borrow().iter().map(|(k, v)| (Arc::clone(k), v.clone())));
                    } else {
                        return Err(format!(
                            "MergeObject: left must be object, got {}",
                            left.to_display_string()
                        ));
                    }
                    if let Value::Object(r) = &right {
                        for (k, v) in r.borrow().iter() {
                            merged.insert(Arc::clone(k), v.clone());
                        }
                    } else {
                        return Err(format!(
                            "MergeObject: right must be object, got {}",
                            right.to_display_string()
                        ));
                    }
                    self.stack
                        .push(Value::Object(Rc::new(RefCell::new(merged))));
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
                        Value::Array(a) => {
                            Value::Array(Rc::new(RefCell::new(a.borrow().clone())))
                        }
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
                    let result = if let Value::Array(a) = &arr {
                        let arr_borrow = a.borrow();
                        let mapped: Vec<Value> = arr_borrow
                            .iter()
                            .map(|v| {
                                let l: Value = if param_left { (*v).clone() } else { const_val.clone() };
                                let r: Value = if param_left { const_val.clone() } else { (*v).clone() };
                                eval_binop(binop, &l, &r).unwrap_or(Value::Null)
                            })
                            .collect();
                        Value::Array(Rc::new(RefCell::new(mapped)))
                    } else {
                        Value::Null
                    };
                    self.stack.push(result);
                }
                Opcode::ArrayFilterBinOp => {
                    let binop_u8 = code[ip];
                    ip += 1;
                    let const_idx = Self::read_u16(code, &mut ip);
                    let param_left = code[ip] == 0; // 0 = param on left (x op const), 1 = param on right (const op x)
                    ip += 1;
                    let binop = u8_to_binop(binop_u8)
                        .ok_or_else(|| format!("Unknown binop in ArrayFilterBinOp: {}", binop_u8))?;
                    let arr = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let const_val = constants
                        .get(const_idx as usize)
                        .map(|c| c.to_value())
                        .unwrap_or(Value::Null);
                    let result = if let Value::Array(a) = &arr {
                        let arr_borrow = a.borrow();
                        let filtered: Vec<Value> = arr_borrow
                            .iter()
                            .filter(|v| {
                                let l: Value = if param_left { (*v).clone() } else { const_val.clone() };
                                let r: Value = if param_left { const_val.clone() } else { (*v).clone() };
                                let b = eval_binop(binop, &l, &r).unwrap_or(Value::Null);
                                b.is_truthy()
                            })
                            .cloned()
                            .collect();
                        Value::Array(Rc::new(RefCell::new(filtered)))
                    } else {
                        Value::Null
                    };
                    self.stack.push(result);
                }
                Opcode::Throw => {
                    let v = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let (catch_ip, stack_len) = try_handlers
                        .pop()
                        .ok_or_else(|| format!("Uncaught throw: {}", v.to_display_string()))?;
                    self.stack.truncate(stack_len);
                    self.stack.push(v);
                    ip = catch_ip;
                }
                Opcode::LoadNativeExport => {
                    let spec_idx = Self::read_u16(code, &mut ip);
                    let export_idx = Self::read_u16(code, &mut ip);
                    let spec = match constants.get(spec_idx as usize) {
                        Some(Constant::String(s)) => s.as_ref(),
                        _ => {
                            return Err("LoadNativeExport: spec constant out of bounds or not string".to_string());
                        }
                    };
                    let export_name = match constants.get(export_idx as usize) {
                        Some(Constant::String(s)) => s.as_ref(),
                        _ => {
                            return Err("LoadNativeExport: export_name constant out of bounds or not string".to_string());
                        }
                    };
                    let v = get_builtin_export(spec, export_name).ok_or_else(|| {
                        format!(
                            "Built-in module '{}' does not export '{}' or feature not enabled. Rebuild with --features fs, http, or process.",
                            spec, export_name
                        )
                    })?;
                    self.stack.push(v);
                }
                Opcode::Closure | Opcode::PopN | Opcode::LoadThis => {
                    return Err(format!("Unhandled opcode: {:?}", opcode));
                }
            }
        }

        Ok(self.stack.pop().unwrap_or(Value::Null))
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

fn eval_binop(op: BinOp, l: &Value, r: &Value) -> Result<Value, String> {
    use tish_ast::BinOp::*;
    use tish_core::Value::*;
    let ln = l.as_number().unwrap_or(f64::NAN);
    let rn = r.as_number().unwrap_or(f64::NAN);
    match op {
        Add => {
            if matches!(l, Value::String(_)) || matches!(r, Value::String(_)) {
                Ok(String(format!("{}{}", l.to_display_string(), r.to_display_string()).into()))
            } else {
                Ok(Number(ln + rn))
            }
        }
        Sub => Ok(Number(ln - rn)),
        Mul => Ok(Number(ln * rn)),
        Div => Ok(Number(if rn == 0.0 { f64::NAN } else { ln / rn })),
        Mod => Ok(Number(if rn == 0.0 { f64::NAN } else { ln % rn })),
        Pow => Ok(Number(ln.powf(rn))),
        Eq => Ok(Bool(l.strict_eq(r))),
        Ne => Ok(Bool(!l.strict_eq(r))),
        StrictEq => Ok(Bool(l.strict_eq(r))),
        StrictNe => Ok(Bool(!l.strict_eq(r))),
        Lt => Ok(Bool(ln < rn)),
        Le => Ok(Bool(ln <= rn)),
        Gt => Ok(Bool(ln > rn)),
        Ge => Ok(Bool(ln >= rn)),
        And => Ok(Bool(l.is_truthy() && r.is_truthy())),
        Or => Ok(Bool(l.is_truthy() || r.is_truthy())),
        BitAnd => Ok(Number((ln as i32 & rn as i32) as f64)),
        BitOr => Ok(Number((ln as i32 | rn as i32) as f64)),
        BitXor => Ok(Number((ln as i32 ^ rn as i32) as f64)),
        Shl => Ok(Number(((ln as i32) << (rn as i32)) as f64)),
        Shr => Ok(Number(((ln as i32) >> (rn as i32)) as f64)),
        In => {
            let key_s: Arc<str> = l.to_display_string().into();
            Ok(Bool(match r {
                Value::Object(m) => m.borrow().contains_key(&key_s),
                Value::Array(a) => {
                    if key_s.as_ref() == "length" {
                        true
                    } else if let Ok(idx) = key_s.parse::<usize>() {
                        idx < a.borrow().len()
                    } else {
                        false
                    }
                }
                _ => false,
            }))
        }
    }
}

fn eval_unary(op: UnaryOp, o: &Value) -> Result<Value, String> {
    use tish_ast::UnaryOp::*;
    use tish_core::Value::*;
    match op {
        Not => Ok(Bool(!o.is_truthy())),
        Neg => Ok(Number(-o.as_number().unwrap_or(f64::NAN))),
        Pos => Ok(Number(o.as_number().unwrap_or(f64::NAN))),
        BitNot => Ok(Number(!(o.as_number().unwrap_or(0.0) as i32) as f64)),
        Void => Ok(Null),
    }
}

fn get_member(obj: &Value, key: &Arc<str>) -> Result<Value, String> {
    match obj {
        Value::Object(m) => {
            let map = m.borrow();
            map.get(key.as_ref()).cloned().ok_or_else(|| {
                format!("Property '{}' not found", key)
            })
        }
        Value::Array(a) => {
            let key_s = key.as_ref();
            if let Ok(idx) = key_s.parse::<usize>() {
                let arr = a.borrow();
                return arr.get(idx).cloned().ok_or_else(|| "Index out of bounds".to_string());
            }
            if key_s == "length" {
                return Ok(Value::Number(a.borrow().len() as f64));
            }
            let a_clone = Rc::clone(a);
            let method: ArrayMethodFn = match key_s {
                "push" => Rc::new(move |args: &[Value]| arr_builtins::push(&Value::Array(Rc::clone(&a_clone)), args)),
                "pop" => Rc::new(move |_args: &[Value]| arr_builtins::pop(&Value::Array(Rc::clone(&a_clone)))),
                "shift" => Rc::new(move |_args: &[Value]| arr_builtins::shift(&Value::Array(Rc::clone(&a_clone)))),
                "unshift" => Rc::new(move |args: &[Value]| arr_builtins::unshift(&Value::Array(Rc::clone(&a_clone)), args)),
                "reverse" => Rc::new(move |_args: &[Value]| arr_builtins::reverse(&Value::Array(Rc::clone(&a_clone)))),
                "shuffle" => Rc::new(move |_args: &[Value]| arr_builtins::shuffle(&Value::Array(Rc::clone(&a_clone)))),
                "slice" => Rc::new(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let end = args.get(1).unwrap_or(&Value::Null);
                    arr_builtins::slice(&Value::Array(Rc::clone(&a_clone)), start, end)
                }),
                "concat" => Rc::new(move |args: &[Value]| arr_builtins::concat(&Value::Array(Rc::clone(&a_clone)), args)),
                "join" => Rc::new(move |args: &[Value]| {
                    let sep = args.first().unwrap_or(&Value::Null);
                    arr_builtins::join(&Value::Array(Rc::clone(&a_clone)), sep)
                }),
                "indexOf" => Rc::new(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    arr_builtins::index_of(&Value::Array(Rc::clone(&a_clone)), search)
                }),
                "includes" => Rc::new(move |args: &[Value]| {
                    let search = args.first().unwrap_or(&Value::Null);
                    arr_builtins::includes(&Value::Array(Rc::clone(&a_clone)), search)
                }),
                "map" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::map(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "filter" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::filter(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "reduce" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    let init = args.get(1).cloned().unwrap_or(Value::Null);
                    arr_builtins::reduce(&Value::Array(Rc::clone(&a_clone)), &cb, &init)
                }),
                "forEach" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::for_each(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "find" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "findIndex" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::find_index(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "some" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::some(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "every" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::every(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "flat" => Rc::new(move |args: &[Value]| {
                    let depth = args.first().unwrap_or(&Value::Number(1.0));
                    arr_builtins::flat(&Value::Array(Rc::clone(&a_clone)), depth)
                }),
                "flatMap" => Rc::new(move |args: &[Value]| {
                    let cb = args.first().cloned().unwrap_or(Value::Null);
                    arr_builtins::flat_map(&Value::Array(Rc::clone(&a_clone)), &cb)
                }),
                "sort" => Rc::new(move |args: &[Value]| {
                    let cmp = args.first();
                    if let Some(Value::Function(_)) = cmp {
                        arr_builtins::sort_with_comparator(&Value::Array(Rc::clone(&a_clone)), cmp.unwrap())
                    } else {
                        arr_builtins::sort_default(&Value::Array(Rc::clone(&a_clone)))
                    }
                }),
                "splice" => Rc::new(move |args: &[Value]| {
                    let start = args.first().unwrap_or(&Value::Null);
                    let delete_count = args.get(1).map(|v| v as &Value);
                    let items: Vec<Value> = args.get(2..).unwrap_or(&[]).to_vec();
                    arr_builtins::splice(&Value::Array(Rc::clone(&a_clone)), start, delete_count, &items)
                }),
                _ => return Err(format!("Property '{}' not found", key)),
            };
            Ok(Value::Function(method))
        }
        _ => Err(format!("Cannot read property '{}' of {}", key, obj.type_name())),
    }
}

fn set_member(obj: &Value, key: &Arc<str>, val: Value) -> Result<(), String> {
    match obj {
        Value::Object(m) => {
            m.borrow_mut().insert(Arc::clone(key), val);
            Ok(())
        }
        Value::Array(a) => {
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
        _ => Err(format!("Cannot set property of {}", obj.type_name())),
    }
}

fn get_index(obj: &Value, idx: &Value) -> Result<Value, String> {
    let key: Arc<str> = idx.to_display_string().into();
    get_member(obj, &key)
}

fn set_index(obj: &Value, idx: &Value, val: Value) -> Result<(), String> {
    let key: Arc<str> = idx.to_display_string().into();
    set_member(obj, &key, val)
}

/// Run a chunk and return the result.
pub fn run(chunk: &Chunk) -> Result<Value, String> {
    let mut vm = Vm::new();
    vm.run(chunk)
}
