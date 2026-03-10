//! Stack-based bytecode VM.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_bytecode::{Chunk, Constant, Opcode};
use tish_core::Value;

/// Initialize default globals (console, Math, JSON, etc.)
fn init_globals() -> HashMap<Arc<str>, Value> {
    use tish_core::Value::*;
    let mut g = HashMap::new();

    let mut console = HashMap::new();
    console.insert(
        "log".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            println!("{}", s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "info".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            println!("{}", s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "warn".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            eprintln!("{}", s.join(" "));
            Value::Null
        })),
    );
    console.insert(
        "error".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let s: Vec<std::string::String> = args.iter().map(|v| v.to_display_string()).collect();
            eprintln!("{}", s.join(" "));
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

    g.insert("parseInt".into(), Value::Function(Rc::new(|args: &[Value]| {
            let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
        let s = s.trim();
        let radix = args.get(1).and_then(|v| v.as_number()).unwrap_or(10.0) as i32;
        let n = if (2..=36).contains(&radix) {
            let prefix: std::string::String = s
                .chars()
                .take_while(|c| *c == '-' || *c == '+' || c.is_ascii_digit())
                .collect();
            i64::from_str_radix(&prefix, radix as u32)
                .ok()
                .map(|n| n as f64)
        } else {
            None
        };
        Value::Number(n.unwrap_or(f64::NAN))
    })));
    g.insert("parseFloat".into(), Value::Function(Rc::new(|args: &[Value]| {
            let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
        Value::Number(s.trim().parse().unwrap_or(f64::NAN))
    })));
    g.insert("Infinity".into(), Value::Number(f64::INFINITY));
    g.insert("NaN".into(), Value::Number(f64::NAN));
    g.insert(
        "typeof".into(),
        Value::Function(Rc::new(|args: &[Value]| {
            let v = args.first().unwrap_or(&Value::Null);
            Value::String(type_name(v).into())
        })),
    );

    g
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Bool(_) => "boolean",
        Value::Null => "object",
        Value::Array(_) => "object",
        Value::Object(_) => "object",
        Value::Function(_) => "function",
        #[cfg(feature = "regex")]
        Value::RegExp(_) => "object",
        Value::Promise(_) => "object",
        Value::Opaque(o) => o.type_name(),
    }
}

pub struct Vm {
    stack: Vec<Value>,
    scope: HashMap<Arc<str>, Value>,
    globals: Rc<RefCell<HashMap<Arc<str>, Value>>>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            scope: HashMap::new(),
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
        let mut local_scope = HashMap::new();
        for (i, name) in chunk.names.iter().take(args.len()).enumerate() {
            if let Some(v) = args.get(i) {
                local_scope.insert(Arc::clone(name), v.clone());
            }
        }

        loop {
            if ip >= code.len() {
                break;
            }
            let op = code[ip];
            ip += 1;
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
                            Value::Function(Rc::new(move |args: &[Value]| {
                                let mut vm = Vm {
                                    stack: Vec::new(),
                                    scope: HashMap::new(),
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
                        .get(name.as_ref())
                        .cloned()
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
                    if local_scope.contains_key(name.as_ref()) {
                        local_scope.insert(Arc::clone(name), v);
                    } else if self.scope.contains_key(name.as_ref()) {
                        self.scope.insert(Arc::clone(name), v);
                    } else {
                        self.globals
                            .borrow_mut()
                            .insert(Arc::clone(name), v);
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
                                type_name(&callee)
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
                    let op = code[ip];
                    ip += 1;
                    let r = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let l = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
                    let result = eval_binop(op, &l, &r)?;
                    self.stack.push(result);
                }
                Opcode::UnaryOp => {
                    let op = code[ip];
                    ip += 1;
                    let o = self
                        .stack
                        .pop()
                        .ok_or_else(|| "Stack underflow".to_string())?;
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
                    set_member(&obj, key, val)?;
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
                    set_index(&obj, &idx_val, val)?;
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

fn eval_binop(op: u8, l: &Value, r: &Value) -> Result<Value, String> {
    use tish_core::Value::*;
    let ln = l.as_number().unwrap_or(f64::NAN);
    let rn = r.as_number().unwrap_or(f64::NAN);
    match op {
        0 => {
            // Add: string concat if either is string, else numeric
            if matches!(l, Value::String(_)) || matches!(r, Value::String(_)) {
                Ok(String(format!("{}{}", l.to_display_string(), r.to_display_string()).into()))
            } else {
                Ok(Number(ln + rn))
            }
        }
        1 => Ok(Number(ln - rn)),  // Sub
        2 => Ok(Number(ln * rn)),
        3 => Ok(Number(if rn == 0.0 { f64::NAN } else { ln / rn })),
        4 => Ok(Number(if rn == 0.0 { f64::NAN } else { ln % rn })),
        5 => Ok(Number(ln.powf(rn))),
        6 => Ok(Bool(l.strict_eq(r))),  // Eq
        7 => Ok(Bool(!l.strict_eq(r))),
        8 => Ok(Bool(l.strict_eq(r))),
        9 => Ok(Bool(!l.strict_eq(r))),
        10 => Ok(Bool(ln < rn)),
        11 => Ok(Bool(ln <= rn)),
        12 => Ok(Bool(ln > rn)),
        13 => Ok(Bool(ln >= rn)),
        14 => Ok(Bool(l.is_truthy() && r.is_truthy())),
        15 => Ok(Bool(l.is_truthy() || r.is_truthy())),
        _ => Err(format!("Unknown binop: {}", op)),
    }
}

fn eval_unary(op: u8, o: &Value) -> Result<Value, String> {
    use tish_core::Value::*;
    match op {
        0 => Ok(Bool(!o.is_truthy())), // Not
        1 => Ok(Number(-o.as_number().unwrap_or(f64::NAN))), // Neg
        2 => Ok(Number(o.as_number().unwrap_or(f64::NAN))),  // Pos
        3 | 4 => Ok(Null), // BitNot, Void
        _ => Err(format!("Unknown unary op: {}", op)),
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
            let arr = a.borrow();
            let idx: usize = key.as_ref().parse().unwrap_or(0);
            arr.get(idx).cloned().ok_or_else(|| "Index out of bounds".to_string())
        }
        _ => Err(format!("Cannot read property of {}", type_name(obj))),
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
                Ok(())
            } else {
                Err("Index out of bounds".to_string())
            }
        }
        _ => Err(format!("Cannot set property of {}", type_name(obj))),
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
