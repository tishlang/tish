//! Code generation: AST -> Rust source.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use tish_ast::{ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern, Expr, Literal, MemberProp, ObjectProp, Program, Statement, UnaryOp};
use crate::types::{RustType, TypeContext};

/// Tracks variable usage for move/clone optimization.
/// A variable can be moved instead of cloned if it's at its last use.
#[derive(Debug, Default)]
struct UsageAnalyzer {
    /// Count of remaining uses for each variable in the current scope
    use_counts: HashMap<String, usize>,
}

impl UsageAnalyzer {
    fn new() -> Self {
        Self::default()
    }

    /// Analyze a list of statements to count variable uses
    fn analyze_statements(&mut self, stmts: &[Statement]) {
        for stmt in stmts {
            self.analyze_statement(stmt);
        }
    }

    fn analyze_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::VarDecl { init, .. } => {
                if let Some(e) = init {
                    self.analyze_expr(e);
                }
            }
            Statement::VarDeclDestructure { init, .. } => self.analyze_expr(init),
            Statement::ExprStmt { expr, .. } => self.analyze_expr(expr),
            Statement::Return { value, .. } => {
                if let Some(e) = value {
                    self.analyze_expr(e);
                }
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                self.analyze_expr(cond);
                self.analyze_statement(then_branch);
                if let Some(e) = else_branch {
                    self.analyze_statement(e);
                }
            }
            Statement::Block { statements, .. } => self.analyze_statements(statements),
            Statement::For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    self.analyze_statement(i);
                }
                if let Some(c) = cond {
                    self.analyze_expr(c);
                }
                if let Some(u) = update {
                    self.analyze_expr(u);
                }
                self.analyze_statement(body);
            }
            Statement::ForOf { iterable, body, .. } => {
                self.analyze_expr(iterable);
                self.analyze_statement(body);
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                self.analyze_expr(cond);
                self.analyze_statement(body);
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                self.analyze_expr(expr);
                for (case_expr, stmts) in cases {
                    if let Some(e) = case_expr {
                        self.analyze_expr(e);
                    }
                    self.analyze_statements(stmts);
                }
                if let Some(stmts) = default_body {
                    self.analyze_statements(stmts);
                }
            }
            Statement::Throw { value, .. } => self.analyze_expr(value),
            Statement::Try { body, catch_body, finally_body, .. } => {
                self.analyze_statement(body);
                if let Some(c) = catch_body {
                    self.analyze_statement(c);
                }
                if let Some(f) = finally_body {
                    self.analyze_statement(f);
                }
            }
            Statement::FunDecl { body, .. } => {
                self.analyze_statement(body);
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident { name, .. } => {
                *self.use_counts.entry(name.to_string()).or_insert(0) += 1;
            }
            Expr::Literal { .. } => {}
            Expr::Binary { left, right, .. } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::Unary { operand, .. } => self.analyze_expr(operand),
            Expr::Call { callee, args, .. } => {
                self.analyze_expr(callee);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => self.analyze_expr(e),
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                self.analyze_expr(object);
                if let MemberProp::Expr(e) = prop {
                    self.analyze_expr(e);
                }
            }
            Expr::Index { object, index, .. } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
            }
            Expr::Array { elements, .. } => {
                for elem in elements {
                    match elem {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => self.analyze_expr(e),
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, v) => self.analyze_expr(v),
                        ObjectProp::Spread(e) => self.analyze_expr(e),
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => {
                match body {
                    ArrowBody::Expr(e) => self.analyze_expr(e),
                    ArrowBody::Block(s) => self.analyze_statement(s),
                }
            }
            Expr::Assign { value, .. } => self.analyze_expr(value),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                self.analyze_expr(cond);
                self.analyze_expr(then_branch);
                self.analyze_expr(else_branch);
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::TypeOf { operand, .. } => self.analyze_expr(operand),
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    self.analyze_expr(e);
                }
            }
            Expr::CompoundAssign { value, name, .. } => {
                *self.use_counts.entry(name.to_string()).or_insert(0) += 1;
                self.analyze_expr(value);
            }
            Expr::PostfixInc { name, .. } | Expr::PostfixDec { name, .. } | Expr::PrefixInc { name, .. } | Expr::PrefixDec { name, .. } => {
                *self.use_counts.entry(name.to_string()).or_insert(0) += 1;
            }
            Expr::MemberAssign { object, value, .. } => {
                self.analyze_expr(object);
                self.analyze_expr(value);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
                self.analyze_expr(value);
            }
        }
    }

    /// Check if a variable use is its last use (use_count will be 1 after decrement)
    fn is_last_use(&mut self, name: &str) -> bool {
        if let Some(count) = self.use_counts.get_mut(name) {
            if *count > 0 {
                *count -= 1;
                return *count == 0;
            }
        }
        false
    }
}

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
    /// Stack of scopes, each containing function names declared in that scope
    /// Used to capture sibling functions for mutual recursion
    function_scope_stack: Vec<Vec<String>>,
    /// Stack of parameter names from outer function scopes
    /// Used to clone outer parameters for nested function captures
    outer_params_stack: Vec<Vec<String>>,
    /// Stack of variable names declared in outer scopes (module level and outer functions)
    /// Used to capture outer variables for closures
    outer_vars_stack: Vec<Vec<String>>,
    /// Variables currently wrapped in Rc<RefCell<Value>> for mutable capture in closures
    /// These need special handling: reads via .borrow().clone(), writes via *var.borrow_mut()
    refcell_wrapped_vars: std::collections::HashSet<String>,
    /// Usage analyzer for move/clone optimization
    usage_analyzer: Option<UsageAnalyzer>,
    /// Type context for tracking variable types (for static typing)
    type_context: TypeContext,
}

