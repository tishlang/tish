//! Code generation: AST -> Rust source.

use std::collections::HashMap;
use std::sync::Arc;

use tish_ast::{BinOp, Expr, Literal, MemberProp, Program, Statement, UnaryOp};

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
    func_index: usize,
    func_names: HashMap<Arc<str>, String>, // Tish name -> Rust fn name
}

impl Codegen {
    fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            func_index: 0,
            func_names: HashMap::new(),
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
        self.write("use std::collections::HashMap;\n");
        self.write("use std::rc::Rc;\n");
        self.write("use std::sync::Arc;\n");
        self.write("use tish_runtime::{print as tish_print, Value};\n\n");

        // First pass: emit function declarations
        for stmt in &program.statements {
            if let Statement::FunDecl { name, .. } = stmt {
                let rust_name = format!("tish_fn_{}", self.func_index);
                self.func_index += 1;
                self.func_names.insert(Arc::clone(name), rust_name);
            }
        }

        for stmt in &program.statements {
            if let Statement::FunDecl { .. } = stmt {
                self.emit_fun_decl(stmt)?;
            }
        }

        self.writeln("fn main() {");
        self.indent += 1;

        // Initialize builtins
        self.writeln("let print = Value::Function(Rc::new(|args: &[Value]| {");
        self.indent += 1;
        self.writeln("tish_print(args);");
        self.writeln("Value::Null");
        self.indent -= 1;
        self.writeln("}));");

        for stmt in &program.statements {
            self.emit_statement(stmt)?;
        }

        self.indent -= 1;
        self.writeln("}");
        Ok(())
    }

    fn emit_fun_decl(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        let (name, params, body) = match stmt {
            Statement::FunDecl {
                name,
                params,
                body,
                ..
            } => (name, params, body),
            _ => unreachable!(),
        };
        let rust_name = self.func_names.get(name).unwrap();
        self.write(&format!(
            "fn {}(args: &[Value]) -> Value {{\n",
            rust_name
        ));
        self.indent += 1;
        for (i, p) in params.iter().enumerate() {
            self.writeln(&format!(
                "let {} = args.get({}).cloned().unwrap_or(Value::Null);",
                p.as_ref(),
                i
            ));
        }
        self.emit_statement(body)?;
        self.writeln("Value::Null"); // fallthrough if no return
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
            Statement::VarDecl { name, init, .. } => {
                let expr = init
                    .as_ref()
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_else(|| "Value::Null".to_string());
                self.writeln(&format!("let mut {} = {};", name.as_ref(), expr));
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
                self.write(&format!("while {}.is_truthy() {{\n", c));
                self.indent += 1;
                self.emit_statement(body)?;
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
                let cond_expr = cond
                    .as_ref()
                    .map(|c| format!("{}.is_truthy()", self.emit_expr(c).unwrap()))
                    .unwrap_or_else(|| "true".to_string());
                self.write(&format!("while {} {{\n", cond_expr));
                self.indent += 1;
                self.emit_statement(body)?;
                if let Some(u) = update {
                    let ue = self.emit_expr(u)?;
                    self.writeln(&format!("{};", ue));
                }
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
            Statement::Break { .. } => self.writeln("break;"),
            Statement::Continue { .. } => self.writeln("continue;"),
            Statement::FunDecl { name, .. } => {
                let rust_name = self.func_names.get(name).unwrap();
                self.writeln(&format!(
                    "let {} = Value::Function(Rc::new(|args: &[Value]| {}(args)));",
                    name.as_ref(),
                    rust_name
                ));
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
                }
            }
            Expr::Call { callee, args, .. } => {
                let callee_expr = self.emit_expr(callee)?;
                let arg_exprs: Result<Vec<_>, _> =
                    args.iter().map(|a| self.emit_expr(a)).collect();
                let arg_exprs = arg_exprs?;
                let args_vec = arg_exprs.join(", ");
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
                        "{{ let o = {}; if matches!(o, Value::Null) {{ Value::Null }} else {{ \
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
                        "{{ let o = {}; if matches!(o, Value::Null) {{ Value::Null }} else {{ \
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
                    "{{ let _v = {}; if matches!(_v, Value::Null) {{ {} }} else {{ _v }} }}",
                    l, r
                )
            }
            Expr::Array { elements, .. } => {
                let els: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.emit_expr(e)).collect();
                let els = els?;
                format!(
                    "Value::Array(Rc::new(vec![{}]))",
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
                    "Value::Object(Rc::new(HashMap::from([{}])))",
                    parts.join(", ")
                )
            }
            Expr::Assign { name, value, .. } => {
                let val = self.emit_expr(value)?;
                format!("{{ let _v = {}; {} = _v.clone(); _v }}", val, name.as_ref())
            }
        })
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
            BinOp::Eq | BinOp::Ne => {
                return Err(CompileError {
                    message: "Loose equality not supported".to_string(),
                })
            }
        })
    }
}
