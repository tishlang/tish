//! Code generation: AST -> Rust source.

use tish_ast::{BinOp, CompoundOp, Expr, Literal, MemberProp, Program, Statement, UnaryOp};

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CompileError {}

pub fn compile(program: &Program) -> Result<String, CompileError> {
    let mut g = Codegen::new();
    g.emit_program(program)?;
    Ok(g.output)
}

struct Codegen {
    output: String,
    indent: usize,
    loop_label_index: usize,
    loop_stack: Vec<(String, Option<String>)>, // (break_label, continue_update) for innermost loop
}

impl Codegen {
    fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            loop_label_index: 0,
            loop_stack: Vec::new(),
        }
    }

    fn writeln(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn write(&mut self, s: &str) {
        self.output.push_str(s);
    }

    fn emit_program(&mut self, program: &Program) -> Result<(), CompileError> {
        self.write("use std::cell::RefCell;\n");
        self.write("use std::collections::HashMap;\n");
        self.write("use std::rc::Rc;\n");
        self.write("use std::sync::Arc;\n");
        self.write("use tish_runtime::{console_debug as tish_console_debug, console_info as tish_console_info, console_log as tish_console_log, console_warn as tish_console_warn, console_error as tish_console_error, decode_uri as tish_decode_uri, encode_uri as tish_encode_uri, in_operator as tish_in_operator, is_finite as tish_is_finite, is_nan as tish_is_nan, json_parse as tish_json_parse, json_stringify as tish_json_stringify, math_abs as tish_math_abs, math_ceil as tish_math_ceil, math_floor as tish_math_floor, math_max as tish_math_max, math_min as tish_math_min, math_round as tish_math_round, math_sqrt as tish_math_sqrt, parse_float as tish_parse_float, parse_int as tish_parse_int, TishError, Value};\n");
        #[cfg(feature = "http")]
        self.write("use tish_runtime::{http_fetch as tish_http_fetch, http_fetch_all as tish_http_fetch_all};\n");
        self.write("\n");

        self.writeln("fn main() {");
        self.indent += 1;
        self.writeln("if let Err(e) = run() {");
        self.indent += 1;
        self.writeln("eprintln!(\"Error: {}\", e);");
        self.writeln("std::process::exit(1);");
        self.indent -= 1;
        self.writeln("}");
        self.indent -= 1;
        self.writeln("}");
        self.writeln("");
        self.writeln("fn run() -> Result<(), Box<dyn std::error::Error>> {");
        self.indent += 1;

        // Initialize builtins
        self.writeln("let mut console = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"debug\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_debug(args); Value::Null }))),");
        self.writeln("(Arc::from(\"info\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_info(args); Value::Null }))),");
        self.writeln("(Arc::from(\"log\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_log(args); Value::Null }))),");
        self.writeln("(Arc::from(\"warn\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_warn(args); Value::Null }))),");
        self.writeln("(Arc::from(\"error\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_error(args); Value::Null }))),");
        self.indent -= 1;
        self.writeln("]))));");
        self.writeln("let parseInt = Value::Function(Rc::new(|args: &[Value]| tish_parse_int(args)));");
        self.writeln("let parseFloat = Value::Function(Rc::new(|args: &[Value]| tish_parse_float(args)));");
        self.writeln("let decodeURI = Value::Function(Rc::new(|args: &[Value]| tish_decode_uri(args)));");
        self.writeln("let encodeURI = Value::Function(Rc::new(|args: &[Value]| tish_encode_uri(args)));");
        self.writeln("let isFinite = Value::Function(Rc::new(|args: &[Value]| tish_is_finite(args)));");
        self.writeln("let isNaN = Value::Function(Rc::new(|args: &[Value]| tish_is_nan(args)));");
        self.writeln("let Infinity = Value::Number(f64::INFINITY);");
        self.writeln("let NaN = Value::Number(f64::NAN);");
        self.writeln("let Math = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"abs\"), Value::Function(Rc::new(|args: &[Value]| tish_math_abs(args)))),");
        self.writeln("(Arc::from(\"sqrt\"), Value::Function(Rc::new(|args: &[Value]| tish_math_sqrt(args)))),");
        self.writeln("(Arc::from(\"min\"), Value::Function(Rc::new(|args: &[Value]| tish_math_min(args)))),");
        self.writeln("(Arc::from(\"max\"), Value::Function(Rc::new(|args: &[Value]| tish_math_max(args)))),");
        self.writeln("(Arc::from(\"floor\"), Value::Function(Rc::new(|args: &[Value]| tish_math_floor(args)))),");
        self.writeln("(Arc::from(\"ceil\"), Value::Function(Rc::new(|args: &[Value]| tish_math_ceil(args)))),");
        self.writeln("(Arc::from(\"round\"), Value::Function(Rc::new(|args: &[Value]| tish_math_round(args)))),");
        self.indent -= 1;
        self.writeln("]))));");
        self.writeln("let JSON = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"parse\"), Value::Function(Rc::new(|args: &[Value]| tish_json_parse(args)))),");
        self.writeln("(Arc::from(\"stringify\"), Value::Function(Rc::new(|args: &[Value]| tish_json_stringify(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        #[cfg(feature = "http")]
        {
            self.writeln("let fetch = Value::Function(Rc::new(|args: &[Value]| tish_http_fetch(args)));");
            self.writeln("let fetchAll = Value::Function(Rc::new(|args: &[Value]| tish_http_fetch_all(args)));");
        }

        for stmt in &program.statements {
            self.emit_statement(stmt)?;
        }

        self.writeln("Ok(())");
        self.indent -= 1;
        self.writeln("}");
        Ok(())
    }

    fn emit_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } => {
                self.writeln("{");
                self.indent += 1;
                for s in statements {
                    self.emit_statement(s)?;
                }
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::VarDecl { name, mutable, init, .. } => {
                let expr = init
                    .as_ref()
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_else(|| "Value::Null".to_string());
                let mutability = if *mutable { "let mut" } else { "let" };
                // Clone to ensure JS-like reference semantics where multiple variables can hold the same value
                self.writeln(&format!("{} {} = ({}).clone();", mutability, name.as_ref(), expr));
            }
            Statement::ExprStmt { expr, .. } => {
                let e = self.emit_expr(expr)?;
                self.writeln(&format!("{};", e));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_expr(cond)?;
                self.write(&format!("if {}.is_truthy() {{\n", c));
                self.indent += 1;
                self.emit_statement(then_branch)?;
                self.indent -= 1;
                if let Some(eb) = else_branch {
                    self.writeln("} else {");
                    self.indent += 1;
                    self.emit_statement(eb)?;
                    self.indent -= 1;
                }
                self.writeln("}");
            }
            Statement::While { cond, body, .. } => {
                let c = self.emit_expr(cond)?;
                let label = format!("'while_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.write(&format!("{}: while {}.is_truthy() {{\n", label, c));
                self.indent += 1;
                self.emit_statement(body)?;
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::ForOf { name, iterable, body, .. } => {
                let iter_expr = self.emit_expr(iterable)?;
                self.writeln(&format!("{{ let _fof = {};", iter_expr));
                self.indent += 1;
                self.writeln("match &_fof {");
                self.indent += 1;
                self.writeln("Value::Array(ref _arr) => {");
                self.indent += 1;
                self.writeln("for _v in _arr.borrow().iter() {");
                self.indent += 1;
                self.writeln(&format!("let {} = _v.clone();", name.as_ref()));
                self.emit_statement(body)?;
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
                self.writeln("Value::String(ref _s) => {");
                self.indent += 1;
                self.writeln("for _ch in _s.chars() {");
                self.indent += 1;
                self.writeln(&format!(
                    "let {} = Value::String(std::sync::Arc::from(_ch.to_string()));",
                    name.as_ref()
                ));
                self.emit_statement(body)?;
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
                self.writeln("_ => panic!(\"for-of requires array or string\"),");
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.writeln("{");
                self.indent += 1;
                if let Some(i) = init {
                    self.emit_statement(i)?;
                }
                let label = format!("'for_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                let cond_expr = cond
                    .as_ref()
                    .map(|c| format!("{}.is_truthy()", self.emit_expr(c).unwrap()))
                    .unwrap_or_else(|| "true".to_string());
                let update_code = update.as_ref().map(|u| {
                    let ue = self.emit_expr(u).unwrap();
                    format!("{};", ue)
                });
                self.loop_stack.push((label.clone(), update_code));
                self.write(&format!("{}: loop {{\n", label));
                self.indent += 1;
                self.writeln(&format!("if !{} {{ break; }}", cond_expr));
                self.emit_statement(body)?;
                if let Some(u) = update {
                    let ue = self.emit_expr(u)?;
                    self.writeln(&format!("{};", ue));
                }
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Return { value, .. } => {
                let v = value
                    .as_ref()
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_else(|| "Value::Null".to_string());
                self.writeln(&format!("return {};", v));
            }
            Statement::Break { .. } => {
                if let Some((label, _)) = self.loop_stack.last() {
                    self.writeln(&format!("break {};", label));
                } else {
                    self.writeln("break;");
                }
            }
            Statement::Continue { .. } => {
                let snippet = self.loop_stack.last().map(|(label, update)| {
                    (
                        label.clone(),
                        update.clone(),
                    )
                });
                if let Some((label, Some(update))) = snippet {
                    self.writeln(&update);
                    self.writeln(&format!("continue {};", label));
                } else if let Some((label, None)) = snippet {
                    self.writeln(&format!("continue {};", label));
                } else {
                    self.writeln("continue;");
                }
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                let e = self.emit_expr(expr)?;
                self.writeln(&format!("let _sv = {};", e));
                self.writeln("match () {");
                self.indent += 1;
                for (case_expr, body) in cases {
                    if let Some(ce) = case_expr {
                        let c = self.emit_expr(ce)?;
                        self.write(&format!("_ if _sv.strict_eq(&{}) => {{\n", c));
                    } else {
                        self.writeln("_ => {");
                    }
                    self.indent += 1;
                    for s in body {
                        self.emit_statement(s)?;
                    }
                    self.indent -= 1;
                    self.writeln("}");
                }
                if let Some(body) = default_body {
                    self.writeln("_ => {");
                    self.indent += 1;
                    for s in body {
                        self.emit_statement(s)?;
                    }
                    self.indent -= 1;
                    self.writeln("}");
                } else if !cases.is_empty() {
                    self.writeln("_ => {}");
                }
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::DoWhile { body, cond, .. } => {
                let c = self.emit_expr(cond)?;
                let label = format!("'dowhile_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.write(&format!("{}: loop {{\n", label));
                self.indent += 1;
                self.emit_statement(body)?;
                self.write(&format!("if !{}.is_truthy() {{ break; }}\n", c));
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Throw { value, .. } => {
                let v = self.emit_expr(value)?;
                self.writeln(&format!(
                    "return Err(Box::new(tish_runtime::TishError::Throw({})) as Box<dyn std::error::Error>);",
                    v
                ));
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                ..
            } => {
                self.writeln("let _try_result: Result<Value, Box<dyn std::error::Error>> = (|| {");
                self.indent += 1;
                self.emit_statement(body)?;
                self.writeln("Ok(Value::Null)");
                self.indent -= 1;
                self.writeln("})();");
                if let Some(param) = catch_param {
                    self.writeln("if let Err(e) = _try_result {");
                    self.indent += 1;
                    self.writeln("match e.downcast::<tish_runtime::TishError>() {");
                    self.indent += 1;
                    self.writeln("Ok(tish_err) => {");
                    self.indent += 1;
                    self.writeln("if let tish_runtime::TishError::Throw(v) = *tish_err {");
                    self.writeln(&format!("let {} = v.clone();", param.as_ref()));
                    self.emit_statement(catch_body)?;
                    self.writeln("} else { return Err(Box::new(tish_err)); }");
                    self.indent -= 1;
                    self.writeln("}");
                    self.writeln("Err(orig) => return Err(orig),");
                    self.indent -= 1;
                    self.writeln("}");
                    self.indent -= 1;
                } else {
                    self.writeln("if let Err(_e) = _try_result {");
                    self.indent += 1;
                    self.emit_statement(catch_body)?;
                    self.indent -= 1;
                }
                self.writeln("}");
            }
            Statement::FunDecl { name, params, rest_param, body, .. } => {
                self.writeln(&format!("let {} = {{", name.as_ref()));
                self.indent += 1;
                self.writeln("let console = console.clone();");
                self.writeln("let Math = Math.clone();");
                self.writeln("let JSON = JSON.clone();");
                self.writeln("Value::Function(Rc::new(move |args: &[Value]| {");
                self.indent += 1;
                // Extract just the parameter names (type annotations are parsed but not used in codegen yet)
                for (i, p) in params.iter().enumerate() {
                    self.writeln(&format!(
                        "let {} = args.get({}).cloned().unwrap_or(Value::Null);",
                        p.name.as_ref(),
                        i
                    ));
                }
                if let Some(rest) = rest_param {
                    self.writeln(&format!(
                        "let {} = Value::Array(std::rc::Rc::new(RefCell::new(args[{}..].to_vec())));",
                        rest.name.as_ref(),
                        params.len()
                    ));
                }
                self.emit_statement(body)?;
                self.writeln("Value::Null");
                self.indent -= 1;
                self.writeln("}))");
                self.indent -= 1;
                self.writeln("};");
            }
        }
        Ok(())
    }

    fn emit_expr(&mut self, expr: &Expr) -> Result<String, CompileError> {
        Ok(match expr {
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => format!("Value::Number({}_f64)", n),
                Literal::String(s) => format!("Value::String({:?}.into())", s.as_ref()),
                Literal::Bool(b) => format!("Value::Bool({})", b),
                Literal::Null => "Value::Null".to_string(),
            },
            Expr::Ident { name, .. } => name.to_string(),
            Expr::Binary { left, op, right, .. } => {
                let l = self.emit_expr(left)?;
                let r = self.emit_expr(right)?;
                self.emit_binop(&l, *op, &r)?
            }
            Expr::Unary { op, operand, .. } => {
                let o = self.emit_expr(operand)?;
                match op {
                    UnaryOp::Not => format!("Value::Bool(!{}.is_truthy())", o),
                    UnaryOp::Neg => format!(
                        "Value::Number({{ let Value::Number(n) = &{} else {{ panic!(\"Expected number\") }}; -n }})",
                        o
                    ),
                    UnaryOp::Pos => format!(
                        "Value::Number({{ let Value::Number(n) = &{} else {{ panic!(\"Expected number\") }}; *n }})",
                        o
                    ),
                    UnaryOp::BitNot => format!(
                        "Value::Number({{ let Value::Number(n) = &{} else {{ panic!(\"Expected number\") }}; (!(*n as i32)) as f64 }})",
                        o
                    ),
                    UnaryOp::Void => format!("{{ {}; Value::Null }}", o),
                }
            }
            Expr::Call { callee, args, .. } => {
                // Check for built-in method calls on arrays/strings
                if let Expr::Member { object, prop: MemberProp::Name(method_name), .. } = callee.as_ref() {
                    let obj_expr = self.emit_expr(object)?;
                    let arg_exprs: Result<Vec<_>, _> =
                        args.iter().map(|a| self.emit_expr(a)).collect();
                    let arg_exprs = arg_exprs?;
                    
                    // Array methods
                    match method_name.as_ref() {
                        "push" => {
                            let args_vec = arg_exprs.iter()
                                .map(|a| format!("{}.clone()", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!(
                                "tish_runtime::array_push(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        "pop" => {
                            return Ok(format!("tish_runtime::array_pop(&{})", obj_expr));
                        }
                        "shift" => {
                            return Ok(format!("tish_runtime::array_shift(&{})", obj_expr));
                        }
                        "unshift" => {
                            let args_vec = arg_exprs.iter()
                                .map(|a| format!("{}.clone()", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!(
                                "tish_runtime::array_unshift(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        "indexOf" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tish_runtime::array_index_of(&_obj, &{}), Value::String(_) => tish_runtime::string_index_of(&_obj, &{}), _ => Value::Number(-1.0) }} }}",
                                obj_expr, search, search
                            ));
                        }
                        "includes" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tish_runtime::array_includes(&_obj, &{}), Value::String(_) => tish_runtime::string_includes(&_obj, &{}), _ => Value::Bool(false) }} }}",
                                obj_expr, search, search
                            ));
                        }
                        "join" => {
                            let sep = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_join(&{}, &{})",
                                obj_expr, sep
                            ));
                        }
                        "reverse" => {
                            return Ok(format!("tish_runtime::array_reverse(&{})", obj_expr));
                        }
                        "slice" => {
                            let start = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let end = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tish_runtime::array_slice(&_obj, &{}, &{}), Value::String(_) => tish_runtime::string_slice(&_obj, &{}, &{}), _ => Value::Null }} }}",
                                obj_expr, start, end, start, end
                            ));
                        }
                        "concat" => {
                            let args_vec = arg_exprs.iter()
                                .map(|a| format!("{}.clone()", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!(
                                "tish_runtime::array_concat(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        // String-only methods
                        "substring" => {
                            let start = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let end = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_substring(&{}, &{}, &{})",
                                obj_expr, start, end
                            ));
                        }
                        "split" => {
                            let sep = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_split(&{}, &{})",
                                obj_expr, sep
                            ));
                        }
                        "trim" => {
                            return Ok(format!("tish_runtime::string_trim(&{})", obj_expr));
                        }
                        "toUpperCase" => {
                            return Ok(format!("tish_runtime::string_to_upper_case(&{})", obj_expr));
                        }
                        "toLowerCase" => {
                            return Ok(format!("tish_runtime::string_to_lower_case(&{})", obj_expr));
                        }
                        "startsWith" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_starts_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "endsWith" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_ends_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "replace" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_replace(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "replaceAll" => {
                            let search = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_replace_all(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "charAt" => {
                            let idx = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_char_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "charCodeAt" => {
                            let idx = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_char_code_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "repeat" => {
                            let count = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_repeat(&{}, &{})",
                                obj_expr, count
                            ));
                        }
                        "padStart" => {
                            let target_len = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_pad_start(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        "padEnd" => {
                            let target_len = arg_exprs.get(0).cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_pad_end(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        _ => {} // Fall through to normal function call
                    }
                }
                
                let callee_expr = self.emit_expr(callee)?;
                let arg_exprs: Result<Vec<_>, _> =
                    args.iter().map(|a| self.emit_expr(a)).collect();
                let arg_exprs = arg_exprs?;
                let args_vec = arg_exprs
                    .iter()
                    .map(|a| format!("{}.clone()", a))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "({{\n\
                     {}    let f = &{};\n\
                     {}    match f {{ Value::Function(cb) => cb(&[{}]), _ => panic!(\"Not a function\") }}\n\
                     {}}})",
                    "    ".repeat(self.indent),
                    callee_expr,
                    "    ".repeat(self.indent),
                    args_vec,
                    "    ".repeat(self.indent)
                )
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                let key = match prop {
                    MemberProp::Name(n) => format!("{:?}", n.as_ref()),
                    MemberProp::Expr(e) => {
                        let k = self.emit_expr(e)?;
                        format!("{}.to_display_string()", k)
                    }
                };
                if *optional {
                    format!(
                        "{{ let o = {}.clone(); if matches!(o, Value::Null) {{ Value::Null }} else {{ \
                         tish_runtime::get_prop(&o, {}) }} }}",
                        obj, key
                    )
                } else {
                    format!("tish_runtime::get_prop(&{}, {})", obj, key)
                }
            }
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                if *optional {
                    format!(
                        "{{ let o = {}.clone(); if matches!(o, Value::Null) {{ Value::Null }} else {{ \
                         tish_runtime::get_index(&o, &{}) }} }}",
                        obj, idx
                    )
                } else {
                    format!("tish_runtime::get_index(&{}, &{})", obj, idx)
                }
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_expr(cond)?;
                let t = self.emit_expr(then_branch)?;
                let e = self.emit_expr(else_branch)?;
                format!("if {}.is_truthy() {{ {} }} else {{ {} }}", c, t, e)
            }
            Expr::NullishCoalesce { left, right, .. } => {
                let l = self.emit_expr(left)?;
                let r = self.emit_expr(right)?;
                format!(
                    "{{ let _v = {}.clone(); if matches!(_v, Value::Null) {{ {} }} else {{ _v }} }}",
                    l, r
                )
            }
            Expr::Array { elements, .. } => {
                let els: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.emit_expr(e)).collect();
                let els = els?;
                format!(
                    "Value::Array(Rc::new(RefCell::new(vec![{}])))",
                    els.join(", ")
                )
            }
            Expr::Object { props, .. } => {
                let mut parts = Vec::new();
                for (k, v) in props {
                    let val = self.emit_expr(v)?;
                    parts.push(format!("(Arc::from({:?}), {})", k.as_ref(), val));
                }
                format!(
                    "Value::Object(Rc::new(RefCell::new(HashMap::from([{}]))))",
                    parts.join(", ")
                )
            }
            Expr::Assign { name, value, .. } => {
                let val = self.emit_expr(value)?;
                format!("{{ let _v = {}; {} = _v.clone(); _v }}", val, name.as_ref())
            }
            Expr::TypeOf { operand, .. } => {
                let o = self.emit_expr(operand)?;
                format!(
                    "Value::String(match &{} {{ \
                     Value::Number(_) => \"number\".into(), Value::String(_) => \"string\".into(), \
                     Value::Bool(_) => \"boolean\".into(), Value::Null => \"object\".into(), \
                     Value::Array(_) => \"object\".into(), Value::Object(_) => \"object\".into(), \
                     Value::Function(_) => \"function\".into(), _ => \"object\".into() }})",
                    o
                )
            }
            Expr::PostfixInc { name, .. } => {
                format!(
                    "{{ let _v = {}.clone(); {} = Value::Number(match &_v {{ Value::Number(n) => n + 1.0, _ => panic!(\"++ needs number\") }}); _v }}",
                    name.as_ref(),
                    name.as_ref()
                )
            }
            Expr::PostfixDec { name, .. } => {
                format!(
                    "{{ let _v = {}.clone(); {} = Value::Number(match &_v {{ Value::Number(n) => n - 1.0, _ => panic!(\"-- needs number\") }}); _v }}",
                    name.as_ref(),
                    name.as_ref()
                )
            }
            Expr::PrefixInc { name, .. } => {
                format!(
                    "{{ {} = Value::Number(match &{} {{ Value::Number(n) => n + 1.0, _ => panic!(\"++ needs number\") }}); {}.clone() }}",
                    name.as_ref(),
                    name.as_ref(),
                    name.as_ref()
                )
            }
            Expr::PrefixDec { name, .. } => {
                format!(
                    "{{ {} = Value::Number(match &{} {{ Value::Number(n) => n - 1.0, _ => panic!(\"-- needs number\") }}); {}.clone() }}",
                    name.as_ref(),
                    name.as_ref(),
                    name.as_ref()
                )
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                let val = self.emit_expr(value)?;
                let op_fn = match op {
                    CompoundOp::Add => "add",
                    CompoundOp::Sub => "sub",
                    CompoundOp::Mul => "mul",
                    CompoundOp::Div => "div",
                    CompoundOp::Mod => "modulo",
                };
                format!(
                    "{{ let _rhs = {}; {} = tish_runtime::ops::{}(&{}, &_rhs)?; {}.clone() }}",
                    val,
                    name.as_ref(),
                    op_fn,
                    name.as_ref(),
                    name.as_ref()
                )
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                let obj = self.emit_expr(object)?;
                let val = self.emit_expr(value)?;
                format!(
                    "{{ let _obj = ({}).clone(); let _val = {}; match _obj {{ Value::Object(map) => {{ map.borrow_mut().insert(Arc::from(\"{}\"), _val.clone()); _val }}, _ => panic!(\"Cannot assign property on non-object\") }} }}",
                    obj,
                    val,
                    prop.as_ref()
                )
            }
            Expr::IndexAssign { object, index, value, .. } => {
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let val = self.emit_expr(value)?;
                format!(
                    "{{ let _obj = ({}).clone(); let _idx = {}; let _val = {}; match _obj {{ Value::Array(arr) => {{ let idx = match &_idx {{ Value::Number(n) => *n as usize, _ => panic!(\"Array index must be number\") }}; let mut arr_mut = arr.borrow_mut(); while arr_mut.len() <= idx {{ arr_mut.push(Value::Null); }} arr_mut[idx] = _val.clone(); _val }}, Value::Object(map) => {{ let key: Arc<str> = match &_idx {{ Value::Number(n) => n.to_string().into(), Value::String(s) => Arc::clone(s), _ => panic!(\"Object key must be string or number\") }}; map.borrow_mut().insert(key, _val.clone()); _val }}, _ => panic!(\"Cannot index assign on non-array/object\") }} }}",
                    obj,
                    idx,
                    val
                )
            }
            Expr::ArrowFunction { params, body, .. } => {
                self.emit_arrow_function(params, body)?
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                // Build the template string
                let mut parts = Vec::new();
                for (i, quasi) in quasis.iter().enumerate() {
                    // Escape the quasi string for Rust
                    let escaped = quasi.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t");
                    parts.push(format!("\"{}\"", escaped));
                    if i < exprs.len() {
                        let expr_code = self.emit_expr(&exprs[i])?;
                        parts.push(format!("&({}).to_display_string()", expr_code));
                    }
                }
                format!("Value::String([{}].concat().into())", parts.join(", "))
            }
        })
    }

    fn emit_arrow_function(
        &mut self,
        _params: &[tish_ast::TypedParam],
        _body: &tish_ast::ArrowBody,
    ) -> Result<String, CompileError> {
        // Arrow functions are not yet supported in the compiler
        // They work in the interpreter
        Err(CompileError { message: "Arrow functions are not yet supported in compiled mode. Use named functions instead.".to_string() })
    }

    fn emit_binop(
        &self,
        l: &str,
        op: BinOp,
        r: &str,
    ) -> Result<String, CompileError> {
        Ok(match op {
            BinOp::Add => format!(
                "{{ match (&{}, &{}) {{
                    (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
                    (Value::String(a), Value::String(b)) => Value::String(format!(\"{{}}{{}}\", a, b).into()),
                    (a, b) => Value::String(format!(\"{{}}{{}}\", a.to_display_string(), b.to_display_string()).into()),
                }} }}",
                l, r
            ),
            BinOp::Sub => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a - b }})",
                l, r
            ),
            BinOp::Mul => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a * b }})",
                l, r
            ),
            BinOp::Div => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a / b }})",
                l, r
            ),
            BinOp::Pow => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a.powf(*b) }})",
                l, r
            ),
            BinOp::Mod => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a % b }})",
                l, r
            ),
            BinOp::StrictEq => format!("Value::Bool({}.strict_eq(&{}))", l, r),
            BinOp::StrictNe => format!("Value::Bool(!{}.strict_eq(&{}))", l, r),
            BinOp::Lt => format!(
                "Value::Bool({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a < b }})",
                l, r
            ),
            BinOp::Le => format!(
                "Value::Bool({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a <= b }})",
                l, r
            ),
            BinOp::Gt => format!(
                "Value::Bool({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a > b }})",
                l, r
            ),
            BinOp::Ge => format!(
                "Value::Bool({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; a >= b }})",
                l, r
            ),
            BinOp::And => format!("Value::Bool({}.is_truthy() && {}.is_truthy())", l, r),
            BinOp::Or => format!("Value::Bool({}.is_truthy() || {}.is_truthy())", l, r),
            BinOp::BitAnd => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; ((*a as i32) & (*b as i32)) as f64 }})",
                l, r
            ),
            BinOp::BitOr => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; ((*a as i32) | (*b as i32)) as f64 }})",
                l, r
            ),
            BinOp::BitXor => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; ((*a as i32) ^ (*b as i32)) as f64 }})",
                l, r
            ),
            BinOp::Shl => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; ((*a as i32) << (*b as i32)) as f64 }})",
                l, r
            ),
            BinOp::Shr => format!(
                "Value::Number({{ let Value::Number(a) = &{} else {{ panic!() }}; let Value::Number(b) = &{} else {{ panic!() }}; ((*a as i32) >> (*b as i32)) as f64 }})",
                l, r
            ),
            BinOp::In => format!("tish_in_operator(&{}, &{})", l, r),
            BinOp::Eq | BinOp::Ne => {
                return Err(CompileError {
                    message: "Loose equality not supported".to_string(),
                })
            }
        })
    }
}
