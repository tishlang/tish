//! Tree-walk evaluator for Tish.

#![allow(clippy::type_complexity, clippy::cloned_ref_to_slice_refs)]

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
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        if let Some(ref parent) = self.parent {
            return parent.borrow().get(name);
        }
        None
    }

    fn set(&mut self, name: Arc<str>, value: Value, mutable: bool) {
        if !mutable {
            self.consts.insert(Arc::clone(&name));
        }
        self.vars.insert(name, value);
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<bool, String> {
        if let Some(existing) = self.vars.get_mut(name) {
            if self.consts.contains(name) {
                return Err(format!("Cannot assign to const variable: {}", name));
            }
            *existing = value;
            return Ok(true);
        }
        if let Some(ref parent) = self.parent {
            return parent.borrow_mut().assign(name, value);
        }
        Ok(false)
    }
}

pub struct Evaluator {
    scope: Rc<std::cell::RefCell<Scope>>,
}

impl Evaluator {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        use crate::natives;
        
        let scope = Scope::new();
        {
            let mut s = scope.borrow_mut();
            let mut console = HashMap::with_capacity(5);
            console.insert("debug".into(), Value::Native(natives::console_debug));
            console.insert("info".into(), Value::Native(natives::console_info));
            console.insert("log".into(), Value::Native(natives::console_log));
            console.insert("warn".into(), Value::Native(natives::console_warn));
            console.insert("error".into(), Value::Native(natives::console_error));
            s.set("console".into(), Value::Object(Rc::new(RefCell::new(console))), true);
            s.set("parseInt".into(), Value::Native(natives::parse_int), true);
            s.set("parseFloat".into(), Value::Native(natives::parse_float), true);
            s.set("decodeURI".into(), Value::Native(natives::decode_uri), true);
            s.set("encodeURI".into(), Value::Native(natives::encode_uri), true);
            s.set("isFinite".into(), Value::Native(natives::is_finite), true);
            s.set("isNaN".into(), Value::Native(natives::is_nan), true);
            s.set("Infinity".into(), Value::Number(f64::INFINITY), true);
            s.set("NaN".into(), Value::Number(f64::NAN), true);
            let mut math = HashMap::with_capacity(18);
            math.insert("abs".into(), Value::Native(natives::math_abs));
            math.insert("sqrt".into(), Value::Native(natives::math_sqrt));
            math.insert("min".into(), Value::Native(natives::math_min));
            math.insert("max".into(), Value::Native(natives::math_max));
            math.insert("floor".into(), Value::Native(natives::math_floor));
            math.insert("ceil".into(), Value::Native(natives::math_ceil));
            math.insert("round".into(), Value::Native(natives::math_round));
            math.insert("random".into(), Value::Native(natives::math_random));
            math.insert("pow".into(), Value::Native(natives::math_pow));
            math.insert("sin".into(), Value::Native(natives::math_sin));
            math.insert("cos".into(), Value::Native(natives::math_cos));
            math.insert("tan".into(), Value::Native(natives::math_tan));
            math.insert("log".into(), Value::Native(natives::math_log));
            math.insert("exp".into(), Value::Native(natives::math_exp));
            math.insert("sign".into(), Value::Native(natives::math_sign));
            math.insert("trunc".into(), Value::Native(natives::math_trunc));
            math.insert("PI".into(), Value::Number(std::f64::consts::PI));
            math.insert("E".into(), Value::Number(std::f64::consts::E));
            s.set("Math".into(), Value::Object(Rc::new(RefCell::new(math))), true);

            let mut json = HashMap::with_capacity(2);
            json.insert("parse".into(), Value::Native(Self::json_parse_native));
            json.insert("stringify".into(), Value::Native(Self::json_stringify_native));
            s.set("JSON".into(), Value::Object(Rc::new(RefCell::new(json))), true);

            let mut object = HashMap::with_capacity(5);
            object.insert("keys".into(), Value::Native(Self::object_keys));
            object.insert("values".into(), Value::Native(Self::object_values));
            object.insert("entries".into(), Value::Native(Self::object_entries));
            object.insert("assign".into(), Value::Native(Self::object_assign));
            object.insert("fromEntries".into(), Value::Native(Self::object_from_entries));
            s.set("Object".into(), Value::Object(Rc::new(RefCell::new(object))), true);

            let mut array_obj = HashMap::with_capacity(1);
            array_obj.insert("isArray".into(), Value::Native(natives::array_is_array));
            s.set("Array".into(), Value::Object(Rc::new(RefCell::new(array_obj))), true);

            let mut string_obj = HashMap::with_capacity(1);
            string_obj.insert("fromCharCode".into(), Value::Native(natives::string_from_char_code));
            s.set("String".into(), Value::Object(Rc::new(RefCell::new(string_obj))), true);

            let mut date = HashMap::with_capacity(1);
            date.insert("now".into(), Value::Native(natives::date_now));
            s.set("Date".into(), Value::Object(Rc::new(RefCell::new(date))), true);

            #[cfg(feature = "regex")]
            {
                s.set("RegExp".into(), Value::Native(Self::regexp_constructor_native), true);
            }

            #[cfg(feature = "process")]
            {
                let mut process = HashMap::with_capacity(4);
                process.insert("exit".into(), Value::Native(natives::process_exit));
                process.insert("cwd".into(), Value::Native(natives::process_cwd));
                let argv: Vec<Value> = std::env::args()
                    .map(|s| Value::String(s.into()))
                    .collect();
                process.insert("argv".into(), Value::Array(Rc::new(RefCell::new(argv))));
                let env_obj: HashMap<Arc<str>, Value> = std::env::vars()
                    .map(|(key, value)| (Arc::from(key.as_str()), Value::String(value.into())))
                    .collect();
                process.insert("env".into(), Value::Object(Rc::new(RefCell::new(env_obj))));
                s.set("process".into(), Value::Object(Rc::new(RefCell::new(process))), true);
            }

            #[cfg(feature = "http")]
            {
                s.set("fetch".into(), Value::Native(Self::fetch_native), true);
                s.set("fetchAll".into(), Value::Native(Self::fetch_all_native), true);
                s.set("serve".into(), Value::Serve, true);
            }

            #[cfg(feature = "fs")]
            {
                s.set("readFile".into(), Value::Native(natives::read_file), true);
                s.set("writeFile".into(), Value::Native(natives::write_file), true);
                s.set("fileExists".into(), Value::Native(natives::file_exists), true);
                s.set("readDir".into(), Value::Native(natives::read_dir), true);
                s.set("mkdir".into(), Value::Native(natives::mkdir), true);
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
            Statement::VarDeclDestructure { pattern, mutable, init, .. } => {
                let value = self.eval_expr(init)?;
                self.bind_destruct_pattern(pattern, &value, *mutable)?;
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
                // Extract parameter names and defaults using Arc for cheap cloning
                let param_names: Arc<[Arc<str>]> = params.iter().map(|p| Arc::clone(&p.name)).collect();
                let defaults: Arc<[Option<Expr>]> = params.iter().map(|p| p.default.clone()).collect();
                let rest_param_name = rest_param.as_ref().map(|p| Arc::clone(&p.name));
                let body = Arc::new(body.as_ref().clone());
                let func = Value::Function {
                    params: param_names,
                    defaults,
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
                finally_body,
                ..
            } => {
                let try_result = self.eval_statement(body);
                
                let result = match try_result {
                    Ok(v) => Ok(v),
                    Err(EvalError::Throw(thrown)) => {
                        if let Some(catch_stmt) = catch_body {
                            if let Some(param) = catch_param {
                                let scope = Scope::child(Rc::clone(&self.scope));
                                let prev = std::mem::replace(&mut self.scope, Rc::clone(&scope));
                                scope.borrow_mut().set(Arc::clone(param), thrown, true);
                                let res = self.eval_statement(catch_stmt);
                                self.scope = prev;
                                res
                            } else {
                                self.eval_statement(catch_stmt)
                            }
                        } else {
                            Err(EvalError::Throw(thrown))
                        }
                    }
                    Err(e) => Err(e),
                };
                
                if let Some(finally_stmt) = finally_body {
                    let _ = self.eval_statement(finally_stmt);
                }
                
                result
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
                    let arg_vals = self.eval_call_args(args)?;
                    
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
                                let search = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for (i, v) in arr_borrow.iter().enumerate() {
                                    if v.strict_eq(&search) {
                                        return Ok(Value::Number(i as f64));
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "includes" => {
                                let search = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                for v in arr_borrow.iter() {
                                    if v.strict_eq(&search) {
                                        return Ok(Value::Bool(true));
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "join" => {
                                let sep = match arg_vals.first() {
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
                            "sort" => {
                                let comparator = arg_vals.into_iter().next();
                                let mut arr_mut = arr.borrow_mut();

                                if let Some(cmp_fn) = comparator {
                                    // Check for fast path: (a, b) => a - b numeric ascending
                                    let is_numeric_asc = Self::is_numeric_sort_comparator(&cmp_fn, false);
                                    let is_numeric_desc = !is_numeric_asc && Self::is_numeric_sort_comparator(&cmp_fn, true);

                                    if is_numeric_asc {
                                        // Fast path: numeric ascending sort
                                        arr_mut.sort_by(|a, b| {
                                            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
                                            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
                                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                                        });
                                    } else if is_numeric_desc {
                                        // Fast path: numeric descending sort
                                        arr_mut.sort_by(|a, b| {
                                            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
                                            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
                                            nb.partial_cmp(&na).unwrap_or(std::cmp::Ordering::Equal)
                                        });
                                    } else {
                                        // General case: use comparator function with optimized scope reuse
                                        let len = arr_mut.len();
                                        let mut indices: Vec<usize> = (0..len).collect();
                                        let arr_values: Vec<Value> = std::mem::take(&mut *arr_mut);

                                        if let Some((scope, params, body)) = self.create_callback_scope(&cmp_fn) {
                                            indices.sort_by(|&i, &j| {
                                                let result = self.call_with_scope(&scope, &params, &body, &[arr_values[i].clone(), arr_values[j].clone()]);
                                                match result {
                                                    Ok(Value::Number(n)) if n < 0.0 => std::cmp::Ordering::Less,
                                                    Ok(Value::Number(n)) if n > 0.0 => std::cmp::Ordering::Greater,
                                                    _ => std::cmp::Ordering::Equal,
                                                }
                                            });
                                        } else {
                                            indices.sort_by(|&i, &j| {
                                                let result = self.call_func(&cmp_fn, &[arr_values[i].clone(), arr_values[j].clone()]);
                                                match result {
                                                    Ok(Value::Number(n)) if n < 0.0 => std::cmp::Ordering::Less,
                                                    Ok(Value::Number(n)) if n > 0.0 => std::cmp::Ordering::Greater,
                                                    _ => std::cmp::Ordering::Equal,
                                                }
                                            });
                                        }

                                        *arr_mut = indices.into_iter().map(|i| arr_values[i].clone()).collect();
                                    }
                                } else {
                                    // Default string sort - precompute strings once
                                    let mut pairs: Vec<(String, usize)> = arr_mut
                                        .iter()
                                        .enumerate()
                                        .map(|(i, v)| (v.to_string(), i))
                                        .collect();
                                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                                    let arr_values: Vec<Value> = std::mem::take(&mut *arr_mut);
                                    *arr_mut = pairs.into_iter().map(|(_, i)| arr_values[i].clone()).collect();
                                }
                                drop(arr_mut);
                                return Ok(obj.clone());
                            }
                            "splice" => {
                                let mut arr_mut = arr.borrow_mut();
                                let len = arr_mut.len() as i64;
                                
                                let start = match arg_vals.first() {
                                    Some(Value::Number(n)) => {
                                        let n = *n as i64;
                                        if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                                    }
                                    _ => 0,
                                };
                                
                                let delete_count = match arg_vals.get(1) {
                                    Some(Value::Number(n)) => (*n as i64).max(0) as usize,
                                    _ => (len as usize).saturating_sub(start),
                                };
                                
                                let actual_delete = delete_count.min(arr_mut.len().saturating_sub(start));
                                let removed: Vec<Value> = arr_mut.drain(start..start + actual_delete).collect();
                                
                                if arg_vals.len() > 2 {
                                    let items_to_insert: Vec<Value> = arg_vals[2..].to_vec();
                                    for (i, item) in items_to_insert.into_iter().enumerate() {
                                        arr_mut.insert(start + i, item);
                                    }
                                }
                                
                                return Ok(Value::Array(Rc::new(RefCell::new(removed))));
                            }
                            "slice" => {
                                let arr_borrow = arr.borrow();
                                let len = arr_borrow.len() as i64;
                                let start = match arg_vals.first() {
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
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let mut result = Vec::with_capacity(arr_borrow.len());
                                // Try fastest path: simple single-expression callbacks
                                let first_result = self.eval_simple_callback(&callback, &[arr_borrow.first().cloned().unwrap_or(Value::Null)]);
                                if first_result.is_some() {
                                    // Simple callback path - inline evaluation
                                    for v in arr_borrow.iter() {
                                        if let Some(r) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            result.push(r?);
                                        } else {
                                            // Shouldn't happen, but fall back
                                            result.push(self.call_func(&callback, &[v.clone()])?);
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    // Reusable scope path
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        result.push(mapped);
                                    }
                                } else {
                                    // Full call_func path
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let mapped = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        result.push(mapped);
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "filter" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let mut result = Vec::new();
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(keep) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if keep?.is_truthy() {
                                                result.push(v.clone());
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let keep = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        if keep.is_truthy() {
                                            result.push(v.clone());
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let keep = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        if keep.is_truthy() {
                                            result.push(v.clone());
                                        }
                                    }
                                }
                                return Ok(Value::Array(Rc::new(RefCell::new(result))));
                            }
                            "reduce" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                let (mut acc, start_idx) = if arg_vals.len() > 1 {
                                    (arg_vals[1].clone(), 0)
                                } else if !arr_borrow.is_empty() {
                                    (arr_borrow[0].clone(), 1)
                                } else {
                                    return Err(EvalError::Error("Reduce of empty array with no initial value".to_string()));
                                };
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate().skip(start_idx) {
                                        acc = self.call_with_scope(&scope, &params, &body, &[acc, v.clone(), Value::Number(i as f64)])?;
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate().skip(start_idx) {
                                        acc = self.call_func(&callback, &[acc, v.clone(), Value::Number(i as f64)])?;
                                    }
                                }
                                return Ok(acc);
                            }
                            "find" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(found) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if found?.is_truthy() {
                                                return Ok(v.clone());
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        if found.is_truthy() {
                                            return Ok(v.clone());
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        if found.is_truthy() {
                                            return Ok(v.clone());
                                        }
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "findIndex" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        if found.is_truthy() {
                                            return Ok(Value::Number(i as f64));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let found = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        if found.is_truthy() {
                                            return Ok(Value::Number(i as f64));
                                        }
                                    }
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            "forEach" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                    }
                                }
                                return Ok(Value::Null);
                            }
                            "some" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(result) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if result?.is_truthy() {
                                                return Ok(Value::Bool(true));
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        if result.is_truthy() {
                                            return Ok(Value::Bool(true));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        if result.is_truthy() {
                                            return Ok(Value::Bool(true));
                                        }
                                    }
                                }
                                return Ok(Value::Bool(false));
                            }
                            "every" => {
                                let callback = arg_vals.first().cloned().unwrap_or(Value::Null);
                                let arr_borrow = arr.borrow();
                                // Try simple callback fast path
                                let use_simple = arr_borrow.first().map(|v| {
                                    self.eval_simple_callback(&callback, &[v.clone()]).is_some()
                                }).unwrap_or(false);
                                if use_simple {
                                    for v in arr_borrow.iter() {
                                        if let Some(result) = self.eval_simple_callback(&callback, &[v.clone()]) {
                                            if !result?.is_truthy() {
                                                return Ok(Value::Bool(false));
                                            }
                                        }
                                    }
                                } else if let Some((scope, params, body)) = self.create_callback_scope(&callback) {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_with_scope(&scope, &params, &body, &[v.clone(), Value::Number(i as f64)])?;
                                        if !result.is_truthy() {
                                            return Ok(Value::Bool(false));
                                        }
                                    }
                                } else {
                                    for (i, v) in arr_borrow.iter().enumerate() {
                                        let result = self.call_func(&callback, &[v.clone(), Value::Number(i as f64)])?;
                                        if !result.is_truthy() {
                                            return Ok(Value::Bool(false));
                                        }
                                    }
                                }
                                return Ok(Value::Bool(true));
                            }
                            "flat" => {
                                let depth = match arg_vals.first() {
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
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Number(-1.0)),
                                };
                                return Ok(Value::Number(
                                    s.find(search).map(|i| i as f64).unwrap_or(-1.0)
                                ));
                            }
                            "includes" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.contains(search)));
                            }
                            "slice" => {
                                let chars: Vec<char> = s.chars().collect();
                                let len = chars.len() as i64;
                                let start = match arg_vals.first() {
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
                                let start = match arg_vals.first() {
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
                                #[cfg(feature = "regex")]
                                if let Some(sep) = arg_vals.first() {
                                    let limit = arg_vals.get(1).and_then(|v| match v {
                                        Value::Number(n) => Some(*n as usize),
                                        _ => None,
                                    });
                                    return Ok(crate::regex::string_split(s, sep, limit));
                                }
                                #[cfg(not(feature = "regex"))]
                                {
                                    let sep = match arg_vals.first() {
                                        Some(Value::String(ss)) => ss.as_ref(),
                                        _ => return Ok(Value::Array(Rc::new(RefCell::new(vec![obj.clone()])))),
                                    };
                                    let parts: Vec<Value> = s.split(sep)
                                        .map(|p| Value::String(p.into()))
                                        .collect();
                                    return Ok(Value::Array(Rc::new(RefCell::new(parts))));
                                }
                                #[cfg(feature = "regex")]
                                return Ok(Value::Array(Rc::new(RefCell::new(vec![obj.clone()]))));
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
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.starts_with(search)));
                            }
                            "endsWith" => {
                                let search = match arg_vals.first() {
                                    Some(Value::String(ss)) => ss.as_ref(),
                                    _ => return Ok(Value::Bool(false)),
                                };
                                return Ok(Value::Bool(s.ends_with(search)));
                            }
                            "replace" => {
                                #[cfg(feature = "regex")]
                                if let (Some(search), Some(replace)) = (arg_vals.first(), arg_vals.get(1)) {
                                    return Ok(crate::regex::string_replace(s, search, replace));
                                }
                                #[cfg(not(feature = "regex"))]
                                {
                                    let search = match arg_vals.first() {
                                        Some(Value::String(ss)) => ss.to_string(),
                                        _ => return Ok(obj.clone()),
                                    };
                                    let replacement = match arg_vals.get(1) {
                                        Some(Value::String(ss)) => ss.to_string(),
                                        _ => String::new(),
                                    };
                                    return Ok(Value::String(s.replacen(&search, &replacement, 1).into()));
                                }
                                #[cfg(feature = "regex")]
                                return Ok(obj.clone());
                            }
                            "replaceAll" => {
                                let search = match arg_vals.first() {
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
                                let idx = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                let chars: Vec<char> = s.chars().collect();
                                return Ok(chars.get(idx)
                                    .map(|c| Value::String(c.to_string().into()))
                                    .unwrap_or(Value::String("".into())));
                            }
                            "charCodeAt" => {
                                let idx = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => 0,
                                };
                                let chars: Vec<char> = s.chars().collect();
                                return Ok(chars.get(idx)
                                    .map(|c| Value::Number(*c as u32 as f64))
                                    .unwrap_or(Value::Number(f64::NAN)));
                            }
                            "repeat" => {
                                let count = match arg_vals.first() {
                                    Some(Value::Number(n)) if *n >= 0.0 => *n as usize,
                                    _ => 0,
                                };
                                return Ok(Value::String(s.repeat(count).into()));
                            }
                            "padStart" => {
                                let target_len = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let char_count = s.chars().count();
                                if char_count >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - char_count;
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", padding, s).into()));
                            }
                            "padEnd" => {
                                let target_len = match arg_vals.first() {
                                    Some(Value::Number(n)) => *n as usize,
                                    _ => return Ok(obj.clone()),
                                };
                                let pad = match arg_vals.get(1) {
                                    Some(Value::String(p)) => p.to_string(),
                                    _ => " ".to_string(),
                                };
                                let char_count = s.chars().count();
                                if char_count >= target_len || pad.is_empty() {
                                    return Ok(obj.clone());
                                }
                                let needed = target_len - char_count;
                                let padding: String = pad.chars().cycle().take(needed).collect();
                                return Ok(Value::String(format!("{}{}", s, padding).into()));
                            }
                            #[cfg(feature = "regex")]
                            "match" => {
                                if let Some(regexp) = arg_vals.first() {
                                    return Ok(crate::regex::string_match(s, regexp));
                                }
                                return Ok(Value::Null);
                            }
                            #[cfg(feature = "regex")]
                            "search" => {
                                if let Some(regexp) = arg_vals.first() {
                                    return Ok(crate::regex::string_search(s, regexp));
                                }
                                return Ok(Value::Number(-1.0));
                            }
                            _ => {}
                        }
                    }

                    // RegExp methods
                    #[cfg(feature = "regex")]
                    if let Value::RegExp(re) = &obj {
                        match method_name.as_ref() {
                            "test" => {
                                let input = arg_vals.first()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let result = re.borrow_mut().test(&input);
                                return Ok(Value::Bool(result));
                            }
                            "exec" => {
                                let input = arg_vals.first()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let result = crate::regex::regexp_exec(&mut re.borrow_mut(), &input);
                                return Ok(result);
                            }
                            _ => {}
                        }
                    }
                    
                    // Fall through to normal function call
                    let f = self.get_prop(&obj, method_name).map_err(EvalError::Error)?;
                    return self.call_func(&f, &arg_vals);
                }
                
                let f = self.eval_expr(callee)?;
                let arg_vals = self.eval_call_args(args)?;
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
                let mut vals = Vec::with_capacity(elements.len());
                for elem in elements {
                    match elem {
                        tish_ast::ArrayElement::Expr(e) => {
                            vals.push(self.eval_expr(e)?);
                        }
                        tish_ast::ArrayElement::Spread(e) => {
                            let spread_val = self.eval_expr(e)?;
                            if let Value::Array(arr) = spread_val {
                                vals.extend(arr.borrow().iter().cloned());
                            }
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(vals))))
            }
            Expr::Object { props, .. } => {
                let mut map = HashMap::new();
                for prop in props {
                    match prop {
                        tish_ast::ObjectProp::KeyValue(k, v) => {
                            map.insert(Arc::clone(k), self.eval_expr(v)?);
                        }
                        tish_ast::ObjectProp::Spread(e) => {
                            let spread_val = self.eval_expr(e)?;
                            if let Value::Object(obj) = spread_val {
                                for (k, v) in obj.borrow().iter() {
                                    map.insert(Arc::clone(k), v.clone());
                                }
                            }
                        }
                    }
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
                    Value::Function { .. } | Value::Native(_) => "function".into(),
                    #[cfg(feature = "http")]
                    Value::Serve => "function".into(),
                    #[cfg(feature = "regex")]
                    Value::RegExp(_) => "object".into(),
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
                // Convert arrow function to regular function using Arc for cheap cloning
                let param_names: Arc<[Arc<str>]> = params.iter().map(|p| Arc::clone(&p.name)).collect();
                let defaults: Arc<[Option<tish_ast::Expr>]> = params.iter().map(|p| p.default.clone()).collect();
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
                    defaults,
                    rest_param: None,
                    body: Arc::new(body_stmt),
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
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(a);
                    s.push_str(b);
                    Ok(Value::String(s.into()))
                }
                (Value::String(a), b) => {
                    let b_str = b.to_string();
                    let mut s = String::with_capacity(a.len() + b_str.len());
                    s.push_str(a);
                    s.push_str(&b_str);
                    Ok(Value::String(s.into()))
                }
                (a, Value::String(b)) => {
                    let a_str = a.to_string();
                    let mut s = String::with_capacity(a_str.len() + b.len());
                    s.push_str(&a_str);
                    s.push_str(b);
                    Ok(Value::String(s.into()))
                }
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

    /// Check if a function value is the common numeric sort comparator pattern.
    /// descending = false: checks for `(a, b) => a - b`
    /// descending = true: checks for `(a, b) => b - a`
    fn is_numeric_sort_comparator(f: &Value, descending: bool) -> bool {
        if let Value::Function { params, body, defaults, rest_param } = f {
            // Must have exactly 2 params, no defaults, no rest
            if params.len() != 2 || rest_param.is_some() {
                return false;
            }
            if defaults.iter().any(|d| d.is_some()) {
                return false;
            }

            // Body must be a return of a - b (or b - a for descending)
            let param_a = &params[0];
            let param_b = &params[1];

            // Check for both Statement::Return and Statement::ExprStmt (arrow implicit return)
            let expr = match body.as_ref() {
                Statement::Return { value: Some(e), .. } => e,
                Statement::ExprStmt { expr: e, .. } => e,
                _ => return false,
            };

            // Check for binary subtraction
            if let Expr::Binary { left, op: BinOp::Sub, right, .. } = expr {
                // Check left is Ident(a) and right is Ident(b)
                let (expected_left, expected_right) = if descending {
                    (param_b, param_a)  // b - a
                } else {
                    (param_a, param_b)  // a - b
                };

                if let (Expr::Ident { name: left_name, .. }, Expr::Ident { name: right_name, .. }) = (left.as_ref(), right.as_ref()) {
                    return left_name == expected_left && right_name == expected_right;
                }
            }
        }
        false
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

    /// Optimized callback invocation for array methods.
    /// Creates a reusable scope that can be updated for each iteration.
    fn create_callback_scope(&self, f: &Value) -> Option<(Rc<RefCell<Scope>>, Arc<[Arc<str>]>, Arc<Statement>)> {
        if let Value::Function { params, body, defaults, rest_param } = f {
            // Only optimize simple cases: no defaults used, no rest params
            if rest_param.is_some() || defaults.iter().any(|d| d.is_some()) {
                return None;
            }
            let scope = Scope::child(Rc::clone(&self.scope));
            // Pre-initialize parameters to Null
            {
                let mut s = scope.borrow_mut();
                for p in params.iter() {
                    s.set(Arc::clone(p), Value::Null, true);
                }
            }
            return Some((scope, Arc::clone(params), Arc::clone(body)));
        }
        None
    }

    /// Fast callback invocation that reuses an existing scope.
    fn call_with_scope(
        &self,
        scope: &Rc<RefCell<Scope>>,
        params: &[Arc<str>],
        body: &Statement,
        args: &[Value],
    ) -> Result<Value, EvalError> {
        {
            let mut s = scope.borrow_mut();
            for (i, p) in params.iter().enumerate() {
                let val = args.get(i).cloned().unwrap_or(Value::Null);
                // Direct assignment - we know these vars exist and are mutable
                if let Some(existing) = s.vars.get_mut(p.as_ref()) {
                    *existing = val;
                }
            }
        }
        let mut eval = Evaluator { scope: Rc::clone(scope) };
        match eval.eval_statement(body) {
            Ok(v) => Ok(v),
            Err(EvalError::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Try to evaluate a simple callback expression directly without creating a scope.
    /// Returns Some(result) for simple patterns like `x => x * 2` or `x => x > 5`.
    fn eval_simple_callback(
        &self,
        f: &Value,
        args: &[Value],
    ) -> Option<Result<Value, EvalError>> {
        if let Value::Function { params, body, defaults, rest_param } = f {
            // Only optimize single-parameter functions without defaults or rest
            if params.len() != 1 || rest_param.is_some() || defaults.iter().any(|d| d.is_some()) {
                return None;
            }
            let param_name = &params[0];
            let arg = args.first().cloned().unwrap_or(Value::Null);

            // Get the expression from the body
            let expr = match body.as_ref() {
                Statement::Return { value: Some(e), .. } => e,
                Statement::ExprStmt { expr: e, .. } => e,
                _ => return None,
            };

            // Fast path for common patterns
            match expr {
                // x * constant or x + constant, etc.
                Expr::Binary { left, op, right, .. } => {
                    let left_val = self.eval_simple_operand(left, param_name, &arg)?;
                    let right_val = self.eval_simple_operand(right, param_name, &arg)?;
                    Some(self.eval_binop(&left_val, *op, &right_val).map_err(EvalError::Error))
                }
                // Just return the parameter
                Expr::Ident { name, .. } if name == param_name => {
                    Some(Ok(arg))
                }
                // Property access: x.prop
                Expr::Member { object, prop, optional, .. } => {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if name == param_name {
                            return self.eval_simple_member(&arg, prop, *optional);
                        }
                    }
                    None
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Evaluate a simple operand (identifier or literal).
    fn eval_simple_operand(&self, expr: &Expr, param_name: &Arc<str>, param_val: &Value) -> Option<Value> {
        match expr {
            Expr::Ident { name, .. } if name == param_name => Some(param_val.clone()),
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => Some(Value::Number(*n)),
                Literal::String(s) => Some(Value::String(Arc::clone(s))),
                Literal::Bool(b) => Some(Value::Bool(*b)),
                Literal::Null => Some(Value::Null),
            },
            _ => None,
        }
    }

    /// Evaluate simple member access.
    fn eval_simple_member(&self, obj: &Value, property: &MemberProp, _optional: bool) -> Option<Result<Value, EvalError>> {
        match property {
            MemberProp::Name(name) => {
                match obj {
                    Value::Object(o) => {
                        let result = o.borrow().get(name.as_ref()).cloned().unwrap_or(Value::Null);
                        Some(Ok(result))
                    }
                    Value::Array(arr) if name.as_ref() == "length" => {
                        Some(Ok(Value::Number(arr.borrow().len() as f64)))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn call_func(&self, f: &Value, args: &[Value]) -> Result<Value, EvalError> {
        match f {
            Value::Native(native_fn) => {
                native_fn(args).map_err(EvalError::Error)
            }
            #[cfg(feature = "http")]
            Value::Serve => self.run_http_server(args),
            #[cfg(feature = "regex")]
            Value::RegExp(_) => Err(EvalError::Error("RegExp is not callable".to_string())),
            Value::Function { params, defaults, rest_param, body } => {
                let scope = Scope::child(Rc::clone(&self.scope));
                {
                    let mut s = scope.borrow_mut();
                    for (i, p) in params.iter().enumerate() {
                        let val = match args.get(i) {
                            Some(v) => v.clone(),
                            None => {
                                if let Some(Some(default_expr)) = defaults.get(i) {
                                    drop(s);
                                    let default_val = self.eval_expr(default_expr)?;
                                    s = scope.borrow_mut();
                                    default_val
                                } else {
                                    Value::Null
                                }
                            }
                        };
                        s.set(Arc::clone(p), val, true);
                    }
                    if let Some(ref rest_name) = rest_param {
                        let rest_vals: Vec<Value> = args.iter().skip(params.len()).cloned().collect();
                        s.set(Arc::clone(rest_name), Value::Array(Rc::new(RefCell::new(rest_vals))), true);
                    }
                }
                let mut eval = Evaluator { scope };
                match eval.eval_statement(body) {
                    Ok(v) => Ok(v),
                    Err(EvalError::Return(v)) => Ok(v),
                    Err(EvalError::Throw(v)) => Err(EvalError::Throw(v)),
                    Err(EvalError::Error(s)) => Err(EvalError::Error(s)),
                    Err(EvalError::Break) => Err(EvalError::Error("break outside loop".to_string())),
                    Err(EvalError::Continue) => Err(EvalError::Error("continue outside loop".to_string())),
                }
            }
            _ => Err(EvalError::Error("Not a function".to_string())),
        }
    }

    #[cfg(feature = "http")]
    fn run_http_server(&self, args: &[Value]) -> Result<Value, EvalError> {
        let port = match args.first() {
            Some(Value::Number(n)) => *n as u16,
            _ => return Err(EvalError::Error("serve requires a port number".to_string())),
        };

        let handler = match args.get(1) {
            Some(f @ Value::Function { .. }) | Some(f @ Value::Native(_)) => f.clone(),
            _ => {
                return Err(EvalError::Error(
                    "serve requires a handler function".to_string(),
                ))
            }
        };

        let server = crate::http::create_server(port).map_err(EvalError::Error)?;
        println!("Server listening on http://0.0.0.0:{}", port);

        for mut request in server.incoming_requests() {
            let req_value = crate::http::request_to_value(&mut request);

            let response_value = match self.call_func(&handler, &[req_value]) {
                Ok(v) => v,
                Err(EvalError::Throw(v)) => {
                    let mut err_obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(2);
                    err_obj.insert(Arc::from("status"), Value::Number(500.0));
                    err_obj.insert(Arc::from("body"), Value::String(v.to_string().into()));
                    Value::Object(Rc::new(RefCell::new(err_obj)))
                }
                Err(e) => {
                    let mut err_obj: HashMap<Arc<str>, Value> = HashMap::with_capacity(2);
                    err_obj.insert(Arc::from("status"), Value::Number(500.0));
                    err_obj.insert(Arc::from("body"), Value::String(e.to_string().into()));
                    Value::Object(Rc::new(RefCell::new(err_obj)))
                }
            };

            let (status, headers, body) = crate::http::value_to_response(&response_value);
            crate::http::send_response(request, status, headers, body);
        }

        Ok(Value::Null)
    }

    fn eval_call_args(&self, args: &[tish_ast::CallArg]) -> Result<Vec<Value>, EvalError> {
        let mut result = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                tish_ast::CallArg::Expr(e) => {
                    result.push(self.eval_expr(e)?);
                }
                tish_ast::CallArg::Spread(e) => {
                    let spread_val = self.eval_expr(e)?;
                    if let Value::Array(arr) = spread_val {
                        result.extend(arr.borrow().iter().cloned());
                    }
                }
            }
        }
        Ok(result)
    }

    fn bind_destruct_pattern(&mut self, pattern: &tish_ast::DestructPattern, value: &Value, mutable: bool) -> Result<(), EvalError> {
        match pattern {
            tish_ast::DestructPattern::Array(elements) => {
                let arr = match value {
                    Value::Array(a) => a.borrow().clone(),
                    _ => return Err(EvalError::Error("Cannot destructure non-array value".to_string())),
                };
                
                for (i, elem) in elements.iter().enumerate() {
                    if let Some(el) = elem {
                        match el {
                            tish_ast::DestructElement::Ident(name) => {
                                let val = arr.get(i).cloned().unwrap_or(Value::Null);
                                self.scope.borrow_mut().set(Arc::clone(name), val, mutable);
                            }
                            tish_ast::DestructElement::Pattern(nested) => {
                                let val = arr.get(i).cloned().unwrap_or(Value::Null);
                                self.bind_destruct_pattern(nested, &val, mutable)?;
                            }
                            tish_ast::DestructElement::Rest(name) => {
                                let rest: Vec<Value> = arr.iter().skip(i).cloned().collect();
                                self.scope.borrow_mut().set(Arc::clone(name), Value::Array(Rc::new(RefCell::new(rest))), mutable);
                                break;
                            }
                        }
                    }
                }
            }
            tish_ast::DestructPattern::Object(props) => {
                let obj = match value {
                    Value::Object(o) => o.borrow().clone(),
                    _ => return Err(EvalError::Error("Cannot destructure non-object value".to_string())),
                };
                
                for prop in props {
                    let val = obj.get(&prop.key).cloned().unwrap_or(Value::Null);
                    match &prop.value {
                        tish_ast::DestructElement::Ident(name) => {
                            self.scope.borrow_mut().set(Arc::clone(name), val, mutable);
                        }
                        tish_ast::DestructElement::Pattern(nested) => {
                            self.bind_destruct_pattern(nested, &val, mutable)?;
                        }
                        tish_ast::DestructElement::Rest(_) => {
                            return Err(EvalError::Error("Rest not supported in object destructuring".to_string()));
                        }
                    }
                }
            }
        }
        Ok(())
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
            #[cfg(feature = "regex")]
            Value::RegExp(re) => {
                let re = re.borrow();
                match key {
                    "source" => Ok(Value::String(re.source.clone().into())),
                    "flags" => Ok(Value::String(re.flags_string().into())),
                    "lastIndex" => Ok(Value::Number(re.last_index as f64)),
                    "global" => Ok(Value::Bool(re.flags.global)),
                    "ignoreCase" => Ok(Value::Bool(re.flags.ignore_case)),
                    "multiline" => Ok(Value::Bool(re.flags.multiline)),
                    "dotAll" => Ok(Value::Bool(re.flags.dot_all)),
                    "unicode" => Ok(Value::Bool(re.flags.unicode)),
                    "sticky" => Ok(Value::Bool(re.flags.sticky)),
                    _ => Ok(Value::Null),
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
            Value::Function { .. } | Value::Native(_) => "null".to_string(),
            #[cfg(feature = "http")]
            Value::Serve => "null".to_string(),
            #[cfg(feature = "regex")]
            Value::RegExp(_) => "null".to_string(),
        }
    }

    // Static native wrapper functions (these need to be fn pointers, not closures with &self)
    fn json_parse_native(args: &[Value]) -> Result<Value, String> {
        let s = args.first().map(|v| v.to_string()).unwrap_or_default();
        Ok(Self::json_parse(&s))
    }

    fn json_stringify_native(args: &[Value]) -> Result<Value, String> {
        let v = args.first().cloned().unwrap_or(Value::Null);
        Ok(Value::String(Self::json_stringify_value(&v).into()))
    }

    fn object_keys(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let keys: Vec<Value> = obj.borrow().keys().map(|k| Value::String(Arc::clone(k))).collect();
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_values(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let values: Vec<Value> = obj.borrow().values().cloned().collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_entries(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(obj)) = args.first() {
            let entries: Vec<Value> = obj.borrow().iter().map(|(k, v)| {
                Value::Array(Rc::new(RefCell::new(vec![
                    Value::String(Arc::clone(k)),
                    v.clone(),
                ])))
            }).collect();
            Ok(Value::Array(Rc::new(RefCell::new(entries))))
        } else {
            Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
        }
    }

    fn object_assign(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Object(target)) = args.first() {
            let mut t = target.borrow_mut();
            for src in args.iter().skip(1) {
                if let Value::Object(src_obj) = src {
                    for (k, v) in src_obj.borrow().iter() {
                        t.insert(Arc::clone(k), v.clone());
                    }
                }
            }
            drop(t);
            Ok(args.first().cloned().unwrap())
        } else {
            Ok(Value::Null)
        }
    }

    fn object_from_entries(args: &[Value]) -> Result<Value, String> {
        if let Some(Value::Array(arr)) = args.first() {
            let mut map = HashMap::new();
            for entry in arr.borrow().iter() {
                if let Value::Array(pair) = entry {
                    let pair = pair.borrow();
                    if let (Some(key), Some(value)) = (pair.first(), pair.get(1)) {
                        let key_str: Arc<str> = key.to_string().into();
                        map.insert(key_str, value.clone());
                    }
                }
            }
            Ok(Value::Object(Rc::new(RefCell::new(map))))
        } else {
            Ok(Value::Object(Rc::new(RefCell::new(HashMap::new()))))
        }
    }

    #[cfg(feature = "regex")]
    fn regexp_constructor_native(args: &[Value]) -> Result<Value, String> {
        crate::regex::regexp_constructor(args)
    }

    #[cfg(feature = "http")]
    fn fetch_native(args: &[Value]) -> Result<Value, String> {
        crate::http::fetch(args)
    }

    #[cfg(feature = "http")]
    fn fetch_all_native(args: &[Value]) -> Result<Value, String> {
        crate::http::fetch_all(args)
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