impl Codegen {
    fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            loop_label_index: 0,
            loop_stack: Vec::new(),
            function_scope_stack: vec![Vec::new()], // Start with global scope
            outer_params_stack: Vec::new(),
            outer_vars_stack: vec![Vec::new()], // Start with module-level scope
            refcell_wrapped_vars: std::collections::HashSet::new(),
            usage_analyzer: None,
            type_context: TypeContext::new(),
        }
    }

    fn writeln(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
        self.output.push_str(s);
        self.output.push('\n');
    }

    /// Pre-scan statements to find all function declarations in this scope
    fn prescan_function_decls(&self, statements: &[Statement]) -> Vec<String> {
        statements
            .iter()
            .filter_map(|s| {
                if let Statement::FunDecl { name, .. } = s {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Escape Rust reserved keywords by prefixing with r#
    fn escape_ident(name: &str) -> Cow<'_, str> {
        const RUST_KEYWORDS: &[&str] = &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn",
            "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
            "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
            "self", "Self", "static", "struct", "super", "trait", "true", "type",
            "unsafe", "use", "where", "while", "abstract", "become", "box", "do",
            "final", "macro", "override", "priv", "try", "typeof", "unsized",
            "virtual", "yield",
        ];
        if RUST_KEYWORDS.contains(&name) {
            Cow::Owned(format!("r#{}", name))
        } else {
            Cow::Borrowed(name)
        }
    }

    /// Check if an expression produces a new value that doesn't need cloning.
    /// Literals, newly constructed arrays/objects, function calls, and arrow functions
    /// all produce new values. Variable references and property accesses need cloning.
    fn needs_clone(expr: &Expr) -> bool {
        !matches!(
            expr,
            Expr::Literal { .. }
                | Expr::Array { .. }
                | Expr::Object { .. }
                | Expr::Call { .. }
                | Expr::ArrowFunction { .. }
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::TypeOf { .. }
                | Expr::TemplateLiteral { .. }
        )
    }

    /// Check if we should clone this expression, taking into account last-use optimization.
    /// If this is a simple variable identifier at its last use, we can move instead of clone.
    fn should_clone(&mut self, expr: &Expr) -> bool {
        if !Self::needs_clone(expr) {
            return false;
        }
        
        // Check for last-use optimization on simple identifiers
        if let Expr::Ident { name, .. } = expr {
            // Don't optimize RefCell-wrapped vars (they're borrowed, not owned)
            if self.refcell_wrapped_vars.contains(name.as_ref()) {
                return true;
            }
            
            // Check if this is the last use
            if let Some(ref mut analyzer) = self.usage_analyzer {
                if analyzer.is_last_use(name.as_ref()) {
                    return false; // Can move instead of clone!
                }
            }
        }
        
        true
    }

    /// Generate code for a numeric binary operation that returns Number.
    fn emit_numeric_binop(l: &str, r: &str, op: &str) -> String {
        format!(
            "Value::Number({{ let Value::Number(a) = &({}) else {{ panic!() }}; \
             let Value::Number(b) = &({}) else {{ panic!() }}; a {} b }})",
            l, r, op
        )
    }

    /// Generate code for increment/decrement operations.
    /// `is_prefix`: true for ++x/--x, false for x++/x--
    /// `delta`: "+1.0" or "-1.0"
    /// `op_name`: "++" or "--" for error message
    fn emit_inc_dec(&self, name: &str, is_prefix: bool, delta: &str, op_name: &str) -> String {
        let n = Self::escape_ident(name);
        let is_wrapped = self.refcell_wrapped_vars.contains(name);
        
        if is_prefix {
            if is_wrapped {
                format!(
                    "{{ *{n}.borrow_mut() = Value::Number(match &*{n}.borrow() {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); {n}.borrow().clone() }}"
                )
            } else {
                format!(
                    "{{ {n} = Value::Number(match &{n} {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); {n}.clone() }}"
                )
            }
        } else {
            if is_wrapped {
                format!(
                    "{{ let _v = {n}.borrow().clone(); *{n}.borrow_mut() = Value::Number(match &_v {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); _v }}"
                )
            } else {
                format!(
                    "{{ let _v = {n}.clone(); {n} = Value::Number(match &_v {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); _v }}"
                )
            }
        }
    }

    /// Generate code for a numeric comparison that returns Bool.
    fn emit_numeric_cmp(l: &str, r: &str, op: &str) -> String {
        format!(
            "Value::Bool({{ let Value::Number(a) = &({}) else {{ panic!() }}; \
             let Value::Number(b) = &({}) else {{ panic!() }}; a {} b }})",
            l, r, op
        )
    }

    /// Generate code for a bitwise binary operation.
    fn emit_bitwise_binop(l: &str, r: &str, op: &str) -> String {
        format!(
            "Value::Number({{ let Value::Number(a) = &({}) else {{ panic!() }}; \
             let Value::Number(b) = &({}) else {{ panic!() }}; ((*a as i32) {} (*b as i32)) as f64 }})",
            l, r, op
        )
    }

    fn write(&mut self, s: &str) {
        self.output.push_str(s);
    }

    /// Detect if an expression is a numeric sort comparator: (a, b) => a - b or (a, b) => b - a
    /// Returns Some(true) for ascending, Some(false) for descending, None if not detected
    fn detect_numeric_sort_comparator(expr: &Expr) -> Option<bool> {
        use tish_ast::ArrowBody;
        
        if let Expr::ArrowFunction { params, body, .. } = expr {
            // Must have exactly 2 params
            if params.len() != 2 {
                return None;
            }
            let param_a = params[0].name.as_ref();
            let param_b = params[1].name.as_ref();
            
            // Body must be a single expression that's a subtraction
            let body_expr = match body {
                ArrowBody::Expr(e) => e.as_ref(),
                ArrowBody::Block(stmt) => {
                    if let Statement::ExprStmt { expr, .. } = stmt.as_ref() {
                        expr
                    } else {
                        return None;
                    }
                }
            };
            
            if let Expr::Binary { left, op: BinOp::Sub, right, .. } = body_expr {
                // Check for a - b (ascending) or b - a (descending)
                if let (Expr::Ident { name: left_name, .. }, Expr::Ident { name: right_name, .. }) = (left.as_ref(), right.as_ref()) {
                    if left_name.as_ref() == param_a && right_name.as_ref() == param_b {
                        return Some(true); // ascending
                    }
                    if left_name.as_ref() == param_b && right_name.as_ref() == param_a {
                        return Some(false); // descending
                    }
                }
            }
        }
        None
    }

    fn emit_program(&mut self, program: &Program) -> Result<(), CompileError> {
        self.write("#![allow(unused, non_snake_case)]\n\n");
        self.write("use std::cell::RefCell;\n");
        self.write("use std::collections::HashMap;\n");
        self.write("use std::rc::Rc;\n");
        self.write("use std::sync::Arc;\n");
        self.write("use tish_runtime::{console_debug as tish_console_debug, console_info as tish_console_info, console_log as tish_console_log, console_warn as tish_console_warn, console_error as tish_console_error, decode_uri as tish_decode_uri, encode_uri as tish_encode_uri, in_operator as tish_in_operator, is_finite as tish_is_finite, is_nan as tish_is_nan, json_parse as tish_json_parse, json_stringify as tish_json_stringify, math_abs as tish_math_abs, math_ceil as tish_math_ceil, math_floor as tish_math_floor, math_max as tish_math_max, math_min as tish_math_min, math_round as tish_math_round, math_sqrt as tish_math_sqrt, parse_float as tish_parse_float, parse_int as tish_parse_int, math_random as tish_math_random, math_pow as tish_math_pow, math_sin as tish_math_sin, math_cos as tish_math_cos, math_tan as tish_math_tan, math_log as tish_math_log, math_exp as tish_math_exp, math_sign as tish_math_sign, math_trunc as tish_math_trunc, date_now as tish_date_now, array_is_array as tish_array_is_array, string_from_char_code as tish_string_from_char_code, object_assign as tish_object_assign, object_keys as tish_object_keys, object_values as tish_object_values, object_entries as tish_object_entries, object_from_entries as tish_object_from_entries, TishError, Value};\n");
        #[cfg(feature = "process")]
        self.write("use tish_runtime::{process_exit as tish_process_exit, process_cwd as tish_process_cwd};\n");
        #[cfg(feature = "http")]
        self.write("use tish_runtime::{http_fetch as tish_http_fetch, http_fetch_all as tish_http_fetch_all, http_serve as tish_http_serve};\n");
        #[cfg(feature = "fs")]
        self.write("use tish_runtime::{read_file as tish_read_file, write_file as tish_write_file, file_exists as tish_file_exists, read_dir as tish_read_dir, mkdir as tish_mkdir};\n");
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
        self.writeln("(Arc::from(\"random\"), Value::Function(Rc::new(|args: &[Value]| tish_math_random(args)))),");
        self.writeln("(Arc::from(\"pow\"), Value::Function(Rc::new(|args: &[Value]| tish_math_pow(args)))),");
        self.writeln("(Arc::from(\"sin\"), Value::Function(Rc::new(|args: &[Value]| tish_math_sin(args)))),");
        self.writeln("(Arc::from(\"cos\"), Value::Function(Rc::new(|args: &[Value]| tish_math_cos(args)))),");
        self.writeln("(Arc::from(\"tan\"), Value::Function(Rc::new(|args: &[Value]| tish_math_tan(args)))),");
        self.writeln("(Arc::from(\"log\"), Value::Function(Rc::new(|args: &[Value]| tish_math_log(args)))),");
        self.writeln("(Arc::from(\"exp\"), Value::Function(Rc::new(|args: &[Value]| tish_math_exp(args)))),");
        self.writeln("(Arc::from(\"sign\"), Value::Function(Rc::new(|args: &[Value]| tish_math_sign(args)))),");
        self.writeln("(Arc::from(\"trunc\"), Value::Function(Rc::new(|args: &[Value]| tish_math_trunc(args)))),");
        self.writeln("(Arc::from(\"PI\"), Value::Number(std::f64::consts::PI)),");
        self.writeln("(Arc::from(\"E\"), Value::Number(std::f64::consts::E)),");
        self.indent -= 1;
        self.writeln("]))));");
        self.writeln("let JSON = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"parse\"), Value::Function(Rc::new(|args: &[Value]| tish_json_parse(args)))),");
        self.writeln("(Arc::from(\"stringify\"), Value::Function(Rc::new(|args: &[Value]| tish_json_stringify(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Array = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"isArray\"), Value::Function(Rc::new(|args: &[Value]| tish_array_is_array(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let String = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"fromCharCode\"), Value::Function(Rc::new(|args: &[Value]| tish_string_from_char_code(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Date = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"now\"), Value::Function(Rc::new(|args: &[Value]| tish_date_now(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Object = Value::Object(Rc::new(RefCell::new(HashMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"assign\"), Value::Function(Rc::new(|args: &[Value]| tish_object_assign(args)))),");
        self.writeln("(Arc::from(\"keys\"), Value::Function(Rc::new(|args: &[Value]| tish_object_keys(args)))),");
        self.writeln("(Arc::from(\"values\"), Value::Function(Rc::new(|args: &[Value]| tish_object_values(args)))),");
        self.writeln("(Arc::from(\"entries\"), Value::Function(Rc::new(|args: &[Value]| tish_object_entries(args)))),");
        self.writeln("(Arc::from(\"fromEntries\"), Value::Function(Rc::new(|args: &[Value]| tish_object_from_entries(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        #[cfg(feature = "process")]
        {
            self.writeln("let process = Value::Object(Rc::new(RefCell::new({");
            self.indent += 1;
            self.writeln("let mut p = HashMap::new();");
            self.writeln("p.insert(Arc::from(\"exit\"), Value::Function(Rc::new(|args: &[Value]| tish_process_exit(args))));");
            self.writeln("p.insert(Arc::from(\"cwd\"), Value::Function(Rc::new(|args: &[Value]| tish_process_cwd(args))));");
            self.writeln("let argv: Vec<Value> = std::env::args().map(|s| Value::String(s.into())).collect();");
            self.writeln("p.insert(Arc::from(\"argv\"), Value::Array(Rc::new(RefCell::new(argv))));");
            self.writeln("let mut env_obj = HashMap::new();");
            self.writeln("for (key, value) in std::env::vars() {");
            self.indent += 1;
            self.writeln("env_obj.insert(Arc::from(key.as_str()), Value::String(value.into()));");
            self.indent -= 1;
            self.writeln("}");
            self.writeln("p.insert(Arc::from(\"env\"), Value::Object(Rc::new(RefCell::new(env_obj))));");
            self.writeln("p");
            self.indent -= 1;
            self.writeln("})));");
        }

        #[cfg(feature = "http")]
        {
            self.writeln("let fetch = Value::Function(Rc::new(|args: &[Value]| tish_http_fetch(args)));");
            self.writeln("let fetchAll = Value::Function(Rc::new(|args: &[Value]| tish_http_fetch_all(args)));");
            self.writeln("let serve = Value::Function(Rc::new(|args: &[Value]| {");
            self.indent += 1;
            self.writeln("let port = args.first().cloned().unwrap_or(Value::Null);");
            self.writeln("let handler = args.get(1).cloned().unwrap_or(Value::Null);");
            self.writeln("if let Value::Function(f) = handler {");
            self.indent += 1;
            self.writeln("tish_http_serve(&[port], move |req_args| f(req_args))");
            self.indent -= 1;
            self.writeln("} else {");
            self.indent += 1;
            self.writeln("Value::Null");
            self.indent -= 1;
            self.writeln("}");
            self.indent -= 1;
            self.writeln("}));");
        }

        #[cfg(feature = "fs")]
        {
            self.writeln("let readFile = Value::Function(Rc::new(|args: &[Value]| tish_read_file(args)));");
            self.writeln("let writeFile = Value::Function(Rc::new(|args: &[Value]| tish_write_file(args)));");
            self.writeln("let fileExists = Value::Function(Rc::new(|args: &[Value]| tish_file_exists(args)));");
            self.writeln("let readDir = Value::Function(Rc::new(|args: &[Value]| tish_read_dir(args)));");
            self.writeln("let mkdir = Value::Function(Rc::new(|args: &[Value]| tish_mkdir(args)));");
        }

        // Pre-scan for top-level function declarations and create cells (for mutual recursion)
        let top_level_funcs = self.prescan_function_decls(&program.statements);
        *self.function_scope_stack.last_mut().unwrap() = top_level_funcs.clone();
        for func_name in &top_level_funcs {
            let escaped = Self::escape_ident(func_name);
            self.writeln(&format!("let {}_cell: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));", escaped));
        }

        // Initialize usage analyzer for move/clone optimization
        let mut analyzer = UsageAnalyzer::new();
        analyzer.analyze_statements(&program.statements);
        self.usage_analyzer = Some(analyzer);

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
                // Pre-scan for function declarations and create cells (for mutual recursion)
                let func_names = self.prescan_function_decls(statements);
                self.function_scope_stack.push(func_names.clone());
                // Create cells for all functions in this scope
                for func_name in &func_names {
                    let escaped = Self::escape_ident(func_name);
                    self.writeln(&format!("let {}_cell: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));", escaped));
                }
                for s in statements {
                    self.emit_statement(s)?;
                }
                self.function_scope_stack.pop(); // Exit scope
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::VarDecl { name, mutable, type_ann, init, .. } => {
                // Determine the Rust type from annotation
                let rust_type = type_ann
                    .as_ref()
                    .map(RustType::from_annotation)
                    .unwrap_or(RustType::Value);
                
                // DEBUG: Write type info to file
                std::fs::write("/tmp/tish_debug.log", format!("VarDecl: {} type_ann={:?} rust_type={:?} is_native={}\n", 
                    name, type_ann, rust_type, rust_type.is_native())).ok();
                
                // Track the variable type
                self.type_context.define(name.as_ref(), rust_type.clone());
                
                let mutability = if *mutable { "let mut" } else { "let" };
                let escaped_name = Self::escape_ident(name.as_ref());
                
                if rust_type.is_native() {
                    // Generate native typed variable
                    let type_str = rust_type.to_rust_type_str();
                    let expr_str = match init.as_ref() {
                        Some(e) => self.emit_native_expr(e, &rust_type)?,
                        None => rust_type.default_value(),
                    };
                    self.writeln(&format!("{} {}: {} = {};", mutability, escaped_name, type_str, expr_str));
                } else {
                    // Original Value-based codegen
                    let (expr_str, clone_needed) = match init.as_ref() {
                        Some(e) => {
                            let s = self.emit_expr(e)?;
                            let needs = self.should_clone(e);
                            (s, needs)
                        }
                        None => ("Value::Null".to_string(), false),
                    };
                    if clone_needed {
                        self.writeln(&format!("{} {} = ({}).clone();", mutability, escaped_name, expr_str));
                    } else {
                        self.writeln(&format!("{} {} = {};", mutability, escaped_name, expr_str));
                    }
                }
                
                if let Some(scope) = self.outer_vars_stack.last_mut() {
                    scope.push(name.to_string());
                }
            }
            Statement::VarDeclDestructure { pattern, mutable, init, .. } => {
                let expr = self.emit_expr(init)?;
                let mutability = if *mutable { "let mut" } else { "let" };
                let clone_suffix = if Self::needs_clone(init) { ".clone()" } else { "" };
                self.writeln(&format!("{{ let _destruct_val = ({}){};", expr, clone_suffix));
                self.indent += 1;
                self.emit_destruct_bindings(pattern, "_destruct_val", mutability)?;
                self.indent -= 1;
                self.writeln("}");
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
                self.writeln(&format!("{{ let _fof = ({}).clone();", iter_expr));
                self.indent += 1;
                self.writeln("match &_fof {");
                self.indent += 1;
                self.writeln("Value::Array(ref _arr) => {");
                self.indent += 1;
                self.writeln("for _v in _arr.borrow().iter() {");
                self.indent += 1;
                self.writeln(&format!("let {} = _v.clone();", Self::escape_ident(name.as_ref())));
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
                    Self::escape_ident(name.as_ref())
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
                finally_body,
                ..
            } => {
                self.writeln("let _try_result: Result<Value, Box<dyn std::error::Error>> = (|| {");
                self.indent += 1;
                self.emit_statement(body)?;
                self.writeln("Ok(Value::Null)");
                self.indent -= 1;
                self.writeln("})();");
                
                if let Some(catch_stmt) = catch_body {
                    if let Some(param) = catch_param {
                        self.writeln("if let Err(e) = _try_result {");
                        self.indent += 1;
                        self.writeln("match e.downcast::<tish_runtime::TishError>() {");
                        self.indent += 1;
                        self.writeln("Ok(tish_err) => {");
                        self.indent += 1;
                        self.writeln("if let tish_runtime::TishError::Throw(v) = *tish_err {");
                        self.writeln(&format!("let {} = v.clone();", Self::escape_ident(param.as_ref())));
                        self.emit_statement(catch_stmt)?;
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
                        self.emit_statement(catch_stmt)?;
                        self.indent -= 1;
                    }
                    self.writeln("}");
                }
                
                if let Some(finally_stmt) = finally_body {
                    self.emit_statement(finally_stmt)?;
                }
            }
            Statement::FunDecl { name, params, rest_param, body, .. } => {
                // Use Rc<RefCell<>> pattern to allow recursive function calls
                // The function can reference itself through the cell
                let name_raw = name.as_ref();
                let name_str = Self::escape_ident(name_raw);
                // Check if cell was already created by block prescan
                let cell_exists = self.function_scope_stack
                    .last()
                    .map(|scope| scope.contains(&name_raw.to_string()))
                    .unwrap_or(false);
                if !cell_exists {
                    self.writeln(&format!("let {}_cell: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));", name_str));
                }
                
                // Analyze body to find which identifiers are actually referenced
                let mut referenced = HashSet::new();
                Self::collect_stmt_idents(body, &mut referenced);
                let param_names: HashSet<String> = params.iter().map(|p| p.name.to_string()).collect();
                
                // Collect all outer parameters that need to be captured (only those referenced)
                let outer_params: Vec<String> = self.outer_params_stack
                    .iter()
                    .flat_map(|p| p.iter().cloned())
                    .filter(|name| referenced.contains(name) && !param_names.contains(name))
                    .collect();
                
                self.writeln(&format!("let {} = {{", name_str));
                self.indent += 1;
                // Clone the cell so the closure can reference the function recursively
                // Only clone if the function references itself (recursion)
                let needs_self_ref = referenced.contains(name_raw);
                if needs_self_ref {
                    self.writeln(&format!("let {}_ref = {}_cell.clone();", name_str, name_str));
                }
                // Clone sibling function cells for mutual recursion - only those actually referenced
                let sibling_fns: Vec<String> = self.function_scope_stack
                    .last()
                    .map(|scope| scope.iter()
                        .filter(|s| s.as_str() != name_raw && referenced.contains(s.as_str()))
                        .cloned()
                        .collect())
                    .unwrap_or_default();
                for sibling in &sibling_fns {
                    let sibling_escaped = Self::escape_ident(sibling);
                    self.writeln(&format!("let {}_ref = {}_cell.clone();", sibling_escaped, sibling_escaped));
                }
                // Clone outer parameters so they can be captured by the move closure
                for outer_param in &outer_params {
                    let param_escaped = Self::escape_ident(outer_param);
                    self.writeln(&format!("let {} = {}.clone();", param_escaped, param_escaped));
                }
                // Only clone builtins that are actually referenced
                for builtin in &["console", "Math", "JSON", "Date"] {
                    if referenced.contains(*builtin) {
                        self.writeln(&format!("let {} = {}.clone();", builtin, builtin));
                    }
                }
                self.writeln("Value::Function(Rc::new(move |args: &[Value]| {");
                self.indent += 1;
                // Make the function available by its name inside the closure (only if recursive)
                if needs_self_ref {
                    self.writeln(&format!("let {} = {}_ref.borrow().clone();", name_str, name_str));
                }
                // Make sibling functions available for mutual recursion
                for sibling in &sibling_fns {
                    let sibling_escaped = Self::escape_ident(sibling);
                    self.writeln(&format!("let {} = {}_ref.borrow().clone();", sibling_escaped, sibling_escaped));
                }
                // Extract just the parameter names (type annotations are parsed but not used in codegen yet)
                let current_param_names: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
                for (i, p) in params.iter().enumerate() {
                    self.writeln(&format!(
                        "let {} = args.get({}).cloned().unwrap_or(Value::Null);",
                        Self::escape_ident(p.name.as_ref()),
                        i
                    ));
                }
                if let Some(rest) = rest_param {
                    self.writeln(&format!(
                        "let {} = Value::Array(std::rc::Rc::new(RefCell::new(args[{}..].to_vec())));",
                        Self::escape_ident(rest.name.as_ref()),
                        params.len()
                    ));
                }
                
                // Push current params to stack for nested functions
                self.outer_params_stack.push(current_param_names);
                
                // Pre-scan body for nested functions (handles function body as Block)
                if let Statement::Block { statements, .. } = body.as_ref() {
                    let nested_func_names = self.prescan_function_decls(statements);
                    self.function_scope_stack.push(nested_func_names.clone());
                    // Create cells for nested functions
                    for func_name in &nested_func_names {
                        let escaped = Self::escape_ident(func_name);
                        self.writeln(&format!("let {}_cell: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));", escaped));
                    }
                    for s in statements {
                        self.emit_statement(s)?;
                    }
                    self.function_scope_stack.pop();
                } else {
                    self.function_scope_stack.push(Vec::new());
                    self.emit_statement(body)?;
                    self.function_scope_stack.pop();
                }
                
                // Pop params stack
                self.outer_params_stack.pop();
                
                self.writeln("Value::Null");
                self.indent -= 1;
                self.writeln("}))");
                self.indent -= 1;
                self.writeln("};");
                // Update the cell with the actual function value
                self.writeln(&format!("*{}_cell.borrow_mut() = {}.clone();", name_str, name_str));
            }
        }
        Ok(())
    }

    fn emit_call_arg(&mut self, arg: &CallArg) -> Result<String, CompileError> {
        let e = match arg {
            CallArg::Expr(e) | CallArg::Spread(e) => e,
        };
        self.emit_expr(e)
    }

    fn emit_call_args(&mut self, args: &[CallArg]) -> Result<String, CompileError> {
        let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
        if has_spread {
            let mut parts = Vec::new();
            for arg in args {
                match arg {
                    CallArg::Expr(e) => {
                        let val = self.emit_expr(e)?;
                        if self.should_clone(e) {
                            parts.push(format!("_args.push({}.clone());", val));
                        } else {
                            parts.push(format!("_args.push({});", val));
                        }
                    }
                    CallArg::Spread(e) => {
                        let val = self.emit_expr(e)?;
                        parts.push(format!("if let Value::Array(ref _spread) = {} {{ _args.extend(_spread.borrow().iter().cloned()); }}", val));
                    }
                }
            }
            Ok(format!("{{ let mut _args: Vec<Value> = Vec::new(); {} _args }}", parts.join(" ")))
        } else {
            let mut emitted = Vec::new();
            for arg in args {
                if let CallArg::Expr(e) = arg {
                    let val = self.emit_expr(e)?;
                    if self.should_clone(e) {
                        emitted.push(format!("{}.clone()", val));
                    } else {
                        emitted.push(val);
                    }
                } else {
                    return Err(CompileError { message: "Unexpected spread".to_string() });
                }
            }
            Ok(format!("vec![{}]", emitted.join(", ")))
        }
    }

    fn emit_destruct_bindings(&mut self, pattern: &DestructPattern, value_expr: &str, mutability: &str) -> Result<(), CompileError> {
        match pattern {
            DestructPattern::Array(elements) => {
                self.writeln(&format!("if let Value::Array(ref _arr) = {} {{", value_expr));
                self.indent += 1;
                self.writeln("let _arr_borrow = _arr.borrow();");
                for (i, elem) in elements.iter().enumerate() {
                    if let Some(el) = elem {
                        match el {
                            DestructElement::Ident(name) => {
                                self.writeln(&format!("{} {} = _arr_borrow.get({}).cloned().unwrap_or(Value::Null);", 
                                    mutability, Self::escape_ident(name.as_ref()), i));
                            }
                            DestructElement::Pattern(nested) => {
                                let nested_var = format!("_nested_{}", i);
                                self.writeln(&format!("let {} = _arr_borrow.get({}).cloned().unwrap_or(Value::Null);", 
                                    nested_var, i));
                                self.emit_destruct_bindings(nested, &nested_var, mutability)?;
                            }
                            DestructElement::Rest(name) => {
                                self.writeln(&format!("{} {} = Value::Array(Rc::new(RefCell::new(_arr_borrow.iter().skip({}).cloned().collect())));", 
                                    mutability, Self::escape_ident(name.as_ref()), i));
                            }
                        }
                    }
                }
                self.indent -= 1;
                self.writeln("}");
            }
            DestructPattern::Object(props) => {
                self.writeln(&format!("if let Value::Object(ref _obj) = {} {{", value_expr));
                self.indent += 1;
                self.writeln("let _obj_borrow = _obj.borrow();");
                for prop in props {
                    let key = prop.key.as_ref();
                    match &prop.value {
                        DestructElement::Ident(name) => {
                            self.writeln(&format!("{} {} = _obj_borrow.get({:?}).cloned().unwrap_or(Value::Null);", 
                                mutability, Self::escape_ident(name.as_ref()), key));
                        }
                        DestructElement::Pattern(nested) => {
                            let nested_var = format!("_nested_{}", key);
                            self.writeln(&format!("let {} = _obj_borrow.get({:?}).cloned().unwrap_or(Value::Null);", 
                                nested_var, key));
                            self.emit_destruct_bindings(nested, &nested_var, mutability)?;
                        }
                        DestructElement::Rest(_) => {
                            return Err(CompileError { message: "Rest in object destructuring not supported".to_string() });
                        }
                    }
                }
                self.indent -= 1;
                self.writeln("}");
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
            Expr::Ident { name, .. } => {
                let escaped = Self::escape_ident(name.as_ref());
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    format!("{}.borrow().clone()", escaped)
                } else {
                    // Check if this is a typed variable that needs conversion to Value
                    let var_type = self.type_context.get_type(name.as_ref());
                    if var_type.is_native() {
                        // Convert native type to Value for compatibility with existing code
                        var_type.to_value_expr(&escaped)
                    } else {
                        escaped.into_owned()
                    }
                }
            }
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
                        "Value::Number({{ let Value::Number(n) = &({}) else {{ panic!(\"Expected number\") }}; -n }})",
                        o
                    ),
                    UnaryOp::Pos => format!(
                        "Value::Number({{ let Value::Number(n) = &({}) else {{ panic!(\"Expected number\") }}; *n }})",
                        o
                    ),
                    UnaryOp::BitNot => format!(
                        "Value::Number({{ let Value::Number(n) = &({}) else {{ panic!(\"Expected number\") }}; (!(*n as i32)) as f64 }})",
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
                        args.iter().map(|a| self.emit_call_arg(a)).collect();
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
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tish_runtime::array_index_of(&_obj, &{}), Value::String(_) => tish_runtime::string_index_of(&_obj, &{}), _ => Value::Number(-1.0) }} }}",
                                obj_expr, search, search
                            ));
                        }
                        "includes" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tish_runtime::array_includes(&_obj, &{}), Value::String(_) => tish_runtime::string_includes(&_obj, &{}), _ => Value::Bool(false) }} }}",
                                obj_expr, search, search
                            ));
                        }
                        "join" => {
                            let sep = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_join(&{}, &{})",
                                obj_expr, sep
                            ));
                        }
                        "reverse" => {
                            return Ok(format!("tish_runtime::array_reverse(&{})", obj_expr));
                        }
                        "slice" => {
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
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
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let end = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_substring(&{}, &{}, &{})",
                                obj_expr, start, end
                            ));
                        }
                        "split" => {
                            let sep = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
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
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_starts_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "endsWith" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_ends_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "replace" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_replace(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "replaceAll" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tish_runtime::string_replace_all(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "charAt" => {
                            let idx = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_char_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "charCodeAt" => {
                            let idx = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_char_code_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "repeat" => {
                            let count = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::string_repeat(&{}, &{})",
                                obj_expr, count
                            ));
                        }
                        "padStart" => {
                            let target_len = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_pad_start(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        "padEnd" => {
                            let target_len = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::string_pad_end(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        // Higher-order array methods
                        "map" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_map(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "filter" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_filter(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "reduce" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let initial = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_reduce(&{}, &{}, &{})",
                                obj_expr, callback, initial
                            ));
                        }
                        "forEach" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_for_each(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "find" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_find(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "findIndex" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_find_index(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "some" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_some(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "every" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_every(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "sort" => {
                            // Check for numeric sort fast path: (a, b) => a - b or (a, b) => b - a
                            if let Some(CallArg::Expr(comparator_expr)) = args.first() {
                                if let Some(ascending) = Self::detect_numeric_sort_comparator(comparator_expr) {
                                    if ascending {
                                        return Ok(format!(
                                            "tish_runtime::array_sort_numeric_asc(&{})",
                                            obj_expr
                                        ));
                                    } else {
                                        return Ok(format!(
                                            "tish_runtime::array_sort_numeric_desc(&{})",
                                            obj_expr
                                        ));
                                    }
                                }
                            }
                            // General case: use the callback
                            let comparator = arg_exprs.first().map(|c| format!("Some(&{})", c)).unwrap_or_else(|| "None".to_string());
                            return Ok(format!(
                                "tish_runtime::array_sort(&{}, {})",
                                obj_expr, comparator
                            ));
                        }
                        "splice" => {
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let delete_count = arg_exprs.get(1).map(|d| format!("Some(&{})", d)).unwrap_or_else(|| "None".to_string());
                            let items = if arg_exprs.len() > 2 {
                                let items_vec = arg_exprs[2..].iter()
                                    .map(|a| format!("{}.clone()", a))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!("&[{}]", items_vec)
                            } else {
                                "&[]".to_string()
                            };
                            return Ok(format!(
                                "tish_runtime::array_splice(&{}, &{}, {}, {})",
                                obj_expr, start, delete_count, items
                            ));
                        }
                        "flat" => {
                            let depth = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(1.0)".to_string());
                            return Ok(format!(
                                "tish_runtime::array_flat(&{}, &{})",
                                obj_expr, depth
                            ));
                        }
                        "flatMap" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tish_runtime::array_flat_map(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        _ => {} // Fall through to normal function call
                    }
                }
                
                let callee_expr = self.emit_expr(callee)?;
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    let args_code = self.emit_call_args(args)?;
                    return Ok(format!(
                        "{{ let f = &{}; let _spread_args = {}; match f {{ Value::Function(cb) => cb(&_spread_args), _ => panic!(\"Not a function\") }} }}",
                        callee_expr, args_code
                    ));
                }
                let arg_exprs: Result<Vec<_>, _> =
                    args.iter().map(|a| self.emit_call_arg(a)).collect();
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
                let has_spread = elements.iter().any(|e| matches!(e, ArrayElement::Spread(_)));
                if has_spread {
                    let mut parts = Vec::new();
                    for elem in elements {
                        match elem {
                            ArrayElement::Expr(e) => {
                                let val = self.emit_expr(e)?;
                                if self.should_clone(e) {
                                    parts.push(format!("_arr.push(({}).clone());", val));
                                } else {
                                    parts.push(format!("_arr.push({});", val));
                                }
                            }
                            ArrayElement::Spread(e) => {
                                let val = self.emit_expr(e)?;
                                parts.push(format!("if let Value::Array(ref _spread) = {} {{ _arr.extend(_spread.borrow().iter().cloned()); }}", val));
                            }
                        }
                    }
                    format!("{{ let mut _arr: Vec<Value> = Vec::new(); {} Value::Array(Rc::new(RefCell::new(_arr))) }}", parts.join(" "))
                } else {
                    let mut els = Vec::new();
                    for elem in elements {
                        if let ArrayElement::Expr(expr) = elem {
                            let v = self.emit_expr(expr)?;
                            if self.should_clone(expr) {
                                els.push(format!("({}).clone()", v));
                            } else {
                                els.push(v);
                            }
                        } else {
                            return Err(CompileError { message: "Unexpected spread".to_string() });
                        }
                    }
                    format!(
                        "Value::Array(Rc::new(RefCell::new(vec![{}])))",
                        els.join(", ")
                    )
                }
            }
            Expr::Object { props, .. } => {
                let has_spread = props.iter().any(|p| matches!(p, ObjectProp::Spread(_)));
                if has_spread {
                    let mut parts = Vec::new();
                    for prop in props {
                        match prop {
                            ObjectProp::KeyValue(k, v) => {
                                let val = self.emit_expr(v)?;
                                if self.should_clone(v) {
                                    parts.push(format!("_obj.insert(Arc::from({:?}), ({}).clone());", k.as_ref(), val));
                                } else {
                                    parts.push(format!("_obj.insert(Arc::from({:?}), {});", k.as_ref(), val));
                                }
                            }
                            ObjectProp::Spread(e) => {
                                let val = self.emit_expr(e)?;
                                parts.push(format!("if let Value::Object(ref _spread) = {} {{ for (k, v) in _spread.borrow().iter() {{ _obj.insert(Arc::clone(k), v.clone()); }} }}", val));
                            }
                        }
                    }
                    format!("{{ let mut _obj: HashMap<Arc<str>, Value> = HashMap::new(); {} Value::Object(Rc::new(RefCell::new(_obj))) }}", parts.join(" "))
                } else {
                    let mut parts = Vec::new();
                    for prop in props {
                        if let ObjectProp::KeyValue(k, v) = prop {
                            let val = self.emit_expr(v)?;
                            if self.should_clone(v) {
                                parts.push(format!("(Arc::from({:?}), ({}).clone())", k.as_ref(), val));
                            } else {
                                parts.push(format!("(Arc::from({:?}), {})", k.as_ref(), val));
                            }
                        }
                    }
                    format!(
                        "Value::Object(Rc::new(RefCell::new(HashMap::from([{}]))))",
                        parts.join(", ")
                    )
                }
            }
            Expr::Assign { name, value, .. } => {
                let val = self.emit_expr(value)?;
                let escaped = Self::escape_ident(name.as_ref());
                let needs_outer_clone = self.should_clone(value);
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    if needs_outer_clone {
                        format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    } else {
                        format!("{{ let _v = {}; *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    }
                } else {
                    if needs_outer_clone {
                        format!("{{ let _v = ({}).clone(); {} = _v.clone(); _v }}", val, escaped)
                    } else {
                        format!("{{ let _v = {}; {} = _v.clone(); _v }}", val, escaped)
                    }
                }
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
            Expr::PostfixInc { name, .. } => self.emit_inc_dec(name.as_ref(), false, "+ 1.0", "++"),
            Expr::PostfixDec { name, .. } => self.emit_inc_dec(name.as_ref(), false, "- 1.0", "--"),
            Expr::PrefixInc { name, .. } => self.emit_inc_dec(name.as_ref(), true, "+ 1.0", "++"),
            Expr::PrefixDec { name, .. } => self.emit_inc_dec(name.as_ref(), true, "- 1.0", "--"),
            Expr::CompoundAssign { name, op, value, .. } => {
                let val = self.emit_expr(value)?;
                let n = Self::escape_ident(name.as_ref());
                let op_fn = match op {
                    CompoundOp::Add => "add",
                    CompoundOp::Sub => "sub",
                    CompoundOp::Mul => "mul",
                    CompoundOp::Div => "div",
                    CompoundOp::Mod => "modulo",
                };
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    format!(
                        "{{ let _rhs = ({}).clone(); *{}.borrow_mut() = tish_runtime::ops::{}(&{}.borrow(), &_rhs)?; {}.borrow().clone() }}",
                        val, n, op_fn, n, n
                    )
                } else {
                    format!(
                        "{{ let _rhs = ({}).clone(); {} = tish_runtime::ops::{}(&{}, &_rhs)?; {}.clone() }}",
                        val, n, op_fn, n, n
                    )
                }
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                let obj = self.emit_expr(object)?;
                let val = self.emit_expr(value)?;
                format!(
                    "tish_runtime::set_prop(&({}), \"{}\", ({}).clone())",
                    obj,
                    prop.as_ref(),
                    val
                )
            }
            Expr::IndexAssign { object, index, value, .. } => {
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let val = self.emit_expr(value)?;
                format!(
                    "tish_runtime::set_index(&({}), &({}), ({}).clone())",
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
    
    /// Collect all identifiers referenced in an arrow body
    fn collect_referenced_idents(body: &ArrowBody) -> HashSet<String> {
        let mut idents = HashSet::new();
        match body {
            ArrowBody::Expr(expr) => Self::collect_expr_idents(expr, &mut idents),
            ArrowBody::Block(stmt) => Self::collect_stmt_idents(stmt, &mut idents),
        }
        idents
    }
    
    fn collect_expr_idents(expr: &Expr, idents: &mut HashSet<String>) {
        match expr {
            Expr::Ident { name, .. } => { idents.insert(name.to_string()); }
            Expr::Assign { name, value, .. } => {
                idents.insert(name.to_string());
                Self::collect_expr_idents(value, idents);
            }
            Expr::Binary { left, right, .. } => {
                Self::collect_expr_idents(left, idents);
                Self::collect_expr_idents(right, idents);
            }
            Expr::Unary { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::Call { callee, args, .. } => {
                Self::collect_expr_idents(callee, idents);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => Self::collect_expr_idents(e, idents),
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_expr_idents(object, idents);
                if let MemberProp::Expr(e) = prop { Self::collect_expr_idents(e, idents); }
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(value, idents);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(index, idents);
                Self::collect_expr_idents(value, idents);
            }
            Expr::Index { object, index, .. } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(index, idents);
            }
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_expr_idents(then_branch, idents);
                Self::collect_expr_idents(else_branch, idents);
            }
            Expr::PostfixInc { name, .. } | Expr::PostfixDec { name, .. } |
            Expr::PrefixInc { name, .. } | Expr::PrefixDec { name, .. } => {
                idents.insert(name.to_string());
            }
            Expr::CompoundAssign { name, value, .. } => {
                idents.insert(name.to_string());
                Self::collect_expr_idents(value, idents);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => Self::collect_expr_idents(e, idents),
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => Self::collect_expr_idents(e, idents),
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => {
                match body {
                    ArrowBody::Expr(e) => Self::collect_expr_idents(e, idents),
                    ArrowBody::Block(s) => Self::collect_stmt_idents(s, idents),
                }
            }
            Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_expr_idents(left, idents);
                Self::collect_expr_idents(right, idents);
            }
            Expr::TypeOf { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs { Self::collect_expr_idents(e, idents); }
            }
            Expr::Literal { .. } => {}
        }
    }
    
    fn collect_stmt_idents(stmt: &Statement, idents: &mut HashSet<String>) {
        match stmt {
            Statement::ExprStmt { expr, .. } => Self::collect_expr_idents(expr, idents),
            Statement::VarDecl { init, .. } => {
                if let Some(e) = init { Self::collect_expr_idents(e, idents); }
            }
            Statement::VarDeclDestructure { init, .. } => Self::collect_expr_idents(init, idents),
            Statement::Block { statements, .. } => {
                for s in statements { Self::collect_stmt_idents(s, idents); }
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_stmt_idents(then_branch, idents);
                if let Some(e) = else_branch { Self::collect_stmt_idents(e, idents); }
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_stmt_idents(body, idents);
            }
            Statement::For { init, cond, update, body, .. } => {
                if let Some(s) = init { Self::collect_stmt_idents(s, idents); }
                if let Some(e) = cond { Self::collect_expr_idents(e, idents); }
                if let Some(e) = update { Self::collect_expr_idents(e, idents); }
                Self::collect_stmt_idents(body, idents);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_expr_idents(iterable, idents);
                Self::collect_stmt_idents(body, idents);
            }
            Statement::Return { value, .. } => {
                if let Some(e) = value { Self::collect_expr_idents(e, idents); }
            }
            Statement::Throw { value, .. } => Self::collect_expr_idents(value, idents),
            Statement::Try { body, catch_body, finally_body, .. } => {
                Self::collect_stmt_idents(body, idents);
                if let Some(c) = catch_body { Self::collect_stmt_idents(c, idents); }
                if let Some(f) = finally_body { Self::collect_stmt_idents(f, idents); }
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                Self::collect_expr_idents(expr, idents);
                for (case_expr, stmts) in cases {
                    if let Some(e) = case_expr { Self::collect_expr_idents(e, idents); }
                    for s in stmts { Self::collect_stmt_idents(s, idents); }
                }
                if let Some(stmts) = default_body {
                    for s in stmts { Self::collect_stmt_idents(s, idents); }
                }
            }
            Statement::FunDecl { body, .. } => Self::collect_stmt_idents(body, idents),
            Statement::Break { .. } | Statement::Continue { .. } => {}
        }
    }

    fn emit_arrow_function(
        &mut self,
        params: &[tish_ast::TypedParam],
        body: &tish_ast::ArrowBody,
    ) -> Result<String, CompileError> {
        // Build the arrow function as a Value::Function closure
        let mut code = String::new();
        code.push_str("{\n");
        
        // Find which identifiers are actually referenced in the body
        let referenced = Self::collect_referenced_idents(body);
        // Exclude the arrow's own parameters - they're not outer captures
        let param_names: HashSet<String> = params.iter().map(|p| p.name.to_string()).collect();

        // Collect outer parameters that need to be captured
        let outer_params: Vec<String> = self.outer_params_stack
            .iter()
            .flat_map(|p| p.iter().cloned())
            .filter(|name| referenced.contains(name) && !param_names.contains(name))
            .collect();
        
        // Collect outer variables (from outer scopes) that need to be captured
        let outer_vars: Vec<String> = self.outer_vars_stack
            .iter()
            .flat_map(|v| v.iter().cloned())
            .filter(|name| referenced.contains(name) && !param_names.contains(name))
            .collect();

        // Wrap outer captures in Rc<RefCell<>> so they can be mutated inside the closure
        // This is necessary because Fn closures (required for Rc<dyn Fn>) can't mutate captures
        for outer_param in &outer_params {
            let param_escaped = Self::escape_ident(outer_param);
            code.push_str(&format!("    let {} = std::rc::Rc::new(RefCell::new({}.clone()));\n", param_escaped, param_escaped));
        }
        for outer_var in &outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            code.push_str(&format!("    let {} = std::rc::Rc::new(RefCell::new({}.clone()));\n", var_escaped, var_escaped));
        }
        // Only clone builtins that are actually referenced
        for builtin in &["console", "Math", "JSON", "Date"] {
            if referenced.contains(*builtin) {
                code.push_str(&format!("    let {} = {}.clone();\n", builtin, builtin));
            }
        }

        // Clone only function cells that are actually referenced in this arrow
        let referenced_funcs: Vec<String> = self.function_scope_stack
            .last()
            .map(|scope| scope.iter()
                .filter(|f| referenced.contains(f.as_str()) && !param_names.contains(*f))
                .cloned()
                .collect())
            .unwrap_or_default();
        for func_name in &referenced_funcs {
            let escaped = Self::escape_ident(func_name);
            code.push_str(&format!("    let {}_ref = {}_cell.clone();\n", escaped, escaped));
        }

        code.push_str("    Value::Function(Rc::new(move |args: &[Value]| {\n");

        // Make captured functions available
        for func_name in &referenced_funcs {
            let escaped = Self::escape_ident(func_name);
            code.push_str(&format!("        let {} = {}_ref.borrow().clone();\n", escaped, escaped));
        }

        // Extract parameters from args
        let current_param_names: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
        for (i, p) in params.iter().enumerate() {
            code.push_str(&format!(
                "        let mut {} = args.get({}).cloned().unwrap_or(Value::Null);\n",
                Self::escape_ident(p.name.as_ref()),
                i
            ));
        }

        // Push current params for potential nested arrows
        self.outer_params_stack.push(current_param_names);
        // Push empty scope for variables declared inside this arrow function
        self.outer_vars_stack.push(Vec::new());
        
        // Track outer params and vars as RefCell-wrapped for proper read/write handling
        let saved_refcell_vars = std::mem::take(&mut self.refcell_wrapped_vars);
        for outer_param in &outer_params {
            self.refcell_wrapped_vars.insert(outer_param.clone());
        }
        for outer_var in &outer_vars {
            self.refcell_wrapped_vars.insert(outer_var.clone());
        }

        // Emit body based on type
        match body {
            tish_ast::ArrowBody::Expr(expr) => {
                let expr_code = self.emit_expr(expr)?;
                code.push_str(&format!("        {}\n", expr_code));
            }
            tish_ast::ArrowBody::Block(block_stmt) => {
                // For block bodies, emit the block statement
                self.function_scope_stack.push(Vec::new());
                
                // Save current output, emit to temp, then restore
                let saved_output = std::mem::take(&mut self.output);
                let saved_indent = self.indent;
                self.indent = 2; // Base indent inside the closure
                
                self.emit_statement(block_stmt)?;
                
                let body_code = std::mem::replace(&mut self.output, saved_output);
                self.indent = saved_indent;
                self.function_scope_stack.pop();
                
                code.push_str(&body_code);
                code.push_str("        Value::Null\n");
            }
        }

        // Restore state
        self.refcell_wrapped_vars = saved_refcell_vars;
        self.outer_params_stack.pop();
        self.outer_vars_stack.pop();

        code.push_str("    }))\n");
        code.push('}');

        Ok(code)
    }

    /// Emit an expression as a native Rust type (not wrapped in Value).
    /// Falls back to emit_expr + conversion if the expression cannot be directly
    /// emitted as the target type.
    fn emit_native_expr(&mut self, expr: &Expr, target_type: &RustType) -> Result<String, CompileError> {
        // Try to emit literals directly as native types
        if let Expr::Literal { value, .. } = expr {
            match (target_type, value) {
                (RustType::F64, Literal::Number(n)) => {
                    return Ok(format!("{}_f64", n));
                }
                (RustType::String, Literal::String(s)) => {
                    return Ok(format!("{:?}.to_string()", s.as_ref()));
                }
                (RustType::Bool, Literal::Bool(b)) => {
                    return Ok(format!("{}", b));
                }
                (RustType::Unit, Literal::Null) => {
                    return Ok("()".to_string());
                }
                _ => {}
            }
        }
        
        // Try to emit array literals directly as Vec<T>
        if let (RustType::Vec(inner_type), Expr::Array { elements, .. }) = (target_type, expr) {
            let mut items = Vec::new();
            for elem in elements {
                match elem {
                    ArrayElement::Expr(e) => {
                        let item = self.emit_native_expr(e, inner_type)?;
                        items.push(item);
                    }
                    ArrayElement::Spread(_) => {
                        // Spread not supported in native arrays, fall back
                        let value_expr = self.emit_expr(expr)?;
                        return Ok(target_type.from_value_expr(&value_expr));
                    }
                }
            }
            return Ok(format!("vec![{}]", items.join(", ")));
        }
        
        // Check if the identifier is already of the target type
        if let Expr::Ident { name, .. } = expr {
            let var_type = self.type_context.get_type(name.as_ref());
            if &var_type == target_type {
                return Ok(Self::escape_ident(name.as_ref()).into_owned());
            }
        }
        
        // Fall back to emit_expr + conversion
        let value_expr = self.emit_expr(expr)?;
        Ok(target_type.from_value_expr(&value_expr))
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
            BinOp::Sub => Self::emit_numeric_binop(l, r, "-"),
            BinOp::Mul => Self::emit_numeric_binop(l, r, "*"),
            BinOp::Div => Self::emit_numeric_binop(l, r, "/"),
            BinOp::Mod => Self::emit_numeric_binop(l, r, "%"),
            BinOp::Pow => format!(
                "Value::Number({{ let Value::Number(a) = &({}) else {{ panic!() }}; \
                 let Value::Number(b) = &({}) else {{ panic!() }}; a.powf(*b) }})",
                l, r
            ),
            BinOp::StrictEq => format!("Value::Bool({}.strict_eq(&{}))", l, r),
            BinOp::StrictNe => format!("Value::Bool(!{}.strict_eq(&{}))", l, r),
            BinOp::Lt => Self::emit_numeric_cmp(l, r, "<"),
            BinOp::Le => Self::emit_numeric_cmp(l, r, "<="),
            BinOp::Gt => Self::emit_numeric_cmp(l, r, ">"),
            BinOp::Ge => Self::emit_numeric_cmp(l, r, ">="),
            BinOp::And => format!("Value::Bool({}.is_truthy() && {}.is_truthy())", l, r),
            BinOp::Or => format!("Value::Bool({}.is_truthy() || {}.is_truthy())", l, r),
            BinOp::BitAnd => Self::emit_bitwise_binop(l, r, "&"),
            BinOp::BitOr => Self::emit_bitwise_binop(l, r, "|"),
            BinOp::BitXor => Self::emit_bitwise_binop(l, r, "^"),
            BinOp::Shl => Self::emit_bitwise_binop(l, r, "<<"),
            BinOp::Shr => Self::emit_bitwise_binop(l, r, ">>"),
            BinOp::In => format!("tish_in_operator(&{}, &{})", l, r),
            BinOp::Eq | BinOp::Ne => {
                return Err(CompileError {
                    message: "Loose equality not supported".to_string(),
                })
            }
        })
    }
}
