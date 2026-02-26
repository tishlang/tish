//! Tree-walk evaluator for Tish.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_ast::{BinOp, Expr, Literal, MemberProp, Statement, UnaryOp};

use crate::value::Value;

struct Scope {
    vars: HashMap<Arc<str>, Value>,
    parent: Option<Rc<std::cell::RefCell<Scope>>>,
}

impl Scope {
    fn new() -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: HashMap::new(),
            parent: None,
        }))
    }

    fn child(parent: Rc<std::cell::RefCell<Scope>>) -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: HashMap::new(),
            parent: Some(parent),
        }))
    }

    fn get(&self, name: &str) -> Option<Value> {
        self.vars.get(name).cloned().or_else(|| {
            self.parent
                .as_ref()
                .and_then(|p| p.borrow().get(name))
        })
    }

    fn set(&mut self, name: Arc<str>, value: Value) {
        self.vars.insert(name, value);
    }

    fn assign(&mut self, name: &str, value: Value) -> bool {
        if self.vars.contains_key(name) {
            self.vars.insert(name.into(), value);
            return true;
        }
        self.parent
            .as_ref()
            .map(|p| p.borrow_mut().assign(name, value))
            .unwrap_or(false)
    }
}

pub struct Evaluator {
    scope: Rc<std::cell::RefCell<Scope>>,
}

impl Evaluator {
    pub fn new() -> Self {
        let scope = Scope::new();
        {
            let mut s = scope.borrow_mut();
            let mut console = HashMap::new();
            console.insert("debug".into(), Value::NativeConsoleDebug);
            console.insert("info".into(), Value::NativeConsoleInfo);
            console.insert("log".into(), Value::NativeConsoleLog);
            console.insert("warn".into(), Value::NativeConsoleWarn);
            console.insert("error".into(), Value::NativeConsoleError);
            s.set("console".into(), Value::Object(Rc::new(console)));
            s.set("parseInt".into(), Value::NativeParseInt);
            s.set("parseFloat".into(), Value::NativeParseFloat);
            s.set("decodeURI".into(), Value::NativeDecodeURI);
            s.set("encodeURI".into(), Value::NativeEncodeURI);
            s.set("isFinite".into(), Value::NativeIsFinite);
            s.set("isNaN".into(), Value::NativeIsNaN);
            s.set("Infinity".into(), Value::Number(f64::INFINITY));
            s.set("NaN".into(), Value::Number(f64::NAN));
            let mut math = HashMap::new();
            math.insert("abs".into(), Value::NativeMathAbs);
            math.insert("sqrt".into(), Value::NativeMathSqrt);
            math.insert("min".into(), Value::NativeMathMin);
            math.insert("max".into(), Value::NativeMathMax);
            math.insert("floor".into(), Value::NativeMathFloor);
            math.insert("ceil".into(), Value::NativeMathCeil);
            math.insert("round".into(), Value::NativeMathRound);
            s.set("Math".into(), Value::Object(Rc::new(math)));
            let mut json = HashMap::new();
            json.insert("parse".into(), Value::NativeJsonParse);
            json.insert("stringify".into(), Value::NativeJsonStringify);
            s.set("JSON".into(), Value::Object(Rc::new(json)));
        }
        Self { scope }
    }

    pub fn eval_program(&mut self, program: &tish_ast::Program) -> Result<Value, String> {
        let mut last = Value::Null;
        for stmt in &program.statements {
            last = self.eval_statement(stmt).map_err(|e| e.to_string())?;
        }
        Ok(last)
    }

