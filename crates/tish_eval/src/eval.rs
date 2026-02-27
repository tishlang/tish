//! Tree-walk evaluator for Tish.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_ast::{BinOp, CompoundOp, Expr, Literal, MemberProp, Span, Statement, UnaryOp};

use crate::value::Value;

struct Scope {
    vars: HashMap<Arc<str>, Value>,
    consts: std::collections::HashSet<Arc<str>>,
    parent: Option<Rc<std::cell::RefCell<Scope>>>,
}

impl Scope {
    fn new() -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: HashMap::new(),
            consts: std::collections::HashSet::new(),
            parent: None,
        }))
    }

    fn child(parent: Rc<std::cell::RefCell<Scope>>) -> Rc<std::cell::RefCell<Self>> {
        Rc::new(std::cell::RefCell::new(Self {
            vars: HashMap::new(),
            consts: std::collections::HashSet::new(),
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

    fn set(&mut self, name: Arc<str>, value: Value, mutable: bool) {
        if !mutable {
            self.consts.insert(Arc::clone(&name));
        }
        self.vars.insert(name, value);
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<bool, String> {
        if self.vars.contains_key(name) {
            if self.consts.contains(name) {
                return Err(format!("Cannot assign to const variable: {}", name));
            }
            self.vars.insert(name.into(), value);
            return Ok(true);
        }
        self.parent
            .as_ref()
            .map(|p| p.borrow_mut().assign(name, value))
            .unwrap_or(Ok(false))
    }
}

pub struct Evaluator {
    scope: Rc<std::cell::RefCell<Scope>>,
}

impl Evaluator {
    #[allow(clippy::new_without_default)]
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
            s.set("console".into(), Value::Object(Rc::new(RefCell::new(console))), true);
            s.set("parseInt".into(), Value::NativeParseInt, true);
            s.set("parseFloat".into(), Value::NativeParseFloat, true);
            s.set("decodeURI".into(), Value::NativeDecodeURI, true);
            s.set("encodeURI".into(), Value::NativeEncodeURI, true);
            s.set("isFinite".into(), Value::NativeIsFinite, true);
            s.set("isNaN".into(), Value::NativeIsNaN, true);
            s.set("Infinity".into(), Value::Number(f64::INFINITY), true);
            s.set("NaN".into(), Value::Number(f64::NAN), true);
            let mut math = HashMap::new();
            math.insert("abs".into(), Value::NativeMathAbs);
            math.insert("sqrt".into(), Value::NativeMathSqrt);
            math.insert("min".into(), Value::NativeMathMin);
            math.insert("max".into(), Value::NativeMathMax);
            math.insert("floor".into(), Value::NativeMathFloor);
            math.insert("ceil".into(), Value::NativeMathCeil);
            math.insert("round".into(), Value::NativeMathRound);
            s.set("Math".into(), Value::Object(Rc::new(RefCell::new(math))), true);
            let mut json = HashMap::new();
            json.insert("parse".into(), Value::NativeJsonParse);
            json.insert("stringify".into(), Value::NativeJsonStringify);
            s.set("JSON".into(), Value::Object(Rc::new(RefCell::new(json))), true);
            let mut object = HashMap::new();
            object.insert("keys".into(), Value::NativeObjectKeys);
            object.insert("values".into(), Value::NativeObjectValues);
            object.insert("entries".into(), Value::NativeObjectEntries);
            s.set("Object".into(), Value::Object(Rc::new(RefCell::new(object))), true);

            #[cfg(feature = "http")]
            {
                s.set("fetch".into(), Value::NativeFetch, true);
                s.set("fetchAll".into(), Value::NativeFetchAll, true);
            }
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
            Statement::VarDecl { name, mutable, init, .. } => {
                let value = init
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .transpose()?
                    .unwrap_or(Value::Null);
                self.scope.borrow_mut().set(Arc::clone(name), value, *mutable);
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
                    crate::value::Value::Array(arr) => arr.borrow().iter().cloned().collect::<Vec<_>>(),
                    crate::value::Value::String(s) => {
                        s.chars()
                            .map(|c| crate::value::Value::String(Arc::from(c.to_string())))
                            .collect::<Vec<_>>()
                    }
                    _ => {
                        return Err(EvalError::Error(format!(
                            "for-of requires iterable (array or string), got {}",
                            iter_val
                        )));
                    }
                };
                for elem in elements {
                    self.scope.borrow_mut().set(Arc::clone(name), elem, true);
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
                // Extract just the parameter names (ignoring type annotations for now)
                let param_names: Vec<Arc<str>> = params.iter().map(|p| Arc::clone(&p.name)).collect();
                let rest_param_name = rest_param.as_ref().map(|p| Arc::clone(&p.name));
                let body = Box::clone(body);
                let _scope = Rc::clone(&self.scope);
                let func = Value::Function {
                    params: param_names,
                    rest_param: rest_param_name,
                    body,
                };
                self.scope.borrow_mut().set(Arc::clone(name), func, true);
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
                            scope.borrow_mut().set(Arc::clone(param), thrown, true);
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
                // Check for built-in method calls on arrays/strings
                if let Expr::Member { object, prop: MemberProp::Name(method_name), .. } = callee.as_ref() {
                    let obj = self.eval_expr(object)?;
                    let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                    let arg_vals = arg_vals?;
                    
                    // Array methods
                    if let Value::Array(arr) = &obj {
                        match method_name.as_ref() {
                            "push" => {
                                let mut arr_mut = arr.borrow_mut();
                                for v in &arg_vals {
                                    arr_mut.push(v.clone());
                                }
                                return Ok(Value::Number(arr_mut.len() as f64));
                            }
                            "pop" => {
                                return Ok(arr.borrow_mut().pop().unwrap_or(Value::Null));
                            }
                            "shift" => {
                                let mut arr_mut = arr.borrow_mut();
                                if arr_mut.is_empty() {
                                    return Ok(Value::Null);
                                }
                                return Ok(arr_mut.remove(0));
                            }
                            "unshift" => {
                                let mut arr_mut = arr.borrow_mut();
                                for (i, v) in arg_vals.iter().enumerate() {
                                    arr_mut.insert(i, v.clone());
                                }
                                return Ok(Value::Number(arr_mut.len() as f64));
                            }
                            "indexOf" => {
                                let search = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    if v.strict_eq(&search) {
                                        return Ok(Value::Number(i as f64));
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "includes" => {
                                let search = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for v in arr_borrow.iter() {
                                    if v.strict_eq(&search) {
                                        return Ok(Value::Bool(true));
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "join" => {
                                let sep = match arg_vals.get(0) {
                                    Some(Value::String(s)) => s.to_string(),
                                    _ => ",".to_string(),
                                };
                                let arr_borrow = arr.borrow();
                                let parts: Vec<String> = arr_borrow.iter().map(|v| v.to_string()).collect();
                                return Ok(Value::String(parts.join(&sep).into()));
                            }
                            "reverse" => {
                                arr.borrow_mut().reverse();
                                return Ok(obj.clone());
                            }
                            "slice" => {
                                let arr_borrow = arr.borrow();
                                let len = arr_borrow.len() as i64;
                                let start = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => len as usize,
                                };
                                let sliced: Vec<Value> = if start < end {
                                    arr_borrow[start..end].to_vec()
                                } else {
                                    vec![]
                                };
                                return Ok(Value::Array(Rc::new(RefCell::new(sliced))));
                            }
                            "concat" => {
                                let mut result = arr.borrow().clone();
                                for v in &arg_vals {
                                    if let Value::Array(other) = v {
                                        result.extend(other.borrow().iter().cloned());
                                    } else {
                                        result.push(v.clone());
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "map" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let mut result = Vec::with_capacity(arr_borrow.len());
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let mapped = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    result.push(mapped);
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "filter" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let mut result = Vec::new();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let keep = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    if keep.is_truthy() {
                                        result.push(v.clone());
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "reduce" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let (mut acc, start_idx) = if arg_vals.len() > 1 {
                                    (arg_vals[1].clone(), 0)
                                } else if !arr_borrow.is_empty() {
                                    (arr_borrow[0].clone(), 1)
                                } else {
                                    return Err(EvalError::Error("Reduce of empty array with no initial value".to_string()));
                                };
                                for (i, v) in arr_borrow.iter().enumerate().skip(start_idx) {
                                    acc = self.call_func(&callback, &[acc, v.clone(), Value::Number(i as f64)])?;
                                }
                                return Ok(acc);
                            }
                            "find" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    if found.is_truthy() {
                                        return Ok(v.clone());
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "findIndex" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    if found.is_truthy() {
                                        return Ok(Value::Number(i as f64));
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "forEach" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                }
                                return Ok(Value::Null);
                            }
                            "some" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    if result.is_truthy() {
                                        return Ok(Value::Bool(true));
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "every" => {
                                let callback = arg_vals.get(0).cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    if !result.is_truthy() {
                                        return Ok(Value::Bool(false));
                                    }
                                }
                                return Ok(Value::Bool(true));
                            }
                            "flat" => {
                                let depth = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 1,
                                };
                                fn flatten(arr: &[Value], depth: usize) -> Vec<Value> {
                                    let mut result = Vec::new();
                                    for v in arr {
                                        if depth > 0 {
                                            if let Value::Array(inner) = v {
                                                result.extend(flatten(&inner.borrow(), depth - 1));
                                                continue;
                                            }
                                        }
                                        result.push(v.clone());
                                    }
                                    result
                                }
                                let flattened = flatten(&arr.borrow(), depth);
                                return Ok(Value::Array(Rc::new(RefCell::new(flattened))));
                            }
                            _ => {}
                        }
                    }
                    
                    // String methods
                    if let Value::String(s) = &obj {
                        match method_name.as_ref() {
                            "indexOf" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Number(-1.0)),
                                };
                                return Ok(Value::Number(
                                    s.find(search).map(|i| i as f64).unwrap_or(-1.0)
                                ));
                            }
                            "includes" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.contains(search)));
                            }
                            "slice" => {
                                let chars: Vec<char> = s.chars().collect();
                                let len = chars.len() as i64;
                                let start = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => len as usize,
                                };
                                let sliced: String = if start < end {
                                    chars[start..end].iter().collect()
                                } else {
                                    String::new()
                                };
                                return Ok(Value::String(sliced.into()));
                            }
                            "substring" => {
                                let chars: Vec<char> = s.chars().collect();
                                let len = chars.len();
                                let start = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => (*n as usize).min(len),
                                    _ => 0,
                                };
                                let end = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => (*n as usize).min(len),
                                    _ => len,
                                };
                                let (s, e) = (start.min(end), start.max(end));
                                return Ok(Value::String(chars[s..e].iter().collect::<String>().into()));
                            }
                            "split" => {
                                let sep = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Array(Rc::new(RefCell::new(vec![obj.clone()])))),
                                };
                                let parts: Vec<Value> = s.split(sep)
                                    .map(|p| Value::String(p.into()))
                                    .collect();
                                return Ok(Value::Array(Rc::new(RefCell::new(parts))));
                            }
                            "trim" => {
                                return Ok(Value::String(s.trim().into()));
                            }
                            "toUpperCase" => {
                                return Ok(Value::String(s.to_uppercase().into()));
                            }
                            "toLowerCase" => {
                                return Ok(Value::String(s.to_lowercase().into()));
                            }
                            "startsWith" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.starts_with(search)));
                            }
                            "endsWith" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.ends_with(search)));
                            }
                            "replace" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => return Ok(obj.clone()),
                                };
                                let replacement = match arg_vals.get(1) {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => String::new(),
                                };
                                return Ok(Value::String(s.replacen(&search, &replacement, 1).into()));
                            }
                            "replaceAll" => {
                                let search = match arg_vals.get(0) {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => return Ok(obj.clone()),
                                };
                                let replacement = match arg_vals.get(1) {
                                    Some(Value::String(ss)) => ss.to_string(),
                                    _ => String::new(),
                                };
                                return Ok(Value::String(s.replace(&search, &replacement).into()));
                            }
                            "charAt" => {
                                let idx = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                let chars: Vec<char> = s.chars().collect();
                                return Ok(chars.get(idx)
                                    .map(|c| Value::String(c.to_string().into()))
                                    .unwrap_or(Value::String("".into())));
                            }
                            "charCodeAt" => {
                                let idx = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                let chars: Vec<char> = s.chars().collect();
                                return Ok(chars.get(idx)
                                    .map(|c| Value::Number(*c as u32 as f64))
                                    .unwrap_or(Value::Number(f64::NAN)));
                            }
                            "repeat" => {
                                let count = match arg_vals.get(0) {
                                    Some(Value::Number(n)) if *n >= 0.0 => *n as usize,
                                    _ => 0,
                                };
                                return Ok(Value::String(s.repeat(count).into()));
                            }
                            "padStart" => {
                                let target_len = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let chars: Vec<char> = s.chars().collect();
                                if chars.len() >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - chars.len();
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", padding, s).into()));
                            }
                            "padEnd" => {
                                let target_len = match arg_vals.get(0) {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let chars: Vec<char> = s.chars().collect();
                                if chars.len() >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - chars.len();
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", s, padding).into()));
                            }
                            _ => {}
                        }
                    }
                    
                    // Fall through to normal function call
                    let f = self.get_prop(&obj, method_name).map_err(EvalError::Error)?;
                    return self.call_func(&f, &arg_vals);
                }
                
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
                Ok(Value::Array(Rc::new(RefCell::new(vals?))))
            }
            Expr::Object { props, .. } => {
                let mut map = HashMap::new();
                for (k, v) in props {
                    map.insert(Arc::clone(k), self.eval_expr(v)?);
                }
                Ok(Value::Object(Rc::new(RefCell::new(map))))
            }
            Expr::Assign { name, value, .. } => {
                let v = self.eval_expr(value)?;
                match self.scope.borrow_mut().assign(name.as_ref(), v.clone()) {
                    Ok(true) => Ok(v),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
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
                    | Value::NativeEncodeURI
                    | Value::NativeObjectKeys
                    | Value::NativeObjectValues
                    | Value::NativeObjectEntries => "function".into(),
                    #[cfg(feature = "http")]
                    Value::NativeFetch | Value::NativeFetchAll => "function".into(),
                }))
            }
            Expr::PostfixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                match self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n + 1.0)) {
                    Ok(true) => Ok(Value::Number(n)),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PostfixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                match self.scope.borrow_mut().assign(name.as_ref(), Value::Number(n - 1.0)) {
                    Ok(true) => Ok(Value::Number(n)),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PrefixInc { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply ++ to {:?}", v))),
                };
                let new_val = Value::Number(n + 1.0);
                match self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    Ok(true) => Ok(new_val),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::PrefixDec { name, .. } => {
                let v = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let n = match &v {
                    Value::Number(x) => *x,
                    _ => return Err(EvalError::Error(format!("Cannot apply -- to {:?}", v))),
                };
                let new_val = Value::Number(n - 1.0);
                match self.scope.borrow_mut().assign(name.as_ref(), new_val.clone()) {
                    Ok(true) => Ok(new_val),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                let current = self.scope.borrow().get(name.as_ref())
                    .ok_or_else(|| EvalError::Error(format!("Undefined variable: {}", name)))?;
                let rhs = self.eval_expr(value)?;
                let bin_op = match op {
                    CompoundOp::Add => BinOp::Add,
                    CompoundOp::Sub => BinOp::Sub,
                    CompoundOp::Mul => BinOp::Mul,
                    CompoundOp::Div => BinOp::Div,
                    CompoundOp::Mod => BinOp::Mod,
                };
                let result = self.eval_binop(&current, bin_op, &rhs).map_err(EvalError::Error)?;
                match self.scope.borrow_mut().assign(name.as_ref(), result.clone()) {
                    Ok(true) => Ok(result),
                    Ok(false) => Err(EvalError::Error(format!("Undefined variable: {}", name))),
                    Err(e) => Err(EvalError::Error(e)),
                }
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                let obj_val = self.eval_expr(object)?;
                let val = self.eval_expr(value)?;
                match obj_val {
                    Value::Object(map) => {
                        map.borrow_mut().insert(Arc::clone(prop), val.clone());
                        Ok(val)
                    }
                    _ => Err(EvalError::Error(format!(
                        "Cannot assign property '{}' on non-object: {:?}",
                        prop, obj_val
                    ))),
                }
            }
            Expr::IndexAssign { object, index, value, .. } => {
                let obj_val = self.eval_expr(object)?;
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;
                match obj_val {
                    Value::Array(arr) => {
                        let idx = match &idx_val {
                            Value::Number(n) => *n as usize,
                            _ => return Err(EvalError::Error(format!(
                                "Array index must be a number, got {:?}",
                                idx_val
                            ))),
                        };
                        let mut arr_mut = arr.borrow_mut();
                        // Extend array if necessary (JS behavior)
                        while arr_mut.len() <= idx {
                            arr_mut.push(Value::Null);
                        }
                        arr_mut[idx] = val.clone();
                        Ok(val)
                    }
                    Value::Object(map) => {
                        let key: Arc<str> = match &idx_val {
                            Value::Number(n) => n.to_string().into(),
                            Value::String(s) => Arc::clone(s),
                            _ => return Err(EvalError::Error(format!(
                                "Object key must be string or number, got {:?}",
                                idx_val
                            ))),
                        };
                        map.borrow_mut().insert(key, val.clone());
                        Ok(val)
                    }
                    _ => Err(EvalError::Error(format!(
                        "Cannot assign index on non-array/object: {:?}",
                        obj_val
                    ))),
                }
            }
            Expr::ArrowFunction { params, body, .. } => {
                use tish_ast::ArrowBody;
                // Convert arrow function to regular function
                let param_names: Vec<Arc<str>> = params.iter().map(|p| Arc::clone(&p.name)).collect();
                let body_stmt = match body {
                    ArrowBody::Expr(expr) => {
                        // Expression body: wrap in implicit return
                        Statement::Return {
                            value: Some(expr.as_ref().clone()),
                            span: Span { start: (0, 0), end: (0, 0) },
                        }
                    }
                    ArrowBody::Block(stmt) => stmt.as_ref().clone(),
                };
                Ok(Value::Function {
                    params: param_names,
                    rest_param: None,
                    body: Box::new(body_stmt),
                })
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                // Build the string by interleaving quasis and evaluated expressions
                let mut result = String::new();
                for (i, quasi) in quasis.iter().enumerate() {
                    result.push_str(quasi);
                    if i < exprs.len() {
                        let val = self.eval_expr(&exprs[i])?;
                        result.push_str(&val.to_string());
                    }
                }
                Ok(Value::String(result.into()))
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
                (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b).into())),
                (a, Value::String(b)) => Ok(Value::String(format!("{}{}", a, b).into())),
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
                    Value::Object(map) => map.borrow().contains_key(&key),
                    Value::Array(arr) => {
                        key.as_ref() == "length"
                            || key
                                .parse::<usize>()
                                .ok()
                                .map(|i| i < arr.borrow().len())
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
            if Self::get_log_level() == 0 {
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
            let s = args.first().map(|v| v.to_string()).unwrap_or_default();
            let s = s.trim();
            let radix = args
                .get(1)
                .and_then(|v| match v {
                    Value::Number(n) => Some(*n as i32),
                    _ => None,
                })
                .unwrap_or(10);
            let n = if (2..=36).contains(&radix) {
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
            let s = args.first().map(|v| v.to_string()).unwrap_or_default();
            let n: f64 = s.trim().parse().unwrap_or(f64::NAN);
            return Ok(Value::Number(n));
        }
        if matches!(f, Value::NativeIsFinite) {
            let b = args.first().is_some_and(|v| matches!(v, Value::Number(n) if n.is_finite()));
            return Ok(Value::Bool(b));
        }
        if matches!(f, Value::NativeMathAbs) {
            let n = args
                .first()
                .and_then(|v| match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(f64::NAN);
            return Ok(Value::Number(n.abs()));
        }
        if matches!(f, Value::NativeMathSqrt) {
            let n = args
                .first()
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
            let n = args.first().and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.floor()));
        }
        if matches!(f, Value::NativeMathCeil) {
            let n = args.first().and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.ceil()));
        }
        if matches!(f, Value::NativeMathRound) {
            let n = args.first().and_then(|v| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            }).unwrap_or(f64::NAN);
            return Ok(Value::Number(n.round()));
        }
        if matches!(f, Value::NativeIsNaN) {
            let b = args.first().is_none_or(|v| matches!(v, Value::Number(n) if n.is_nan()) || !matches!(v, Value::Number(_)));
            return Ok(Value::Bool(b));
        }
        if matches!(f, Value::NativeJsonParse) {
            let s = args.first().map(|v| v.to_string()).unwrap_or_default();
            return Ok(Self::json_parse(&s));
        }
        if matches!(f, Value::NativeJsonStringify) {
            let v = args.first().cloned().unwrap_or(Value::Null);
            return Ok(Value::String(Self::json_stringify_value(&v).into()));
        }
        if matches!(f, Value::NativeDecodeURI) {
            let s = args.first().map(|v| v.to_string()).unwrap_or_default();
            return Ok(Value::String(tish_core::percent_decode(&s).unwrap_or(s).into()));
        }
        if matches!(f, Value::NativeEncodeURI) {
            let s = args.first().map(|v| v.to_string()).unwrap_or_default();
            return Ok(Value::String(tish_core::percent_encode(&s).into()));
        }
        if matches!(f, Value::NativeObjectKeys) {
            if let Some(Value::Object(obj)) = args.first() {
                let keys: Vec<Value> = obj.borrow().keys().map(|k| Value::String(Arc::clone(k))).collect();
                return Ok(Value::Array(Rc::new(RefCell::new(keys))));
            }
            return Ok(Value::Array(Rc::new(RefCell::new(vec![]))));
        }
        if matches!(f, Value::NativeObjectValues) {
            if let Some(Value::Object(obj)) = args.first() {
                let vals: Vec<Value> = obj.borrow().values().cloned().collect();
                return Ok(Value::Array(Rc::new(RefCell::new(vals))));
            }
            return Ok(Value::Array(Rc::new(RefCell::new(vec![]))));
        }
        if matches!(f, Value::NativeObjectEntries) {
            if let Some(Value::Object(obj)) = args.first() {
                let entries: Vec<Value> = obj.borrow().iter().map(|(k, v)| {
                    Value::Array(Rc::new(RefCell::new(vec![Value::String(Arc::clone(k)), v.clone()])))
                }).collect();
                return Ok(Value::Array(Rc::new(RefCell::new(entries))));
            }
            return Ok(Value::Array(Rc::new(RefCell::new(vec![]))));
        }
        #[cfg(feature = "http")]
        if matches!(f, Value::NativeFetch) {
            return crate::http::fetch(args).map_err(EvalError::Error);
        }
        #[cfg(feature = "http")]
        if matches!(f, Value::NativeFetchAll) {
            return crate::http::fetch_all(args).map_err(EvalError::Error);
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
                s.set(Arc::clone(p), val, true);
            }
            if let Some(rest_name) = rest_param {
                let rest_vals: Vec<Value> = args.iter().skip(params.len()).cloned().collect();
                s.set(rest_name, Value::Array(Rc::new(RefCell::new(rest_vals))), true);
            }
        }
        let mut eval = Evaluator { scope };
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
            Value::Object(map) => Ok(map.borrow().get(key).cloned().unwrap_or(Value::Null)),
            Value::Array(arr) => {
                if key == "length" {
                    Ok(Value::Number(arr.borrow().len() as f64))
                } else if let Ok(idx) = key.parse::<usize>() {
                    Ok(arr.borrow().get(idx).cloned().unwrap_or(Value::Null))
                } else {
                    Ok(Value::Null)
                }
            }
            Value::String(s) => {
                if key == "length" {
                    Ok(Value::Number(s.chars().count() as f64))
                } else {
                    Ok(Value::Null)
                }
            }
            _ => Ok(Value::Null),
        }
    }

    fn get_index(&self, obj: &Value, index: &Value) -> Result<Value, String> {
        match obj {
            Value::Array(arr) => {
                let idx = match index {
                    Value::Number(n) => *n as usize,
                    _ => return Ok(Value::Null),
                };
                Ok(arr.borrow().get(idx).cloned().unwrap_or(Value::Null))
            }
            Value::Object(map) => {
                let key: Arc<str> = match index {
                    Value::Number(n) => n.to_string().into(),
                    Value::String(s) => Arc::clone(s),
                    _ => return Ok(Value::Null),
                };
                Ok(map.borrow().get(&key).cloned().unwrap_or(Value::Null))
            }
            _ => Ok(Value::Null),
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
            return Ok(Value::Array(Rc::new(RefCell::new(vec![]))));
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
        Ok(Value::Array(Rc::new(RefCell::new(vals))))
    }

    fn json_parse_object(s: &str) -> Result<Value, ()> {
        let s = s[1..].trim_start();
        if s.starts_with('}') {
            return Ok(Value::Object(Rc::new(RefCell::new(HashMap::new()))));
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
        Ok(Value::Object(Rc::new(RefCell::new(map))))
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
            for (i, c) in s.char_indices() {
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                    if depth == 0 {
                        let v = Self::json_parse_array(&s[..=i])?;
                        return Ok((v, &s[i + c.len_utf8()..]));
                    }
                }
            }
            Err(())
        } else if s.starts_with('{') {
            let mut depth = 0;
            for (i, c) in s.char_indices() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        let v = Self::json_parse_object(&s[..=i])?;
                        return Ok((v, &s[i + c.len_utf8()..]));
                    }
                }
            }
            Err(())
        } else if let Some(rest) = s.strip_prefix("null") {
            Ok((Value::Null, rest))
        } else if let Some(rest) = s.strip_prefix("true") {
            Ok((Value::Bool(true), rest))
        } else if let Some(rest) = s.strip_prefix("false") {
            Ok((Value::Bool(false), rest))
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
                let inner: Vec<String> = arr.borrow().iter().map(Self::json_stringify_value).collect();
                format!("[{}]", inner.join(","))
            }
            Value::Object(map) => {
                let mut entries: Vec<_> = map
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_ref().to_string(),
                            format!(
                                "\"{}\":{}",
                                k.replace('\\', "\\\\").replace('"', "\\\""),
                                Self::json_stringify_value(v)
                            ),
                        )
                    })
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
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
            | Value::NativeEncodeURI
            | Value::NativeObjectKeys
            | Value::NativeObjectValues
            | Value::NativeObjectEntries => "null".to_string(),
            #[cfg(feature = "http")]
            Value::NativeFetch | Value::NativeFetchAll => "null".to_string(),
        }
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

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EvalError::Return(_) => write!(f, "return"),
            EvalError::Break => write!(f, "break"),
            EvalError::Continue => write!(f, "continue"),
            EvalError::Throw(v) => write!(f, "{}", v),
            EvalError::Error(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for EvalError {}