    fn eval_statement(&mut self, stmt: &Statement) -> Result<Value, EvalError> {
        match stmt {
            Statement::Block { statements, .. } => {
                let scope = Scope::child(Rc::clone(&self.scope));
                let prev = std::mem::replace(&mut self.scope, scope);
                let mut last = Value::Null;
                for s in statements {
                    last = self.eval_statement(s)?;
                }
                self.scope = prev;
                Ok(last)
            }
            Statement::VarDecl { name, init, .. } => {
                let value = init
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .transpose()?
                    .unwrap_or(Value::Null);
                self.scope.borrow_mut().set(Arc::clone(name), value);
                Ok(Value::Null)
            }
            Statement::ExprStmt { expr, .. } => self.eval_expr(expr),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.eval_expr(cond)?;
                if c.is_truthy() {
                    self.eval_statement(then_branch)
                } else if let Some(eb) = else_branch {
                    self.eval_statement(eb)
                } else {
                    Ok(Value::Null)
                }
            }
            Statement::While { cond, body, .. } => {
                loop {
                    if !self.eval_expr(cond)?.is_truthy() {
                        break;
                    }
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }
            Statement::ForOf { name, iterable, body, .. } => {
                let iter_val = self.eval_expr(iterable)?;
                let elements = match &iter_val {
                    crate::value::Value::Array(arr) => arr.iter().cloned().collect::<Vec<_>>(),
                    crate::value::Value::String(s) => {
                        s.chars()
                            .map(|c| crate::value::Value::String(Arc::from(c.to_string())))
                            .collect::<Vec<_>>()
                    }
                    _ => {
                        return Err(EvalError::Error(format!(
                            "for-of requires iterable (array or string), got {}",
                            iter_val.to_string()
                        )));
                    }
                };
                for elem in elements {
                    self.scope.borrow_mut().set(Arc::clone(name), elem);
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(i) = init {
                    self.eval_statement(i)?;
                }
                loop {
                    let cond_ok = cond
                        .as_ref()
                        .map(|c| self.eval_expr(c).map(|v| v.is_truthy()))
                        .transpose()?
                        .unwrap_or(true);
                    if !cond_ok {
                        break;
                    }
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => {
                            if let Some(u) = update {
                                self.eval_expr(u)?;
                            }
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    if let Some(u) = update {
                        self.eval_expr(u)?;
                    }
                }
                Ok(Value::Null)
            }
            Statement::Return { value, .. } => {
                let v = value
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .transpose()?
                    .unwrap_or(Value::Null);
                Err(EvalError::Return(v))
            }
            Statement::Break { .. } => Err(EvalError::Break),
            Statement::Continue { .. } => Err(EvalError::Continue),
            Statement::FunDecl {
                name,
                params,
                rest_param,
                body,
                ..
            } => {
                let params = params.clone();
                let rest_param = rest_param.clone();
                let body = Box::clone(body);
                let _scope = Rc::clone(&self.scope);
                let func = Value::Function {
                    params,
                    rest_param,
                    body,
                };
                self.scope.borrow_mut().set(Arc::clone(name), func);
                Ok(Value::Null)
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                let v = self.eval_expr(expr)?;
                let mut matched = false;
                for (case_expr, body) in cases {
                    if let Some(ce) = case_expr {
                        let cv = self.eval_expr(ce)?;
                        if v.strict_eq(&cv) {
                            matched = true;
                            let scope = Scope::child(Rc::clone(&self.scope));
                            let prev = std::mem::replace(&mut self.scope, scope);
                            for s in body {
                                match self.eval_statement(s) {
                                    Ok(_) => {}
                                    Err(EvalError::Break) => {
                                        self.scope = prev;
                                        return Ok(Value::Null);
                                    }
                                    Err(e) => {
                                        self.scope = prev;
                                        return Err(e);
                                    }
                                }
                            }
                            self.scope = prev;
                            break;
                        }
                    }
                }
                if !matched {
                    if let Some(body) = default_body {
                        let scope = Scope::child(Rc::clone(&self.scope));
                        let prev = std::mem::replace(&mut self.scope, scope);
                        for s in body {
                            match self.eval_statement(s) {
                                Ok(_) => {}
                                Err(EvalError::Break) => break,
                                Err(e) => {
                                    self.scope = prev;
                                    return Err(e);
                                }
                            }
                        }
                        self.scope = prev;
                    }
                }
                Ok(Value::Null)
            }
            Statement::DoWhile { body, cond, .. } => {
                loop {
                    match self.eval_statement(body) {
                        Ok(_) => {}
                        Err(EvalError::Break) => break,
                        Err(EvalError::Continue) => {
                            if !self.eval_expr(cond)?.is_truthy() {
                                break;
                            }
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    if !self.eval_expr(cond)?.is_truthy() {
                        break;
                    }
                }
                Ok(Value::Null)
            }
            Statement::Throw { value, .. } => {
                let v = self.eval_expr(value)?;
                Err(EvalError::Throw(v))
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                ..
            } => {
                match self.eval_statement(body) {
                    Ok(v) => Ok(v),
                    Err(EvalError::Throw(thrown)) => {
                        if let Some(param) = catch_param {
                            let scope = Scope::child(Rc::clone(&self.scope));
                            let prev = std::mem::replace(&mut self.scope, Rc::clone(&scope));
                            scope.borrow_mut().set(Arc::clone(param), thrown);
                            let res = self.eval_statement(catch_body);
                            self.scope = prev;
                            res
                        } else {
                            self.eval_statement(catch_body)
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    fn eval_expr(&self, expr: &Expr) -> Result<Value, EvalError> {
        match expr {
            Expr::Literal { value, .. } => Ok(match value {
                Literal::Number(n) => Value::Number(*n),
                Literal::String(s) => Value::String(Arc::clone(s)),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::Null => Value::Null,
            }),
            Expr::Ident { name, .. } => self
                .scope
                .borrow()
                .get(name.as_ref())
                .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name))),
            Expr::Binary {
                left,
                op,
                right,
                ..
            } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binop(&l, *op, &r).map_err(EvalError::Error)
            }
            Expr::Unary { op, operand, .. } => {
                let o = self.eval_expr(operand)?;
                self.eval_unary(*op, &o).map_err(EvalError::Error)
            }
            Expr::Call { callee, args, .. } => {
                let f = self.eval_expr(callee)?;
                let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                let arg_vals = arg_vals?;
                self.call_func(&f, &arg_vals)
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                let obj = self.eval_expr(object)?;
                if *optional && matches!(obj, Value::Null) {
                    return Ok(Value::Null);
                }
                let key = match prop {
                    MemberProp::Name(n) => Arc::clone(n),
                    MemberProp::Expr(e) => {
                        let v = self.eval_expr(e)?;
                        match v {
                            Value::String(s) => s,
                            _ => return Err(EvalError::Error("Property key must be string".to_string())),
                        }
                    }
                };
                match self.get_prop(&obj, &key) {
                    Ok(v) => Ok(v),
                    Err(_) if *optional => Ok(Value::Null),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                let obj = self.eval_expr(object)?;
                if *optional && matches!(obj, Value::Null) {
                    return Ok(Value::Null);
                }
                let idx = self.eval_expr(index)?;
                self.get_index(&obj, &idx).map_err(EvalError::Error)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                if self.eval_expr(cond)?.is_truthy() {
                    self.eval_expr(then_branch)
                } else {
                    self.eval_expr(else_branch)
                }
            }
            Expr::NullishCoalesce { left, right, .. } => {
                let l = self.eval_expr(left)?;
                if matches!(l, Value::Null) {
                    self.eval_expr(right)
                } else {
                    Ok(l)
                }
            }
            Expr::Array { elements, .. } => {
                let vals: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.eval_expr(e)).collect();
                Ok(Value::Array(Rc::new(vals?)))
            }
            Expr::Object { props, .. } => {
                let mut map = HashMap::new();
                for (k, v) in props {
                    map.insert(Arc::clone(k), self.eval_expr(v)?);
                }
                Ok(Value::Object(Rc::new(map)))
            }
            Expr::Assign { name, value, .. } => {
                let v = self.eval_expr(value)?;
                if !self.scope.borrow_mut().assign(name.as_ref(), v.clone()) {
                    return Err(EvalError::Error(format!("Undefined variable: {}", name)));
                }
                Ok(v)
            }
            Expr::TypeOf { operand, .. } => {
                let v = self.eval_expr(operand)?;
                Ok(Value::String(match &v {
                    Value::Number(_) => "number".into(),
                    Value::String(_) => "string".into(),
                    Value::Bool(_) => "boolean".into(),
                    Value::Null => "object".into(),
                    Value::Array(_) => "object".into(),
                    Value::Object(_) => "object".into(),
                    Value::Function { .. } => "function".into(),
                    Value::NativeConsoleDebug
                    | Value::NativeConsoleInfo
                    | Value::NativeConsoleLog
                    | Value::NativeConsoleWarn
                    | Value::NativeConsoleError
                    | Value::NativeParseInt
                    | Value::NativeParseFloat
                    | Value::NativeIsFinite
                    | Value::NativeIsNaN
                    | Value::NativeMathAbs
                    | Value::NativeMathSqrt
                    | Value::NativeMathMin
                    | Value::NativeMathMax
                    | Value::NativeMathFloor
                    | Value::NativeMathCeil
                    | Value::NativeMathRound
                    | Value::NativeJsonParse
                    | Value::NativeJsonStringify
                    | Value::NativeDecodeURI
                    | Value::NativeEncodeURI => "function".into(),
                }))
            }
            Expr::PostfixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                if !self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n + 1.0)) {
                    return Err(EvalError::Error(format!("Undefined variable: {}", name)));
                }
                Ok(Value::Number(n))
            }
            Expr::PostfixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                if !self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n - 1.0)) {
                    return Err(EvalError::Error(format!("Undefined variable: {}", name)));
                }
                Ok(Value::Number(n))
            }
            Expr::PrefixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                let new_val = Value::Number(n + 1.0);
                if !self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    return Err(EvalError::Error(format!("Undefined variable: {}", name)));
                }
                Ok(new_val)
            }
            Expr::PrefixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                let new_val = Value::Number(n - 1.0);
                if !self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    return Err(EvalError::Error(format!("Undefined variable: {}", name)));
                }
                Ok(new_val)
            }
        }
    }

    fn eval_binop(&self, l: &Value, op: BinOp, r: &Value) -> Result<Value, String> {
        match op {
            BinOp::Add => match (l, r) {
                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                (Value::String(a), Value::String(b)) => {
                    Ok(Value::String(format!("{}{}", a, b).into()))
                }
                (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b.to_string()).into())),
                (a, Value::String(b)) => Ok(Value::String(format!("{}{}", a.to_string(), b).into())),
                _ => Err(format!("Cannot add {:?} and {:?}", l, r)),
            },
            BinOp::Sub => self.binop_number(l, r, |a, b| Value::Number(a - b)),
            BinOp::Mul => self.binop_number(l, r, |a, b| Value::Number(a * b)),
            BinOp::Div => self.binop_number(l, r, |a, b| Value::Number(a / b)),
            BinOp::Mod => self.binop_number(l, r, |a, b| Value::Number(a % b)),
            BinOp::Pow => self.binop_number(l, r, |a, b| Value::Number(a.powf(b))),
            BinOp::StrictEq => Ok(Value::Bool(l.strict_eq(r))),
            BinOp::StrictNe => Ok(Value::Bool(!l.strict_eq(r))),
            BinOp::Lt => self.binop_number(l, r, |a, b| Value::Bool(a < b)),
            BinOp::Le => self.binop_number(l, r, |a, b| Value::Bool(a <= b)),
            BinOp::Gt => self.binop_number(l, r, |a, b| Value::Bool(a > b)),
            BinOp::Ge => self.binop_number(l, r, |a, b| Value::Bool(a >= b)),
            BinOp::And => Ok(Value::Bool(l.is_truthy() && r.is_truthy())),
            BinOp::Or => Ok(Value::Bool(l.is_truthy() || r.is_truthy())),
            BinOp::BitAnd => self.binop_int32(l, r, |a, b| Value::Number((a & b) as f64)),
            BinOp::BitOr => self.binop_int32(l, r, |a, b| Value::Number((a | b) as f64)),
            BinOp::BitXor => self.binop_int32(l, r, |a, b| Value::Number((a ^ b) as f64)),
            BinOp::Shl => self.binop_int32(l, r, |a, b| Value::Number((a << b) as f64)),
            BinOp::Shr => self.binop_int32(l, r, |a, b| Value::Number((a >> b) as f64)),
            BinOp::In => {
                let key: Arc<str> = match l {
                    Value::String(s) => Arc::clone(s),
                    Value::Number(n) => n.to_string().into(),
                    _ => return Err(format!("'in' requires string or number key, got {:?}", l)),
                };
                let ok = match r {
                    Value::Object(map) => map.contains_key(&key),
                    Value::Array(arr) => {
                        key.as_ref() == "length"
                            || key
                                .parse::<usize>()
                                .ok()
                                .map(|i| i < arr.len())
                                .unwrap_or(false)
                    }
                    _ => return Err(format!("'in' requires object or array, got {:?}", r)),
                };
                Ok(Value::Bool(ok))
            }
            BinOp::Eq | BinOp::Ne => Err("Loose equality not supported".to_string()),
        }
    }

    fn to_int32(v: &Value) -> Result<i32, String> {
        match v {
            Value::Number(n) => Ok(*n as i32),
            _ => Err(format!("Bitwise operands must be numbers, got {:?}", v)),
        }
    }

    fn binop_int32<F>(&self, l: &Value, r: &Value, f: F) -> Result<Value, String>
    where
        F: FnOnce(i32, i32) -> Value,
    {
        let a = Self::to_int32(l)?;
        let b = Self::to_int32(r)?;
        Ok(f(a, b))
    }

    fn binop_number<F>(&self, l: &Value, r: &Value, f: F) -> Result<Value, String>
    where
        F: FnOnce(f64, f64) -> Value,
    {
        match (l, r) {
            (Value::Number(a), Value::Number(b)) => Ok(f(*a, *b)),
            _ => Err(format!("Expected numbers, got {:?} and {:?}", l, r)),
        }
    }

    fn eval_unary(&self, op: UnaryOp, v: &Value) -> Result<Value, String> {
        match op {
            UnaryOp::Not => Ok(Value::Bool(!v.is_truthy())),
            UnaryOp::Neg => match v {
                Value::Number(n) => Ok(Value::Number(-n)),
                _ => Err(format!("Cannot negate {:?}", v)),
            },
            UnaryOp::Pos => match v {
                Value::Number(n) => Ok(Value::Number(*n)),
                _ => Err(format!("Cannot apply unary + to {:?}", v)),
            },
            UnaryOp::BitNot => {
                let n = Self::to_int32(v)?;
                Ok(Value::Number((!n) as f64))
            }
            UnaryOp::Void => Ok(Value::Null),
        }
    }

    fn get_log_level() -> u8 {
        match std::env::var("TISH_LOG_LEVEL").as_deref() {
            Ok("debug") => 0,
            Ok("info") => 1,
            Ok("warn") => 3,
            Ok("error") => 4,
            _ => 2, // default: log
        }
    }

    fn call_func(&self, f: &Value, args: &[Value]) -> Result<Value, EvalError> {
        if matches!(f, Value::NativeConsoleDebug) {
            if Self::get_log_level() <= 0 {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                println!("{}", parts.join(" "));
            }
            return Ok(Value::Null);
        }
        if matches!(f, Value::NativeConsoleInfo) {
            if Self::get_log_level() <= 1 {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                println!("{}", parts.join(" "));
            }
            return Ok(Value::Null);
        }
        if matches!(f, Value::NativeConsoleLog) {
            if Self::get_log_level() <= 2 {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                println!("{}", parts.join(" "));
            }
            return Ok(Value::Null);
        }
        if matches!(f, Value::NativeConsoleWarn) {
            if Self::get_log_level() <= 3 {
                let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
                eprintln!("{}", parts.join(" "));
            }
            return Ok(Value::Null);
        }
        if matches!(f, Value::NativeConsoleError) {
            let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
            eprintln!("{}", parts.join(" "));
            return Ok(Value::Null);
        }
        if matches!(f, Value::NativeParseInt) {
            let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
            let s = s.trim();
            let radix = args
                .get(1)
                .and_then(|v| match v {
                    Value::Number(n) => Some(*n as i32),
                    _ => None,
                })
                .unwrap_or(10);
            let n = if radix >= 2 && radix <= 36 {
                let prefix: String = s
                    .chars()
                    .take_while(|c| *c == '-' || *c == '+' || c.is_digit(radix as u32))
                    .collect();
                i64::from_str_radix(&prefix, radix as u32).ok().map(|n| n as f64)
            } else {
                None
            };
            return Ok(Value::Number(n.unwrap_or(f64::NAN)));
        }
        if matches!(f, Value::NativeParseFloat) {
            let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
            let n: f64 = s.trim().parse().unwrap_or(f64::NAN);
            return Ok(Value::Number(n));
        }
        if matches!(f, Value::NativeIsFinite) {
            let b = args.get(0).map_or(false, |v| match v {
                Value::Number(n) => n.is_finite(),
                _ => false,
            });
            return Ok(Value::Bool(b));
        }
        if matches!(f, Value::NativeMathAbs) {
            let n = args
                .get(0)
                .and_then(|v| match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(f64::NAN);
            return Ok(Value::Number(n.abs()));
        }
        if matches!(f, Value::NativeMathSqrt) {
            let n = args
                .get(0)
                .and_then(|v| match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(f64::NAN);
            return Ok(Value::Number(n.sqrt()));
        }
        if matches!(f, Value::NativeMathMin) {
            let nums: Vec<f64> = args
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .collect();
            let n = nums.into_iter().fold(f64::INFINITY, f64::min);
            return Ok(Value::Number(if n == f64::INFINITY { f64::NAN } else { n }));
        }
        if matches!(f, Value::NativeMathMax) {
            let nums: Vec<f64> = args
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .collect();
            let n = nums.into_iter().fold(f64::NEG_INFINITY, f64::max);
            return Ok(Value::Number(if n == f64::NEG_INFINITY { f64::NAN } else { n }));
        }
        if matches!(f, Value::NativeMathFloor) {
            let n = args.get(0).and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.floor()));
        }
        if matches!(f, Value::NativeMathCeil) {
            let n = args.get(0).and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.ceil()));
        }
        if matches!(f, Value::NativeMathRound) {
            let n = args.get(0).and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.round()));
        }
        if matches!(f, Value::NativeIsNaN) {
            let b = args.get(0).map_or(true, |v| match v {
                Value::Number(n) => n.is_nan(),
                _ => true,
            });
            return Ok(Value::Bool(b));
        }
        if matches!(f, Value::NativeJsonParse) {
            let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
            return Ok(Self::json_parse(&s));
        }
        if matches!(f, Value::NativeJsonStringify) {
            let v = args.get(0).cloned().unwrap_or(Value::Null);
            return Ok(Value::String(Self::json_stringify_value(&v).into()));
        }
        if matches!(f, Value::NativeDecodeURI) {
            let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
            return Ok(Value::String(Self::percent_decode(&s).into()));
        }
        if matches!(f, Value::NativeEncodeURI) {
            let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
            return Ok(Value::String(Self::percent_encode(&s).into()));
        }
        let (params, rest_param, body) = match f {
            Value::Function { params, rest_param, body } => {
                (params.clone(), rest_param.clone(), Box::clone(body))
            }
            _ => return Err(EvalError::Error("Not a function".to_string())),
        };
        // Create new scope with params, parent = current scope
        let scope = Scope::child(Rc::clone(&self.scope));
        {
            let mut s = scope.borrow_mut();
            for (i, p) in params.iter().enumerate() {
                let val = args.get(i).cloned().unwrap_or(Value::Null);
                s.set(Arc::clone(p), val);
            }
            if let Some(rest_name) = rest_param {
                let rest_vals: Vec<Value> = args.iter().skip(params.len()).cloned().collect();
                s.set(rest_name, Value::Array(Rc::new(rest_vals)));
            }
        }
        let mut eval = Evaluator {
            scope: scope,
        };
        match eval.eval_statement(&body) {
            Ok(v) => Ok(v),
            Err(EvalError::Return(v)) => Ok(v),
            Err(EvalError::Throw(v)) => Err(EvalError::Throw(v)),
            Err(EvalError::Error(s)) => Err(EvalError::Error(s)),
            Err(EvalError::Break) => Err(EvalError::Error("break outside loop".to_string())),
            Err(EvalError::Continue) => Err(EvalError::Error("continue outside loop".to_string())),
        }
    }

    fn get_prop(&self, obj: &Value, key: &str) -> Result<Value, String> {
        match obj {
            Value::Object(map) => map
                .get(key)
                .cloned()
                .ok_or_else(|| format!("Property not found: {}", key)),
            Value::Array(arr) => {
                if key == "length" {
                    Ok(Value::Number(arr.len() as f64))
                } else {
                    let idx: usize = key.parse().map_err(|_| "Invalid array index")?;
                    arr.get(idx)
                        .cloned()
                        .ok_or_else(|| format!("Index out of bounds: {}", idx))
                }
            }
            Value::String(s) => {
                if key == "length" {
                    Ok(Value::Number(s.chars().count() as f64))
                } else {
                    Err(format!("Cannot read property '{}' of string", key))
                }
            }
            _ => Err(format!("Cannot read property of {:?}", obj)),
        }
    }

    fn get_index(&self, obj: &Value, index: &Value) -> Result<Value, String> {
        match obj {
            Value::Array(arr) => {
                let idx = match index {
                    Value::Number(n) => *n as usize,
                    _ => return Err("Index must be number".to_string()),
                };
                arr.get(idx)
                    .cloned()
                    .ok_or_else(|| format!("Index out of bounds: {}", idx))
            }
            Value::Object(map) => {
                let key: Arc<str> = match index {
                    Value::Number(n) => n.to_string().into(),
                    Value::String(s) => Arc::clone(s),
                    _ => return Err("Index must be number or string".to_string()),
                };
                Ok(map.get(&key).cloned().unwrap_or(Value::Null))
            }
            _ => Err(format!("Cannot index {:?}", obj)),
        }
    }

    fn json_parse(s: &str) -> Value {
        let s = s.trim();
        if s.is_empty() {
            return Value::Null;
        }
        match Self::json_parse_str(s) {
            Ok(v) => v,
            Err(()) => Value::Null,
        }
    }

    fn json_parse_str(s: &str) -> Result<Value, ()> {
        let s = s.trim();
        if s.is_empty() {
            return Err(());
        }
        if s == "null" {
            return Ok(Value::Null);
        }
        if s == "true" {
            return Ok(Value::Bool(true));
        }
        if s == "false" {
            return Ok(Value::Bool(false));
        }
        if s.starts_with('"') {
            return Self::json_parse_string_full(s);
        }
        if s.starts_with('[') {
            return Self::json_parse_array(s);
        }
        if s.starts_with('{') {
            return Self::json_parse_object(s);
        }
        if let Ok(n) = s.parse::<f64>() {
            return Ok(Value::Number(n));
        }
        Err(())
    }

    fn json_parse_string(s: &str) -> Result<(Value, &str), ()> {
        let s = &s[1..];
        let mut out = String::new();
        let mut i = 0;
        let chars: Vec<char> = s.chars().collect();
        while i < chars.len() {
            if chars[i] == '"' {
                let rest_start = s.chars().take(i + 1).map(|c| c.len_utf8()).sum::<usize>();
                return Ok((Value::String(out.into()), &s[rest_start..]));
            }
            if chars[i] == '\\' {
                i += 1;
                if i >= chars.len() {
                    return Err(());
                }
                match chars[i] {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    _ => return Err(()),
                }
            } else {
                out.push(chars[i]);
            }
            i += 1;
        }
        Err(())
    }

    fn json_parse_string_full(s: &str) -> Result<Value, ()> {
        Self::json_parse_string(s).map(|(v, _)| v)
    }

    fn json_parse_array(s: &str) -> Result<Value, ()> {
        let s = s[1..].trim_start();
        if s.starts_with(']') {
            return Ok(Value::Array(Rc::new(vec![])));
        }
        let mut vals = Vec::new();
        let mut rest = s;
        loop {
            let (v, next) = Self::json_parse_one(rest)?;
            vals.push(v);
            rest = next.trim_start();
            if rest.starts_with(']') {
                break;
            }
            if !rest.starts_with(',') {
                return Err(());
            }
            rest = rest[1..].trim_start();
        }
        Ok(Value::Array(Rc::new(vals)))
    }

    fn json_parse_object(s: &str) -> Result<Value, ()> {
        let s = s[1..].trim_start();
        if s.starts_with('}') {
            return Ok(Value::Object(Rc::new(HashMap::new())));
        }
        let mut map = HashMap::new();
        let mut rest = s;
        loop {
            if !rest.starts_with('"') {
                return Err(());
            }
            let (key_val, next) = Self::json_parse_string(rest)?;
            let key = match &key_val {
                Value::String(k) => Arc::clone(k),
                _ => return Err(()),
            };
            rest = next.trim_start();
            if !rest.starts_with(':') {
                return Err(());
            }
            rest = rest[1..].trim_start();
            let (val, next) = Self::json_parse_one(rest)?;
            map.insert(key, val);
            rest = next.trim_start();
            if rest.starts_with('}') {
                break;
            }
            if !rest.starts_with(',') {
                return Err(());
            }
            rest = rest[1..].trim_start();
        }
        Ok(Value::Object(Rc::new(map)))
    }

    fn json_parse_one(s: &str) -> Result<(Value, &str), ()> {
        let s = s.trim();
        if s.is_empty() {
            return Err(());
        }
        if s.starts_with('"') {
            let (v, rest) = Self::json_parse_string(s)?;
            Ok((v, rest))
        } else if s.starts_with('[') {
            let mut depth = 0;
            let mut i = 0;
            for c in s.chars() {
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                    if depth == 0 {
                        let v = Self::json_parse_array(&s[..=i])?;
                        return Ok((v, &s[i + 1..]));
                    }
                }
                i += 1;
            }
            Err(())
        } else if s.starts_with('{') {
            let mut depth = 0;
            let mut i = 0;
            for c in s.chars() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        let v = Self::json_parse_object(&s[..=i])?;
                        return Ok((v, &s[i + 1..]));
                    }
                }
                i += 1;
            }
            Err(())
        } else if s.starts_with("null") {
            Ok((Value::Null, &s[4..]))
        } else if s.starts_with("true") {
            Ok((Value::Bool(true), &s[4..]))
        } else if s.starts_with("false") {
            Ok((Value::Bool(false), &s[5..]))
        } else {
            let end = s
                .find(|c: char| !c.is_ascii_digit() && c != '-' && c != '+' && c != '.' && c != 'e' && c != 'E')
                .unwrap_or(s.len());
            let num_str = &s[..end];
            let n: f64 = num_str.parse().map_err(|_| ())?;
            Ok((Value::Number(n), &s[end..]))
        }
    }

    fn json_stringify_value(v: &Value) -> String {
        match v {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => {
                if n.is_finite() {
                    n.to_string()
                } else {
                    "null".to_string()
                }
            }
            Value::String(s) => format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t")
            ),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.iter().map(Self::json_stringify_value).collect();
                format!("[{}]", inner.join(","))
            }
            Value::Object(map) => {
                let mut entries: Vec<_> = map
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_ref(),
                            format!(
                                "\"{}\":{}",
                                k.replace('\\', "\\\\").replace('"', "\\\""),
                                Self::json_stringify_value(v)
                            ),
                        )
                    })
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                format!("{{{}}}", entries.into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(","))
            }
            Value::Function { .. }
            | Value::NativeConsoleDebug
            | Value::NativeConsoleInfo
            | Value::NativeConsoleLog
            | Value::NativeConsoleWarn
            | Value::NativeConsoleError
            | Value::NativeParseInt
            | Value::NativeParseFloat
            | Value::NativeIsFinite
            | Value::NativeIsNaN
            | Value::NativeMathAbs
            | Value::NativeMathSqrt
            | Value::NativeMathMin
            | Value::NativeMathMax
            | Value::NativeMathFloor
            | Value::NativeMathCeil
            | Value::NativeMathRound
            | Value::NativeJsonParse
            | Value::NativeJsonStringify
            | Value::NativeDecodeURI
            | Value::NativeEncodeURI => "null".to_string(),
        }
    }

    fn percent_decode(s: &str) -> String {
        let mut out = Vec::new();
        let mut i = 0;
        let bytes = s.as_bytes();
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let h = (bytes[i + 1] as char).to_digit(16);
                let l = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn percent_encode(s: &str) -> String {
        const SAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789;-/?:@&=+$,_.!~*'()";
        let mut out = String::new();
        for c in s.chars() {
            if c.len_utf8() == 1 {
                let b = c as u32 as u8;
                if SAFE.contains(&b) {
                    out.push(c);
                    continue;
                }
            }
            for b in c.to_string().as_bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
        out
    }
}

#[derive(Debug)]
enum EvalError {
    Return(Value),
    Break,
    Continue,
    Throw(Value),
    Error(String),
}

impl EvalError {
    fn to_string(&self) -> String {
        match self {
            EvalError::Return(_) => "return".to_string(),
            EvalError::Break => "break".to_string(),
            EvalError::Continue => "continue".to_string(),
            EvalError::Throw(v) => v.to_string(),
            EvalError::Error(s) => s.clone(),
        }
    }
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EvalError::Return(_) => write!(f, "return"),
            EvalError::Break => write!(f, "break"),
            EvalError::Continue => write!(f, "continue"),
            EvalError::Throw(v) => write!(f, "{}", v.to_string()),
            EvalError::Error(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for EvalError {}
