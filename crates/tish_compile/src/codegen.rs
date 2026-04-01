//! Code generation: AST -> Rust source.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tishlang_ast::{ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern, Expr, FunParam, Literal, LogicalAssignOp, MemberProp, ObjectProp, Program, Span, Statement, UnaryOp};
use crate::resolve::is_builtin_native_spec;
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
            Statement::Import { .. } | Statement::Export { .. } => {
                // Import/Export should be resolved by merge_modules before compilation
            }
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
            Expr::New { callee, args, .. } => {
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
            Expr::LogicalAssign { value, name, .. } => {
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
            Expr::Await { operand, .. } => self.analyze_expr(operand),
            Expr::JsxElement { props, children, .. } => {
                for p in props {
                    match p {
                        tishlang_ast::JsxProp::Attr { value: tishlang_ast::JsxAttrValue::Expr(e), .. } | tishlang_ast::JsxProp::Spread(e) => self.analyze_expr(e),
                        _ => {}
                    }
                }
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        self.analyze_expr(e);
                    }
                }
            }
            Expr::JsxFragment { children, .. } => {
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        self.analyze_expr(e);
                    }
                }
            }
            Expr::NativeModuleLoad { .. } => {}
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
    pub span: Option<Span>,
}

impl CompileError {
    fn new(msg: impl Into<String>, span: Option<Span>) -> Self {
        Self { message: msg.into(), span }
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref span) = self.span {
            write!(f, "{}:{}: {}", span.start.0, span.start.1, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for CompileError {}

fn program_uses_async(program: &Program) -> bool {
    use tishlang_ast::Statement;
    fn stmt_has_async(s: &Statement) -> bool {
        match s {
            Statement::FunDecl { async_, .. } if *async_ => true,
            Statement::Block { statements, .. } => statements.iter().any(stmt_has_async),
            Statement::If { then_branch, else_branch, .. } => {
                stmt_has_async(then_branch) || else_branch.as_ref().is_some_and(|s| stmt_has_async(s.as_ref()))
            }
            Statement::While { body, .. } | Statement::For { body, .. } | Statement::ForOf { body, .. }
            | Statement::DoWhile { body, .. } => stmt_has_async(body),
            Statement::Switch { cases, default_body, .. } => {
                cases.iter().any(|(_, stmts)| stmts.iter().any(stmt_has_async))
                    || default_body
                        .as_ref()
                        .is_some_and(|stmts| stmts.iter().any(stmt_has_async))
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                stmt_has_async(body)
                    || catch_body.as_ref().is_some_and(|s| stmt_has_async(s.as_ref()))
                    || finally_body.as_ref().is_some_and(|s| stmt_has_async(s.as_ref()))
            }
            _ => false,
        }
    }
    fn expr_has_await(e: &Expr) -> bool {
        match e {
            Expr::Await { .. } => true,
            Expr::Binary { left, right, .. } => expr_has_await(left) || expr_has_await(right),
            Expr::Unary { operand, .. } | Expr::TypeOf { operand, .. } => expr_has_await(operand),
            Expr::Call { callee, args, .. } => {
                expr_has_await(callee) || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_has_await(e),
                })
            }
            Expr::New { callee, args, .. } => {
                expr_has_await(callee) || args.iter().any(|a| match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => expr_has_await(e),
                })
            }
            Expr::Member { object, prop, .. } => {
                expr_has_await(object)
                    || if let MemberProp::Expr(e) = prop {
                        expr_has_await(e)
                    } else {
                        false
                    }
            }
            Expr::Index { object, index, .. } => expr_has_await(object) || expr_has_await(index),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                expr_has_await(cond) || expr_has_await(then_branch) || expr_has_await(else_branch)
            }
            Expr::NullishCoalesce { left, right, .. } => expr_has_await(left) || expr_has_await(right),
            Expr::Array { elements, .. } => elements.iter().any(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_has_await(e),
            }),
            Expr::Object { props, .. } => props.iter().any(|p| match p {
                ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => expr_has_await(e),
            }),
            Expr::Assign { value, .. } | Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. }
            | Expr::MemberAssign { value, .. } | Expr::IndexAssign { value, .. } => expr_has_await(value),
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => expr_has_await(e),
                ArrowBody::Block(s) => stmt_has_async(s),
            },
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(expr_has_await),
            Expr::JsxElement { props, children, .. } => {
                props.iter().any(|p| match p {
                    tishlang_ast::JsxProp::Attr { value: tishlang_ast::JsxAttrValue::Expr(e), .. } | tishlang_ast::JsxProp::Spread(e) => expr_has_await(e),
                    _ => false,
                }) || children.iter().any(|c| matches!(c, tishlang_ast::JsxChild::Expr(e) if expr_has_await(e)))
            }
            Expr::JsxFragment { children, .. } => {
                children.iter().any(|c| matches!(c, tishlang_ast::JsxChild::Expr(e) if expr_has_await(e)))
            }
            _ => false,
        }
    }
    fn stmt_has_await(s: &Statement) -> bool {
        match s {
            Statement::Block { statements, .. } => statements.iter().any(stmt_has_await),
            Statement::VarDecl { init, .. } => init.as_ref().is_some_and(expr_has_await),
            Statement::VarDeclDestructure { init, .. } => expr_has_await(init),
            Statement::ExprStmt { expr, .. } => expr_has_await(expr),
            Statement::If { cond, then_branch, else_branch, .. } => {
                expr_has_await(cond) || stmt_has_await(then_branch)
                    || else_branch.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
            }
            Statement::While { cond, body, .. } => expr_has_await(cond) || stmt_has_await(body),
            Statement::For { init, cond, update, body, .. } => {
                init.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
                    || cond.as_ref().is_some_and(expr_has_await)
                    || update.as_ref().is_some_and(expr_has_await)
                    || stmt_has_await(body)
            }
            Statement::ForOf { iterable, body, .. } => expr_has_await(iterable) || stmt_has_await(body),
            Statement::Return { value, .. } => value.as_ref().is_some_and(expr_has_await),
            Statement::FunDecl { body, .. } => stmt_has_await(body),
            Statement::Switch { expr, cases, default_body, .. } => {
                expr_has_await(expr)
                    || cases.iter().any(|(c, stmts)| {
                        c.as_ref().is_some_and(expr_has_await) || stmts.iter().any(stmt_has_await)
                    })
                    || default_body
                        .as_ref()
                        .is_some_and(|stmts| stmts.iter().any(stmt_has_await))
            }
            Statement::DoWhile { body, cond, .. } => stmt_has_await(body) || expr_has_await(cond),
            Statement::Throw { value, .. } => expr_has_await(value),
            Statement::Try { body, catch_body, finally_body, .. } => {
                stmt_has_await(body)
                    || catch_body.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
                    || finally_body.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
            }
            Statement::Import { .. } | Statement::Export { .. } => false,
            _ => false,
        }
    }
    program.statements.iter().any(|s| stmt_has_async(s) || stmt_has_await(s))
}

pub fn compile(program: &Program) -> Result<String, CompileError> {
    compile_with_project_root(program, None)
}

pub fn compile_with_project_root(program: &Program, project_root: Option<&Path>) -> Result<String, CompileError> {
    compile_with_features(program, project_root, &[])
}

/// Compile a project from its entry path. Resolves imports, merges modules, then compiles.
/// Features are derived from native imports (e.g. import { egui } from 'tish:egui') and merged
/// with any explicitly passed features. Returns only the Rust code (backward compatible).
pub fn compile_project(
    entry_path: &Path,
    project_root: Option<&Path>,
    features: &[String],
) -> Result<String, CompileError> {
    let (rust, _) = compile_project_full(entry_path, project_root, features, true)?;
    Ok(rust)
}

/// Compile a project and return Rust code plus resolved native modules for Cargo.toml generation.
pub fn compile_project_full(
    entry_path: &Path,
    project_root: Option<&Path>,
    features: &[String],
    optimize: bool,
) -> Result<(String, Vec<crate::resolve::ResolvedNativeModule>), CompileError> {
    use crate::resolve;
    let root = project_root.unwrap_or_else(|| entry_path.parent().unwrap_or(Path::new(".")));
    let modules = resolve::resolve_project(entry_path, project_root)
        .map_err(|e| CompileError { message: e, span: None })?;
    resolve::detect_cycles(&modules)
        .map_err(|e| CompileError { message: e, span: None })?;
    let merged = resolve::merge_modules(modules)
        .map_err(|e| CompileError { message: e, span: None })?;
    let native_modules = resolve::resolve_native_modules(&merged, root)
        .map_err(|e| CompileError { message: e, span: None })?;
    let mut all_features: Vec<String> = features.to_vec();
    for f in resolve::extract_native_import_features(&merged) {
        if !all_features.contains(&f) {
            all_features.push(f);
        }
    }
    let rust = compile_with_native_modules(&merged, project_root, &all_features, &native_modules, optimize)?;
    Ok((rust, native_modules))
}

/// Compile with explicit feature flags. When features are provided, codegen uses them
/// to emit builtins (process, serve, etc.) regardless of tishlang_compile's #[cfg] build.
pub fn compile_with_features(
    program: &Program,
    project_root: Option<&Path>,
    features: &[String],
) -> Result<String, CompileError> {
    compile_with_native_modules(program, project_root, features, &[], true)
}

/// Compile with resolved native modules. Native imports emit calls to the module crates directly.
pub fn compile_with_native_modules(
    program: &Program,
    project_root: Option<&Path>,
    features: &[String],
    native_modules: &[crate::resolve::ResolvedNativeModule],
    optimize: bool,
) -> Result<String, CompileError> {
    let program = if optimize { tishlang_opt::optimize(program) } else { program.clone() };
    // Type-inference pass: fills in `type_ann` on unannotated VarDecl nodes where
    // the type is unambiguous (literals, arithmetic of typed vars, etc.).
    let program = crate::infer::infer_program(&program);
    let map: std::collections::HashMap<String, (String, String)> = native_modules
        .iter()
        .map(|m| (m.spec.clone(), (m.crate_name.clone(), m.export_fn.clone())))
        .collect();
    let mut g = Codegen::new_with_native_modules(project_root, features, map);
    g.emit_program(&program)?;
    Ok(g.output)
}

struct Codegen {
    output: String,
    indent: usize,
    loop_label_index: usize,
    is_async: bool,
    project_root: Option<std::path::PathBuf>,
    /// Requested features (http, process, fs, regex, polars). When non-empty, used instead of #[cfg].
    features: std::collections::HashSet<String>,
    /// spec -> (crate_name, export_fn) for native modules resolved via package.json
    native_module_map: std::collections::HashMap<String, (String, String)>,
    /// Stack: true = async Rust context (run body), false = sync closure (Tish fn body)
    async_context_stack: Vec<bool>,
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
    /// Program uses JSX; emit `tishlang_ui` imports and `h` / `Fragment` globals.
    program_has_jsx: bool,
}

impl Codegen {
    fn new_with_native_modules(
        project_root: Option<&Path>,
        features: &[String],
        native_module_map: std::collections::HashMap<String, (String, String)>,
    ) -> Self {
        let features: std::collections::HashSet<String> = features.iter().cloned().collect();
        Self {
            output: String::new(),
            indent: 0,
            loop_label_index: 0,
            is_async: false,
            project_root: project_root.map(|p| p.to_path_buf()),
            features,
            native_module_map,
            async_context_stack: Vec::new(),
            loop_stack: Vec::new(),
            function_scope_stack: vec![Vec::new()], // Start with global scope
            outer_params_stack: Vec::new(),
            outer_vars_stack: vec![Vec::new()], // Start with module-level scope
            refcell_wrapped_vars: std::collections::HashSet::new(),
            usage_analyzer: None,
            type_context: TypeContext::new(),
            program_has_jsx: false,
        }
    }

    /// Map native module spec to Rust init expression using resolved package.json modules.
    /// For built-in modules (tish:fs, tish:http, tish:process), use builtin_native_module_rust_init.
    fn native_module_rust_init(&self, spec: &str, export_name: &str) -> Option<String> {
        if is_builtin_native_spec(spec) {
            return self.builtin_native_module_rust_init(spec, export_name);
        }
        self.native_module_map.get(spec).map(|(crate_name, export_fn)| {
            // Native modules return a namespace object (like an ES module).
            // Named imports extract the field from that namespace: `import { foo } from "pkg"` → `ns.foo`.
            format!(
                "{{ let _ns = {}::{}(); match _ns {{ Value::Object(ref _o) => _o.borrow().get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }} }}",
                crate_name, export_fn, export_name
            )
        })
    }

    /// Rust init for built-in modules (tish:fs, tish:http, tish:process) - uses tishlang_runtime.
    fn builtin_native_module_rust_init(&self, spec: &str, export_name: &str) -> Option<String> {
        let init = match spec {
            "tish:fs" if self.has_feature("fs") => match export_name {
                    "readFile" => Some("Value::Function(Rc::new(|args: &[Value]| tish_read_file(args)))"),
                    "writeFile" => Some("Value::Function(Rc::new(|args: &[Value]| tish_write_file(args)))"),
                    "fileExists" => Some("Value::Function(Rc::new(|args: &[Value]| tish_file_exists(args)))"),
                    "isDir" => Some("Value::Function(Rc::new(|args: &[Value]| tish_is_dir(args)))"),
                    "readDir" => Some("Value::Function(Rc::new(|args: &[Value]| tish_read_dir(args)))"),
                    "mkdir" => Some("Value::Function(Rc::new(|args: &[Value]| tish_mkdir(args)))"),
                    _ => None,
                },
            "tish:http" if self.has_feature("http") => match export_name {
                    "fetch" => Some("Value::Function(Rc::new(|args: &[Value]| tish_fetch_promise(args.to_vec())))"),
                    "fetchAll" => Some("Value::Function(Rc::new(|args: &[Value]| tish_fetch_all_promise(args.to_vec())))"),
                    "serve" => Some("Value::Function(Rc::new(|args: &[Value]| { let port = args.first().cloned().unwrap_or(Value::Null); let handler = args.get(1).cloned().unwrap_or(Value::Null); if let Value::Function(f) = handler { tish_http_serve(args, move |req_args| f(req_args)) } else { Value::Null } }))"),
                    "Promise" => Some("tish_promise_object()"),
                    "setTimeout" => Some("Value::Function(Rc::new(|args: &[Value]| tish_timer_set_timeout(args)))"),
                    "setInterval" => Some("Value::Function(Rc::new(|_args: &[Value]| panic!(\"setInterval not yet supported in native\")))"),
                    "clearTimeout" => Some("Value::Function(Rc::new(|args: &[Value]| tish_timer_clear_timeout(args)))"),
                    "clearInterval" => Some("Value::Function(Rc::new(|_args: &[Value]| Value::Null))"),
                    _ => None,
                },
            "tish:process" if self.has_feature("process") => match export_name {
                    "exit" => Some("Value::Function(Rc::new(|args: &[Value]| tish_process_exit(args)))"),
                    "cwd" => Some("Value::Function(Rc::new(|args: &[Value]| tish_process_cwd(args)))"),
                    "exec" => Some("Value::Function(Rc::new(|args: &[Value]| tish_process_exec(args)))"),
                    "argv" => Some("Value::Array(Rc::new(RefCell::new(std::env::args().map(|s| Value::String(s.into())).collect())))"),
                    "env" => Some("Value::Object(Rc::new(RefCell::new(std::env::vars().map(|(k,v)| (Arc::from(k.as_str()), Value::String(v.into()))).collect())))"),
                    "process" => Some("{ let mut m = ObjectMap::default(); m.insert(Arc::from(\"exit\"), Value::Function(Rc::new(|args: &[Value]| tish_process_exit(args)))); m.insert(Arc::from(\"cwd\"), Value::Function(Rc::new(|args: &[Value]| tish_process_cwd(args)))); m.insert(Arc::from(\"exec\"), Value::Function(Rc::new(|args: &[Value]| tish_process_exec(args)))); m.insert(Arc::from(\"argv\"), Value::Array(Rc::new(RefCell::new(std::env::args().map(|s| Value::String(s.into())).collect())))); m.insert(Arc::from(\"env\"), Value::Object(Rc::new(RefCell::new(std::env::vars().map(|(k,v)| (Arc::from(k.as_str()), Value::String(v.into()))).collect::<ObjectMap>())))); Value::Object(Rc::new(RefCell::new(m))) }"),
                    _ => None,
                },
            "tish:ws" if self.has_feature("ws") => match export_name {
                    "WebSocket" => Some("Value::Function(Rc::new(|args: &[Value]| tish_ws_client(args)))"),
                    "Server" => Some("Value::Function(Rc::new(|args: &[Value]| tish_ws_server_construct(args)))"),
                    "wsSend" => Some("Value::Function(Rc::new(|args: &[Value]| Value::Bool(tishlang_runtime::ws_send_native(args.first().unwrap_or(&Value::Null), &args.get(1).map(|v| v.to_display_string()).unwrap_or_default()))))"),
                    "wsBroadcast" => Some("Value::Function(Rc::new(|args: &[Value]| tishlang_runtime::ws_broadcast_native(args)))"),
                    _ => None,
                },
            _ => return None,
        };
        init.map(String::from)
    }

    fn has_feature(&self, name: &str) -> bool {
        if self.features.is_empty() {
            #[cfg(feature = "process")]
            if name == "process" {
                return true;
            }
            #[cfg(feature = "http")]
            if name == "http" {
                return true;
            }
            #[cfg(feature = "fs")]
            if name == "fs" {
                return true;
            }
            #[cfg(feature = "regex")]
            if name == "regex" {
                return true;
            }
            #[cfg(feature = "ws")]
            if name == "ws" {
                return true;
            }
            false
        } else {
            self.features.contains(name)
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
        // Rust standard library macros that conflict with variable names
        const RUST_MACROS: &[&str] = &["line", "column", "file", "module_path", "stringify", "concat"];
        if RUST_MACROS.contains(&name) {
            return Cow::Owned(format!("r#{}", name));
        }
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
                | Expr::New { .. }
                | Expr::Await { .. }
                | Expr::ArrowFunction { .. }
                | Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::TypeOf { .. }
                | Expr::TemplateLiteral { .. }
                | Expr::JsxElement { .. }
                | Expr::JsxFragment { .. }
                | Expr::NativeModuleLoad { .. }
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
            
            // Inside a loop, any variable used in an init (e.g. "let x = outerVar") must be cloned:
            // the loop body runs multiple times, so we cannot move on the first iteration.
            if !self.loop_stack.is_empty() {
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

    /// Generate code for increment/decrement operations.
    /// `is_prefix`: true for ++x/--x, false for x++/x--
    /// `delta`: "+1.0" or "-1.0"
    /// `op_name`: "++" or "--" for error message
    fn emit_inc_dec(&self, name: &str, is_prefix: bool, delta: &str, op_name: &str) -> String {
        let n = Self::escape_ident(name);
        let is_wrapped = self.refcell_wrapped_vars.contains(name);
        let var_type = self.type_context.get_type(name);

        // Native fast path: f64 variable → avoid boxing/unboxing.
        if !is_wrapped && var_type == RustType::F64 {
            let op_assign = if delta.contains('+') { "+=" } else { "-=" };
            return if is_prefix {
                format!("{{ {n} {op_assign} 1.0_f64; Value::Number({n}) }}")
            } else {
                format!("{{ let _prev = {n}; {n} {op_assign} 1.0_f64; Value::Number(_prev) }}")
            };
        }

        if is_prefix {
            if is_wrapped {
                format!(
                    "{{ *{n}.borrow_mut() = Value::Number(match &*{n}.borrow() {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); (*{n}.borrow()).clone() }}"
                )
            } else {
                format!(
                    "{{ {n} = Value::Number(match &{n} {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); {n}.clone() }}"
                )
            }
        } else if is_wrapped {
            format!(
                "{{ let _v = (*{n}.borrow()).clone(); *{n}.borrow_mut() = Value::Number(match &_v {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); _v }}"
            )
        } else {
            format!(
                "{{ let _v = {n}.clone(); {n} = Value::Number(match &_v {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); _v }}"
            )
        }
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
        use tishlang_ast::ArrowBody;
        
        if let Expr::ArrowFunction { params, body, .. } = expr {
            if params.len() != 2 {
                return None;
            }
            let (param_a, param_b) = match (&params[0], &params[1]) {
                (FunParam::Simple(a), FunParam::Simple(b))
                    if a.default.is_none() && b.default.is_none() =>
                {
                    (a.name.as_ref(), b.name.as_ref())
                }
                _ => return None,
            };
            
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
        self.is_async = program_uses_async(program);
        self.program_has_jsx = tishlang_ui::jsx::program_contains_jsx(program);
        self.write("#![allow(unused, non_snake_case)]\n\n");
        self.write("use std::cell::RefCell;\n");
        self.write("use std::rc::Rc;\n");
        self.write("use std::sync::Arc;\n");
        self.write("use tishlang_runtime::{console_debug as tish_console_debug, console_info as tish_console_info, console_log as tish_console_log, console_warn as tish_console_warn, console_error as tish_console_error, boolean as tish_boolean, decode_uri as tish_decode_uri, encode_uri as tish_encode_uri, in_operator as tish_in_operator, is_finite as tish_is_finite, is_nan as tish_is_nan, json_parse as tish_json_parse, json_stringify as tish_json_stringify, math_abs as tish_math_abs, math_ceil as tish_math_ceil, math_floor as tish_math_floor, math_max as tish_math_max, math_min as tish_math_min, math_round as tish_math_round, math_sqrt as tish_math_sqrt, parse_float as tish_parse_float, parse_int as tish_parse_int, math_random as tish_math_random, math_pow as tish_math_pow, math_sin as tish_math_sin, math_cos as tish_math_cos, math_tan as tish_math_tan, math_log as tish_math_log, math_exp as tish_math_exp, math_sign as tish_math_sign, math_trunc as tish_math_trunc, date_now as tish_date_now, array_is_array as tish_array_is_array, string_from_char_code as tish_string_from_char_code, object_assign as tish_object_assign, object_keys as tish_object_keys, object_values as tish_object_values, object_entries as tish_object_entries, object_from_entries as tish_object_from_entries, tish_construct, tish_uint8_array_constructor, tish_audio_context_constructor, ObjectMap, TishError, Value};\n");
        if self.program_has_jsx {
            self.write("use tishlang_ui::{fragment_value, install_thread_local_host, native_create_root, native_use_state, ui_h, ui_text, HeadlessHost};\n");
        }
        if self.has_feature("process") {
            self.write("use tishlang_runtime::{process_exit as tish_process_exit, process_cwd as tish_process_cwd, process_exec as tish_process_exec};\n");
        }
        if self.has_feature("http") {
            if self.is_async {
                self.write("use tishlang_runtime::{fetch_promise as tish_fetch_promise, fetch_all_promise as tish_fetch_all_promise, http_serve as tish_http_serve, timer_set_timeout as tish_timer_set_timeout, timer_clear_timeout as tish_timer_clear_timeout, promise_object as tish_promise_object, await_promise as tish_await_promise};\n");
            } else {
                self.write("use tishlang_runtime::{fetch_promise as tish_fetch_promise, fetch_all_promise as tish_fetch_all_promise, http_serve as tish_http_serve};\n");
            }
        }
        if self.has_feature("fs") {
            self.write("use tishlang_runtime::{read_file as tish_read_file, write_file as tish_write_file, file_exists as tish_file_exists, is_dir as tish_is_dir, read_dir as tish_read_dir, mkdir as tish_mkdir};\n");
        }
        if self.has_feature("ws") {
            self.write("use tishlang_runtime::{web_socket_client as tish_ws_client, web_socket_server_construct as tish_ws_server_construct};\n");
        }
        if self.has_feature("regex") {
            self.write("use tishlang_runtime::regexp_new;\n");
        }
        self.write("\n");

        if self.is_async {
            self.writeln("#[tokio::main]");
            self.writeln("async fn main() {");
        } else {
            self.writeln("fn main() {");
        }
        self.indent += 1;
        if self.is_async {
            self.writeln("if let Err(e) = run().await {");
        } else {
            self.writeln("if let Err(e) = run() {");
        }
        self.indent += 1;
        self.writeln("eprintln!(\"Error: {}\", e);");
        self.writeln("std::process::exit(1);");
        self.indent -= 1;
        self.writeln("}");
        self.indent -= 1;
        self.writeln("}");
        self.writeln("");
        if self.is_async {
            self.writeln("async fn run() -> Result<(), Box<dyn std::error::Error>> {");
        } else {
            self.writeln("fn run() -> Result<(), Box<dyn std::error::Error>> {");
        }
        self.indent += 1;

        // Initialize builtins
        self.writeln("let mut console = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"debug\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_debug(args); Value::Null }))),");
        self.writeln("(Arc::from(\"info\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_info(args); Value::Null }))),");
        self.writeln("(Arc::from(\"log\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_log(args); Value::Null }))),");
        self.writeln("(Arc::from(\"warn\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_warn(args); Value::Null }))),");
        self.writeln("(Arc::from(\"error\"), Value::Function(Rc::new(|args: &[Value]| { tish_console_error(args); Value::Null }))),");
        self.indent -= 1;
        self.writeln("]))));");
        self.writeln("let Boolean = Value::Function(Rc::new(|args: &[Value]| tish_boolean(args)));");
        self.writeln("let parseInt = Value::Function(Rc::new(|args: &[Value]| tish_parse_int(args)));");
        self.writeln("let parseFloat = Value::Function(Rc::new(|args: &[Value]| tish_parse_float(args)));");
        self.writeln("let decodeURI = Value::Function(Rc::new(|args: &[Value]| tish_decode_uri(args)));");
        self.writeln("let encodeURI = Value::Function(Rc::new(|args: &[Value]| tish_encode_uri(args)));");
        self.writeln("let isFinite = Value::Function(Rc::new(|args: &[Value]| tish_is_finite(args)));");
        self.writeln("let isNaN = Value::Function(Rc::new(|args: &[Value]| tish_is_nan(args)));");
        self.writeln("let Infinity = Value::Number(f64::INFINITY);");
        self.writeln("let NaN = Value::Number(f64::NAN);");
        self.writeln("let Math = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
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
        self.writeln("let JSON = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"parse\"), Value::Function(Rc::new(|args: &[Value]| tish_json_parse(args)))),");
        self.writeln("(Arc::from(\"stringify\"), Value::Function(Rc::new(|args: &[Value]| tish_json_stringify(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Array = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"isArray\"), Value::Function(Rc::new(|args: &[Value]| tish_array_is_array(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let String = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"fromCharCode\"), Value::Function(Rc::new(|args: &[Value]| tish_string_from_char_code(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Date = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"now\"), Value::Function(Rc::new(|args: &[Value]| tish_date_now(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Object = Value::Object(Rc::new(RefCell::new(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"assign\"), Value::Function(Rc::new(|args: &[Value]| tish_object_assign(args)))),");
        self.writeln("(Arc::from(\"keys\"), Value::Function(Rc::new(|args: &[Value]| tish_object_keys(args)))),");
        self.writeln("(Arc::from(\"values\"), Value::Function(Rc::new(|args: &[Value]| tish_object_values(args)))),");
        self.writeln("(Arc::from(\"entries\"), Value::Function(Rc::new(|args: &[Value]| tish_object_entries(args)))),");
        self.writeln("(Arc::from(\"fromEntries\"), Value::Function(Rc::new(|args: &[Value]| tish_object_from_entries(args)))),");
        self.indent -= 1;
        self.writeln("]))));");

        self.writeln("let Uint8Array = tish_uint8_array_constructor();");
        self.writeln("let AudioContext = tish_audio_context_constructor();");

        if self.has_feature("process") {
            self.writeln("let process = Value::Object(Rc::new(RefCell::new({");
            self.indent += 1;
            self.writeln("let mut p = ObjectMap::default();");
            self.writeln("p.insert(Arc::from(\"exit\"), Value::Function(Rc::new(|args: &[Value]| tish_process_exit(args))));");
            self.writeln("p.insert(Arc::from(\"cwd\"), Value::Function(Rc::new(|args: &[Value]| tish_process_cwd(args))));");
            self.writeln("p.insert(Arc::from(\"exec\"), Value::Function(Rc::new(|args: &[Value]| tish_process_exec(args))));");
            self.writeln("let argv: Vec<Value> = std::env::args().map(|s| Value::String(s.into())).collect();");
            self.writeln("p.insert(Arc::from(\"argv\"), Value::Array(Rc::new(RefCell::new(argv))));");
            self.writeln("let mut env_obj = ObjectMap::default();");
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

        if self.has_feature("http") {
            self.writeln("let fetch = Value::Function(Rc::new(|args: &[Value]| tish_fetch_promise(args.to_vec())));");
            self.writeln("let fetchAll = Value::Function(Rc::new(|args: &[Value]| tish_fetch_all_promise(args.to_vec())));");
            if self.is_async {
                self.writeln("let setTimeout = Value::Function(Rc::new(|args: &[Value]| tish_timer_set_timeout(args)));");
                self.writeln("let clearTimeout = Value::Function(Rc::new(|args: &[Value]| tish_timer_clear_timeout(args)));");
                self.writeln("let Promise = tish_promise_object();");
            }
            self.writeln("let serve = Value::Function(Rc::new(|args: &[Value]| {");
            self.indent += 1;
            self.writeln("let port = args.first().cloned().unwrap_or(Value::Null);");
            self.writeln("let handler = args.get(1).cloned().unwrap_or(Value::Null);");
            self.writeln("if let Value::Function(f) = handler {");
            self.indent += 1;
            self.writeln("tish_http_serve(args, move |req_args| f(req_args))");
            self.indent -= 1;
            self.writeln("} else {");
            self.indent += 1;
            self.writeln("Value::Null");
            self.indent -= 1;
            self.writeln("}");
            self.indent -= 1;
            self.writeln("}));");
        }

        if self.has_feature("fs") {
            self.writeln("let readFile = Value::Function(Rc::new(|args: &[Value]| tish_read_file(args)));");
            self.writeln("let writeFile = Value::Function(Rc::new(|args: &[Value]| tish_write_file(args)));");
            self.writeln("let fileExists = Value::Function(Rc::new(|args: &[Value]| tish_file_exists(args)));");
            self.writeln("let isDir = Value::Function(Rc::new(|args: &[Value]| tish_is_dir(args)));");
            self.writeln("let readDir = Value::Function(Rc::new(|args: &[Value]| tish_read_dir(args)));");
            self.writeln("let mkdir = Value::Function(Rc::new(|args: &[Value]| tish_mkdir(args)));");
        }

        if self.has_feature("regex") {
            self.writeln("let RegExp = Value::Function(Rc::new(|args: &[Value]| regexp_new(args)));");
        }

        if self.program_has_jsx {
            self.writeln("install_thread_local_host(Box::new(HeadlessHost::default()));");
            self.writeln("let Fragment = fragment_value();");
            self.writeln("let h = Value::Function(Rc::new(|args: &[Value]| ui_h(args)));");
            self.writeln("let text = Value::Function(Rc::new(|args: &[Value]| ui_text(args)));");
            self.writeln("let useState = Value::Function(Rc::new(|args: &[Value]| native_use_state(args)));");
            self.writeln("let createRoot = Value::Function(Rc::new(|args: &[Value]| native_create_root(args)));");
        }

        // Polars, Egui etc. are emitted via VarDecl from import { X } from 'tish:...'

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

        // Prepass: vars mutated by nested closures must be RefCell from the start (top-level)
        let top_level_mutated = Self::collect_vars_mutated_by_nested_closures(&program.statements);
        for v in &top_level_mutated {
            self.refcell_wrapped_vars.insert(v.clone());
        }

        if self.is_async {
            self.async_context_stack.push(true); // run() body is async Rust context
        }
        for stmt in &program.statements {
            self.emit_statement(stmt)?;
        }
        if self.is_async {
            self.async_context_stack.pop();
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
                self.type_context.push_scope();
                self.outer_vars_stack.push(Vec::new());
                // Prepass: vars that must be RefCell because nested closures capture and mutate them
                let vars_mutated_by_nested = Self::collect_vars_mutated_by_nested_closures(statements);
                for v in &vars_mutated_by_nested {
                    self.refcell_wrapped_vars.insert(v.clone());
                }
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
                self.outer_vars_stack.pop(); // Exit variable scope
                for v in &vars_mutated_by_nested {
                    self.refcell_wrapped_vars.remove(v);
                }
                self.type_context.pop_scope();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::VarDecl { name, mutable, type_ann, init, .. } => {
                // Determine the Rust type from annotation
                let rust_type = type_ann
                    .as_ref()
                    .map(RustType::from_annotation)
                    .unwrap_or(RustType::Value);

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
                            // Variable refs (Ident) in init must always clone: they may be used
                            // multiple times (e.g. in a loop body) and we cannot move.
                            let needs = matches!(e, Expr::Ident { .. })
                                || self.should_clone(e);
                            (s, needs)
                        }
                        None => ("Value::Null".to_string(), false),
                    };
                    // Vars that are mutated by nested closures must be RefCell from the start
                    if self.refcell_wrapped_vars.contains(name.as_ref()) {
                        let init_val = if clone_needed {
                            format!("({}).clone()", expr_str)
                        } else {
                            expr_str.to_string()
                        };
                        self.writeln(&format!("let {} = std::rc::Rc::new(RefCell::new({}));", escaped_name, init_val));
                    } else if clone_needed {
                        self.writeln(&format!("{} {} = ({}).clone();", mutability, escaped_name, expr_str));
                    } else {
                        self.writeln(&format!("{} {} = {};", mutability, escaped_name, expr_str));
                    }
                }
                
                if let Some(scope) = self.outer_vars_stack.last_mut() {
                    scope.push(name.to_string());
                }
            }
            Statement::VarDeclDestructure { pattern, mutable, init, span, .. } => {
                let expr = self.emit_expr(init)?;
                let mutability = if *mutable { "let mut" } else { "let" };
                let clone_suffix = if Self::needs_clone(init) { ".clone()" } else { "" };
                self.writeln(&format!("let _destruct_val = ({}){};", expr, clone_suffix));
                self.emit_destruct_bindings(pattern, "_destruct_val", mutability, *span)?;
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
                let c = self.emit_cond_expr(cond)?;
                self.write(&format!("if {} {{\n", c));
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
                let c = self.emit_cond_expr(cond)?;
                let label = format!("'while_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.write(&format!("{}: while {} {{\n", label, c));
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
                    .map(|c| self.emit_cond_expr(c).unwrap())
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
            Statement::Import { .. } | Statement::Export { .. } => {
                return Err(CompileError {
                    message: "Import/Export should be resolved before compilation (use compile_project for multi-file projects)".to_string(),
                    span: None,
                });
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
                let c = self.emit_cond_expr(cond)?;
                let label = format!("'dowhile_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.write(&format!("{}: loop {{\n", label));
                self.indent += 1;
                self.emit_statement(body)?;
                self.write(&format!("if !{} {{ break; }}\n", c));
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Throw { value, .. } => {
                let v = self.emit_expr(value)?;
                self.writeln(&format!(
                    "return Err(Box::new(tishlang_runtime::TishError::Throw({})) as Box<dyn std::error::Error>);",
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
                        self.writeln("match e.downcast::<tishlang_runtime::TishError>() {");
                        self.indent += 1;
                        self.writeln("Ok(tish_err) => {");
                        self.indent += 1;
                        self.writeln("if let tishlang_runtime::TishError::Throw(v) = *tish_err {");
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
            Statement::FunDecl { name, params, rest_param, body, span, .. } => {
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
                let param_names: HashSet<String> = params
                    .iter()
                    .flat_map(|p| p.bound_names())
                    .map(|n| n.to_string())
                    .collect();
                
                // Collect all outer parameters that need to be captured (only those referenced)
                let outer_params: Vec<String> = self.outer_params_stack
                    .iter()
                    .flat_map(|p| p.iter().cloned())
                    .filter(|name| referenced.contains(name) && !param_names.contains(name))
                    .collect();
                // Collect outer variables (from outer_vars_stack) - wrap in RefCell for mutable capture
                // Exclude params and variables declared in this function's body (locals)
                let mut local_var_names = HashSet::new();
                Self::collect_local_var_names(body, &mut local_var_names);
                let outer_vars: Vec<String> = self.outer_vars_stack
                    .iter()
                    .flat_map(|v| v.iter().cloned())
                    .filter(|name| referenced.contains(name) && !param_names.contains(name) && !local_var_names.contains(name))
                    .filter(|name| !["Boolean", "console", "Math", "JSON", "Date", "process", "setTimeout", "clearTimeout", "Promise", "RegExp", "Polars"].contains(&name.as_str()))
                    .collect();

                // Outer vars that are assigned in the body need RefCell (capture cell, add to refcell_wrapped_vars).
                // Read-only outer vars get a Value binding to avoid nested_complex param-shadow issues.
                let mut assigned_in_body = HashSet::new();
                Self::collect_assigned_idents_in_stmt(body, &mut assigned_in_body);
                let mutable_outer_vars: Vec<String> = outer_vars
                    .iter()
                    .filter(|v| assigned_in_body.contains(*v))
                    .cloned()
                    .collect();
                let read_only_outer_vars: Vec<String> = outer_vars
                    .iter()
                    .filter(|v| !assigned_in_body.contains(*v))
                    .cloned()
                    .collect();

                // Rebind outer vars to Rc<RefCell<>> with _cell suffix.
                // If outer scope already has the var as RefCell, just clone it.
                for outer_var in &outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    if self.refcell_wrapped_vars.contains(outer_var) {
                        self.writeln(&format!("let {}_cell = {}.clone();", var_escaped, var_escaped));
                    } else {
                        self.writeln(&format!("let {}_cell = std::rc::Rc::new(RefCell::new({}.clone()));", var_escaped, var_escaped));
                    }
                }

                self.writeln(&format!("let {} = {{", name_str));
                self.indent += 1;
                // Clone RefCell for outer vars so closure can capture
                for outer_var in &outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!("let {}_cell = {}_cell.clone();", var_escaped, var_escaped));
                }
                // Clone the cell so the closure can reference the function recursively
                let needs_self_ref = referenced.contains(name_raw);
                if needs_self_ref {
                    self.writeln(&format!("let {}_ref = {}_cell.clone();", name_str, name_str));
                }
                // Clone sibling function cells for mutual recursion
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
                // Only clone builtins that are actually referenced (clone so outer scope can still use them, e.g. process for PORT before serve)
                for builtin in &["Boolean", "console", "Math", "JSON", "Date", "Uint8Array", "AudioContext", "process", "setTimeout", "clearTimeout", "Promise", "RegExp", "Polars"] {
                    if referenced.contains(*builtin) {
                        self.writeln(&format!("let {} = {}.clone();", builtin, builtin));
                    }
                }
                self.writeln("Value::Function(Rc::new(move |args: &[Value]| {");
                self.indent += 1;
                // Mutable outer vars: capture the RefCell so assignments use borrow_mut
                for outer_var in &mutable_outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!("let {} = {}_cell.clone();", var_escaped, var_escaped));
                }
                // Read-only outer vars: Value binding from borrow (avoids param-shadow issues)
                for outer_var in &read_only_outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!("let {} = (*{}_cell.borrow()).clone();", var_escaped, var_escaped));
                }
                // Make the function available by its name inside the closure (only if recursive)
                if needs_self_ref {
                    self.writeln(&format!("let {} = (*{}_ref.borrow()).clone();", name_str, name_str));
                }
                // Make sibling functions available for mutual recursion
                for sibling in &sibling_fns {
                    let sibling_escaped = Self::escape_ident(sibling);
                    self.writeln(&format!("let {} = (*{}_ref.borrow()).clone();", sibling_escaped, sibling_escaped));
                }
                // Extract just the parameter names (type annotations are parsed but not used in codegen yet)
                let current_param_names: Vec<String> = params
                    .iter()
                    .flat_map(|p| p.bound_names())
                    .map(|n| n.to_string())
                    .collect();
                let formal_span = *span;
                for (i, p) in params.iter().enumerate() {
                    match p {
                        FunParam::Simple(tp) => {
                            self.writeln(&format!(
                                "let mut {} = args.get({}).cloned().unwrap_or(Value::Null);",
                                Self::escape_ident(tp.name.as_ref()),
                                i
                            ));
                        }
                        FunParam::Destructure { pattern, .. } => {
                            let tmp = format!("_formal_{}", i);
                            self.writeln(&format!(
                                "let {} = args.get({}).cloned().unwrap_or(Value::Null);",
                                tmp, i
                            ));
                            self.emit_destruct_bindings(pattern, &tmp, "let mut", formal_span)?;
                        }
                    }
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
                
                // Function bodies are sync closures (even Tish async fn) - use block_on for await
                self.async_context_stack.push(false);

                // Mutable outer vars must be in refcell_wrapped_vars so Assign/CompoundAssign emit borrow_mut
                let saved_refcell = self.refcell_wrapped_vars.clone();
                for v in &mutable_outer_vars {
                    self.refcell_wrapped_vars.insert(v.clone());
                }
                
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
                
                self.async_context_stack.pop();

                // Restore refcell_wrapped_vars (remove mutable outer vars we added)
                self.refcell_wrapped_vars = saved_refcell;
                
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
                    if let CallArg::Spread(e) = arg {
                        return Err(CompileError::new("Unexpected spread", Some(e.span())));
                    }
                    unreachable!("else branch only reached for Spread");
                }
            }
            Ok(format!("vec![{}]", emitted.join(", ")))
        }
    }

    fn emit_destruct_bindings(&mut self, pattern: &DestructPattern, value_expr: &str, mutability: &str, span: Span) -> Result<(), CompileError> {
        // Flat `let` bindings so names stay in scope for the rest of the function (e.g. JSX).
        match pattern {
            DestructPattern::Array(elements) => {
                for (i, elem) in elements.iter().enumerate() {
                    if let Some(el) = elem {
                        match el {
                            DestructElement::Ident(name) => {
                                self.writeln(&format!(
                                    "{} {} = match &({}) {{ Value::Array(ref _a) => _a.borrow().get({}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                    mutability,
                                    Self::escape_ident(name.as_ref()),
                                    value_expr,
                                    i
                                ));
                            }
                            DestructElement::Pattern(nested) => {
                                let nested_var = format!("_nested_arr_{}", i);
                                self.writeln(&format!(
                                    "let {} = match &({}) {{ Value::Array(ref _a) => _a.borrow().get({}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                    nested_var, value_expr, i
                                ));
                                self.emit_destruct_bindings(nested, &nested_var, mutability, span)?;
                            }
                            DestructElement::Rest(name) => {
                                self.writeln(&format!(
                                    "{} {} = match &({}) {{ Value::Array(ref _a) => {{ let _b = _a.borrow(); Value::Array(Rc::new(RefCell::new(_b.iter().skip({}).cloned().collect()))) }}, _ => Value::Array(Rc::new(RefCell::new(Vec::new()))) }};",
                                    mutability,
                                    Self::escape_ident(name.as_ref()),
                                    value_expr,
                                    i
                                ));
                            }
                        }
                    }
                }
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    let key = prop.key.as_ref();
                    match &prop.value {
                        DestructElement::Ident(name) => {
                            self.writeln(&format!(
                                "{} {} = match &({}) {{ Value::Object(ref _o) => _o.borrow().get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                mutability,
                                Self::escape_ident(name.as_ref()),
                                value_expr,
                                key
                            ));
                        }
                        DestructElement::Pattern(nested) => {
                            let nested_var = format!("_nested_obj_{}", key);
                            self.writeln(&format!(
                                "let {} = match &({}) {{ Value::Object(ref _o) => _o.borrow().get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                nested_var, value_expr, key
                            ));
                            self.emit_destruct_bindings(nested, &nested_var, mutability, span)?;
                        }
                        DestructElement::Rest(_) => {
                            return Err(CompileError::new(
                                "Rest in object destructuring not supported",
                                Some(span),
                            ));
                        }
                    }
                }
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
                    format!("(*{}.borrow()).clone()", escaped)
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
            Expr::Binary { .. } => {
                // Delegate to emit_typed_expr; wrap the native result in Value.
                let (code, ty) = self.emit_typed_expr(expr)?;
                if ty.is_native() { ty.to_value_expr(&code) } else { code }
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
                // Compile-time embed: Polars.read_csv("<literal path>") when file exists
                if let Some((crate_name, _)) = self.native_module_map.get("tish:polars") {
                    if let (Some(root), Some(CallArg::Expr(first_arg))) =
                        (self.project_root.as_ref(), args.first())
                    {
                        if let Expr::Member {
                            object,
                            prop: MemberProp::Name(ref method_name),
                            ..
                        } = callee.as_ref()
                        {
                            if method_name.as_ref() == "read_csv"
                                && matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Polars")
                            {
                                if let Expr::Literal {
                                    value: Literal::String(ref path),
                                    ..
                                } = first_arg
                                {
                                    let path_str = path.as_ref();
                                    let normalized = path_str.trim_start_matches("./");
                                    let full_path = root.join(normalized);
                                    if full_path.exists() {
                                        if let Ok(content) = std::fs::read_to_string(&full_path) {
                                            let escaped = format!("{:?}", content);
                                            return Ok(format!(
                                                "{}::polars_read_csv_from_string_runtime({})",
                                                crate_name, escaped
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Check for built-in method calls on arrays/strings
                if let Expr::Member { object, prop: MemberProp::Name(method_name), .. } = callee.as_ref() {
                    // ── native Vec<T> push fast path ──────────────────────────────
                    if method_name.as_ref() == "push" {
                        if let Expr::Ident { name, .. } = object.as_ref() {
                            if !self.refcell_wrapped_vars.contains(name.as_ref()) {
                                let obj_type = self.type_context.get_type(name.as_ref());
                                if let RustType::Vec(elem_type) = obj_type {
                                    let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
                                    // Collect push arguments as native values.
                                    let mut push_stmts: Vec<String> = Vec::new();
                                    for a in args {
                                        if let CallArg::Expr(e) = a {
                                            let (val_code, val_ty) = self.emit_typed_expr(e)?;
                                            let native_val = if val_ty == *elem_type {
                                                val_code
                                            } else if val_ty == RustType::Value {
                                                elem_type.from_value_expr(&val_code)
                                            } else {
                                                val_code
                                            };
                                            push_stmts.push(format!("{}.push({});", esc_obj, native_val));
                                        }
                                    }
                                    return Ok(format!(
                                        "{{ {} Value::Null }}",
                                        push_stmts.join(" ")
                                    ));
                                }
                            }
                        }
                    }

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
                                "tishlang_runtime::array_push(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        "pop" => {
                            return Ok(format!("tishlang_runtime::array_pop(&{})", obj_expr));
                        }
                        "shift" => {
                            return Ok(format!("tishlang_runtime::array_shift(&{})", obj_expr));
                        }
                        "unshift" => {
                            let args_vec = arg_exprs.iter()
                                .map(|a| format!("{}.clone()", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!(
                                "tishlang_runtime::array_unshift(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        "indexOf" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let from = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tishlang_runtime::array_index_of(&_obj, &{}), Value::String(_) => tishlang_runtime::string_index_of(&_obj, &{}, &{}), _ => Value::Number(-1.0) }} }}",
                                obj_expr, search, search, from
                            ));
                        }
                        "includes" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let from = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tishlang_runtime::array_includes(&_obj, &{}, &{}), Value::String(_) => tishlang_runtime::string_includes(&_obj, &{}, &{}), _ => Value::Bool(false) }} }}",
                                obj_expr, search, from, search, from
                            ));
                        }
                        "join" => {
                            let sep = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_join(&{}, &{})",
                                obj_expr, sep
                            ));
                        }
                        "reverse" => {
                            return Ok(format!("tishlang_runtime::array_reverse(&{})", obj_expr));
                        }
                        "shuffle" => {
                            return Ok(format!("tishlang_runtime::array_shuffle(&{})", obj_expr));
                        }
                        "slice" => {
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let end = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::Array(_) => tishlang_runtime::array_slice(&_obj, &{}, &{}), Value::String(_) => tishlang_runtime::string_slice(&_obj, &{}, &{}), _ => Value::Null }} }}",
                                obj_expr, start, end, start, end
                            ));
                        }
                        "concat" => {
                            let args_vec = arg_exprs.iter()
                                .map(|a| format!("{}.clone()", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!(
                                "tishlang_runtime::array_concat(&{}, &[{}])",
                                obj_expr, args_vec
                            ));
                        }
                        // String-only methods
                        "substring" => {
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let end = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_substring(&{}, &{}, &{})",
                                obj_expr, start, end
                            ));
                        }
                        "split" => {
                            let sep = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_split(&{}, &{})",
                                obj_expr, sep
                            ));
                        }
                        "trim" => {
                            return Ok(format!("tishlang_runtime::string_trim(&{})", obj_expr));
                        }
                        "toUpperCase" => {
                            return Ok(format!("tishlang_runtime::string_to_upper_case(&{})", obj_expr));
                        }
                        "toLowerCase" => {
                            return Ok(format!("tishlang_runtime::string_to_lower_case(&{})", obj_expr));
                        }
                        "startsWith" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_starts_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "endsWith" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_ends_with(&{}, &{})",
                                obj_expr, search
                            ));
                        }
                        "replace" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_replace(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "replaceAll" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            let replacement = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::String(\"\".into())".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_replace_all(&{}, &{}, &{})",
                                obj_expr, search, replacement
                            ));
                        }
                        "match" if cfg!(feature = "regex") => {
                            let regexp = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_match_regex(&{}, &{})",
                                obj_expr, regexp
                            ));
                        }
                        "charAt" => {
                            let idx = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_char_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "charCodeAt" => {
                            let idx = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_char_code_at(&{}, &{})",
                                obj_expr, idx
                            ));
                        }
                        "repeat" => {
                            let count = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_repeat(&{}, &{})",
                                obj_expr, count
                            ));
                        }
                        "padStart" => {
                            let target_len = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_pad_start(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        "padEnd" => {
                            let target_len = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let pad = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_pad_end(&{}, &{}, &{})",
                                obj_expr, target_len, pad
                            ));
                        }
                        // Number methods
                        "toFixed" => {
                            let digits = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::number_to_fixed(&{}, &{})",
                                obj_expr, digits
                            ));
                        }
                        // Higher-order array methods
                        "map" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_map(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "filter" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_filter(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "reduce" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let initial = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_reduce(&{}, &{}, &{})",
                                obj_expr, callback, initial
                            ));
                        }
                        "forEach" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_for_each(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "find" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_find(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "findIndex" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_find_index(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "some" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_some(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "every" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_every(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "sort" => {
                            // Check for numeric sort fast path: (a, b) => a - b or (a, b) => b - a
                            if let Some(CallArg::Expr(comparator_expr)) = args.first() {
                                if let Some(ascending) = Self::detect_numeric_sort_comparator(comparator_expr) {
                                    if ascending {
                                        return Ok(format!(
                                            "tishlang_runtime::array_sort_numeric_asc(&{})",
                                            obj_expr
                                        ));
                                    } else {
                                        return Ok(format!(
                                            "tishlang_runtime::array_sort_numeric_desc(&{})",
                                            obj_expr
                                        ));
                                    }
                                }
                            }
                            // General case: use the callback
                            let comparator = arg_exprs.first().map(|c| format!("Some(&{})", c)).unwrap_or_else(|| "None".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_sort(&{}, {})",
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
                                "tishlang_runtime::array_splice(&{}, &{}, {}, {})",
                                obj_expr, start, delete_count, items
                            ));
                        }
                        "flat" => {
                            let depth = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(1.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_flat(&{}, &{})",
                                obj_expr, depth
                            ));
                        }
                        "flatMap" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_flat_map(&{}, &{})",
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
                        "{{ let _callee = &{}; let _spread_args = {}; match _callee {{ Value::Function(cb) => cb(&_spread_args), other => panic!(\"Not a function: tried to call {{:?}} as a function (e.g. method on Null when read failed)\", other) }} }}",
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
                     {}    let _callee = &{};\n\
                     {}    match _callee {{ Value::Function(cb) => cb(&[{}]), other => panic!(\"Not a function: tried to call {{:?}} as a function (e.g. method on Null when read failed)\", other) }}\n\
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
                         tishlang_runtime::get_prop(&o, {}) }} }}",
                        obj, key
                    )
                } else {
                    format!("tishlang_runtime::get_prop(&{}, {})", obj, key)
                }
            }
            Expr::Index { optional, .. } if !optional => {
                // Try native Vec<T> fast path via emit_typed_expr; wrap result.
                let (code, ty) = self.emit_typed_expr(expr)?;
                if ty.is_native() { ty.to_value_expr(&code) } else { code }
            }
            Expr::Index {
                object,
                index,
                ..
            } => {
                // optional chaining: always use runtime path
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                format!(
                    "{{ let o = {}.clone(); if matches!(o, Value::Null) {{ Value::Null }} else {{ \
                     tishlang_runtime::get_index(&o, &{}) }} }}",
                    obj, idx
                )
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
                            if let ArrayElement::Spread(e) = elem {
                                return Err(CompileError::new("Unexpected spread", Some(e.span())));
                            }
                            unreachable!("else only for Spread");
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
                    format!("{{ let mut _obj: ObjectMap = ObjectMap::default(); {} Value::Object(Rc::new(RefCell::new(_obj))) }}", parts.join(" "))
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
                        "Value::Object(Rc::new(RefCell::new(ObjectMap::from([{}]))))",
                        parts.join(", ")
                    )
                }
            }
            Expr::Assign { name, value, .. } => {
                let escaped = Self::escape_ident(name.as_ref());
                // Native fast path: if the target is a scalar native type, emit
                // a direct assignment without boxing/unboxing through Value.
                if !self.refcell_wrapped_vars.contains(name.as_ref()) {
                    let rust_type = self.type_context.get_type(name.as_ref());
                    if rust_type.is_native() && matches!(rust_type, RustType::F64 | RustType::Bool | RustType::String) {
                        let (val_code, val_ty) = self.emit_typed_expr(value)?;
                        let native_val = if val_ty == rust_type {
                            val_code
                        } else if val_ty == RustType::Value {
                            rust_type.from_value_expr(&val_code)
                        } else {
                            val_code
                        };
                        let return_val = rust_type.to_value_expr(&escaped);
                        return Ok(format!(
                            "{{ {} = {}; {} }}",
                            escaped, native_val, return_val
                        ));
                    }
                }
                // Fallback: Value path
                let val = self.emit_expr(value)?;
                let needs_outer_clone = self.should_clone(value);
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    if needs_outer_clone {
                        format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    } else {
                        format!("{{ let _v = {}; *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    }
                } else {
                    let rust_type = self.type_context.get_type(name.as_ref());
                    let assign_rhs = if matches!(rust_type, RustType::Value) {
                        "_v.clone()".to_string()
                    } else {
                        rust_type.from_value_expr("_v")
                    };
                    if needs_outer_clone {
                        format!("{{ let _v = ({}).clone(); {} = {}; _v }}", val, escaped, assign_rhs)
                    } else {
                        format!("{{ let _v = {}; {} = {}; _v }}", val, escaped, assign_rhs)
                    }
                }
            }
            Expr::Await { operand, .. } => {
                #[cfg(feature = "http")]
                if self.is_async {
                    let _in_async = self.async_context_stack.last().copied().unwrap_or(false);
                    if let Expr::Call { callee, args, .. } = operand.as_ref() {
                        if let Expr::Ident { name, .. } = callee.as_ref() {
                            let args_code = self.emit_call_args(args)?;
                            return Ok(match name.as_ref() {
                                "fetch" => {
                                    format!("tish_await_promise(tish_fetch_promise({}))", args_code)
                                }
                                "fetchAll" => {
                                    format!("tish_await_promise(tish_fetch_all_promise({}))", args_code)
                                }
                                _ => {
                                    let o = self.emit_expr(operand)?;
                                    return Ok(format!("tish_await_promise({})", o));
                                }
                            });
                        }
                    }
                    // await Call with non-Ident callee, or await Promise value: wrap in await_promise
                    let o = self.emit_expr(operand)?;
                    return Ok(format!("tish_await_promise({})", o));
                }
                // Fallback: emit operand as sync call (no real .await in our model)
                let o = self.emit_expr(operand)?;
                format!("({})", o)
            }
            Expr::TypeOf { operand, .. } => {
                let o = self.emit_expr(operand)?;
                format!(
                    "Value::String(match &{} {{ \
                     Value::Number(_) => \"number\".into(), Value::String(_) => \"string\".into(), \
                     Value::Bool(_) => \"boolean\".into(), Value::Null => \"null\".into(), \
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
                let n = Self::escape_ident(name.as_ref());
                let is_refcell = self.refcell_wrapped_vars.contains(name.as_ref());
                let var_type = self.type_context.get_type(name.as_ref());

                // ── native f64 fast path: direct arithmetic operators ─────────
                // emit_expr must return a Value expression; wrap the result back.
                if !is_refcell && var_type == RustType::F64 {
                    let (rhs_code, rhs_ty) = self.emit_typed_expr(value)?;
                    let rhs_f64 = if rhs_ty == RustType::F64 {
                        rhs_code
                    } else {
                        // rhs is Value or another native: unbox to f64
                        let rhs_val = if rhs_ty.is_native() {
                            rhs_ty.to_value_expr(&rhs_code)
                        } else {
                            rhs_code
                        };
                        format!("(match &({}) {{ Value::Number(n) => *n, v => panic!(\"compound assign: expected number, got {{:?}}\", v) }})", rhs_val)
                    };
                    let op_str = match op {
                        CompoundOp::Add => "+=",
                        CompoundOp::Sub => "-=",
                        CompoundOp::Mul => "*=",
                        CompoundOp::Div => "/=",
                        CompoundOp::Mod => "%=",
                    };
                    // Wrap in Value::Number so the expression is a valid Value
                    return Ok(format!("{{ {} {} {}; Value::Number({}) }}", n, op_str, rhs_f64, n));
                }

                // ── native String += fast path: push_str ─────────────────────
                if !is_refcell && var_type == RustType::String && matches!(op, CompoundOp::Add) {
                    let (rhs_code, rhs_ty) = self.emit_typed_expr(value)?;
                    let rhs_str = if rhs_ty == RustType::String {
                        rhs_code
                    } else {
                        // Convert rhs Value to display string inline
                        let rhs_val = if rhs_ty.is_native() {
                            rhs_ty.to_value_expr(&rhs_code)
                        } else {
                            rhs_code
                        };
                        format!(
                            "match &({}) {{ \
                             Value::String(s) => s.to_string(), \
                             Value::Number(n) => {{ let i = *n as i64; if (*n - i as f64).abs() < f64::EPSILON {{ i.to_string() }} else {{ n.to_string() }} }}, \
                             Value::Bool(b) => b.to_string(), \
                             Value::Null => \"null\".to_string(), \
                             other => format!(\"{{:?}}\", other) }}",
                            rhs_val
                        )
                    };
                    // Wrap in Value::String so the expression is a valid Value
                    return Ok(format!("{{ {}.push_str(&({})); Value::String({}.clone().into()) }}", n, rhs_str, n));
                }

                // ── fallback: Value path ──────────────────────────────────────
                // If the variable is native, wrap it as Value before calling ops::
                let val = self.emit_expr(value)?;
                let op_fn = match op {
                    CompoundOp::Add => "add",
                    CompoundOp::Sub => "sub",
                    CompoundOp::Mul => "mul",
                    CompoundOp::Div => "div",
                    CompoundOp::Mod => "modulo",
                };
                if is_refcell {
                    format!(
                        "{{ let _rhs = ({}).clone(); *{}.borrow_mut() = tishlang_runtime::ops::{}(&*{}.borrow(), &_rhs)?; (*{}.borrow()).clone() }}",
                        val, n, op_fn, n, n
                    )
                } else if var_type.is_native() {
                    // Wrap native lhs as Value, run ops::, unbox result back to native
                    let n_as_value = var_type.to_value_expr(&n);
                    let result_native = var_type.from_value_expr("_result");
                    let n_as_value2 = var_type.to_value_expr(&n);
                    format!(
                        "{{ let _lhs = {}; let _rhs = ({}).clone(); let _result = tishlang_runtime::ops::{}(&_lhs, &_rhs)?; {} = {}; {} }}",
                        n_as_value, val, op_fn, n, result_native, n_as_value2
                    )
                } else {
                    format!(
                        "{{ let _rhs = ({}).clone(); {} = tishlang_runtime::ops::{}(&{}, &_rhs)?; {}.clone() }}",
                        val, n, op_fn, n, n
                    )
                }
            }
            Expr::LogicalAssign { name, op, value, .. } => {
                let val = self.emit_expr(value)?;
                let n = Self::escape_ident(name.as_ref()).into_owned();
                let is_refcell = self.refcell_wrapped_vars.contains(name.as_ref());
                let var_type = self.type_context.get_type(name.as_ref());

                // ── native type: wrap for condition, unbox for assignment ──────
                if !is_refcell && var_type.is_native() {
                    // n_as_value uses .clone() for String so we don't consume n
                    let n_as_value = var_type.to_value_expr(&n);
                    let val_as_native = var_type.from_value_expr("_v");
                    let (cond, assign_and_return, else_expr) = match op {
                        LogicalAssignOp::AndAnd => (
                            format!("{{ let __chk = {}; __chk.is_truthy() }}", n_as_value),
                            format!("{{ let _v = ({}).clone(); {} = {}; {} }}", val, n, val_as_native, var_type.to_value_expr(&n)),
                            var_type.to_value_expr(&n),
                        ),
                        LogicalAssignOp::OrOr => (
                            format!("!{{ let __chk = {}; __chk.is_truthy() }}", n_as_value),
                            format!("{{ let _v = ({}).clone(); {} = {}; {} }}", val, n, val_as_native, var_type.to_value_expr(&n)),
                            var_type.to_value_expr(&n),
                        ),
                        // Native types (f64, String, bool) are never null — ??= is a no-op
                        LogicalAssignOp::Nullish => (
                            "false".to_string(),
                            var_type.to_value_expr(&n), // unreachable but must type-check
                            var_type.to_value_expr(&n),
                        ),
                    };
                    return Ok(format!("{{ if {} {{ {} }} else {{ {} }} }}", cond, assign_and_return, else_expr));
                }

                // ── Value / refcell path ──────────────────────────────────────
                let (cond, assign_and_return, else_expr) = if is_refcell {
                    match op {
                        LogicalAssignOp::AndAnd => (
                            format!("{}.borrow().is_truthy()", n),
                            format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, n),
                            format!("(*{}.borrow()).clone()", n),
                        ),
                        LogicalAssignOp::OrOr => (
                            format!("!{}.borrow().is_truthy()", n),
                            format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, n),
                            format!("(*{}.borrow()).clone()", n),
                        ),
                        LogicalAssignOp::Nullish => (
                            format!("matches!(*{}.borrow(), Value::Null)", n),
                            format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, n),
                            format!("(*{}.borrow()).clone()", n),
                        ),
                    }
                } else {
                    match op {
                        LogicalAssignOp::AndAnd => (
                            format!("{}.is_truthy()", n),
                            format!("{{ let _v = ({}).clone(); {} = _v.clone(); _v }}", val, n),
                            format!("{}.clone()", n),
                        ),
                        LogicalAssignOp::OrOr => (
                            format!("!{}.is_truthy()", n),
                            format!("{{ let _v = ({}).clone(); {} = _v.clone(); _v }}", val, n),
                            format!("{}.clone()", n),
                        ),
                        LogicalAssignOp::Nullish => (
                            format!("matches!({}, Value::Null)", n),
                            format!("{{ let _v = ({}).clone(); {} = _v.clone(); _v }}", val, n),
                            format!("{}.clone()", n),
                        ),
                    }
                };
                format!("{{ if {} {{ {} }} else {{ {} }} }}", cond, assign_and_return, else_expr)
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                let obj = self.emit_expr(object)?;
                let val = self.emit_expr(value)?;
                format!(
                    "tishlang_runtime::set_prop(&({}), \"{}\", ({}).clone())",
                    obj,
                    prop.as_ref(),
                    val
                )
            }
            Expr::IndexAssign { object, index, value, .. } => {
                // Native fast path: Vec<T>[i] = v
                if let Expr::Ident { name, .. } = object.as_ref() {
                    if !self.refcell_wrapped_vars.contains(name.as_ref()) {
                        let obj_type = self.type_context.get_type(name.as_ref());
                        if let RustType::Vec(elem_type) = obj_type {
                            let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
                            let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
                            let idx_usize = if idx_ty == RustType::F64 {
                                format!("({}) as usize", idx_code)
                            } else {
                                let iv = if idx_ty.is_native() {
                                    idx_ty.to_value_expr(&idx_code)
                                } else {
                                    idx_code
                                };
                                format!(
                                    "{{ let _i = &{}; if let Value::Number(n) = _i {{ *n as usize }} else {{ panic!(\"array index must be a number\") }} }}",
                                    iv
                                )
                            };
                            let (val_code, val_ty) = self.emit_typed_expr(value)?;
                            let native_val = if val_ty == *elem_type {
                                val_code
                            } else if val_ty == RustType::Value {
                                elem_type.from_value_expr(&val_code)
                            } else {
                                // both native but different type — best effort
                                val_code
                            };
                            return Ok(format!(
                                "{{ {}[{}] = {}; Value::Null }}",
                                esc_obj, idx_usize, native_val
                            ));
                        }
                    }
                }
                // Fallback: runtime set_index
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let val = self.emit_expr(value)?;
                format!(
                    "tishlang_runtime::set_index(&({}), &({}), ({}).clone())",
                    obj,
                    idx,
                    val
                )
            }
            Expr::ArrowFunction { params, body, span, .. } => {
                self.emit_arrow_function(params, body, *span)?
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
            Expr::JsxElement { .. } | Expr::JsxFragment { .. } => {
                tishlang_ui::jsx::emit_jsx_rust(expr, &mut |e| {
                    self.emit_expr(e).map_err(|ce| ce.message)
                })
                .map_err(|m| CompileError::new(m, None))?
            }
            Expr::New { callee, args, .. } => {
                let callee_expr = self.emit_expr(callee)?;
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    let args_code = self.emit_call_args(args)?;
                    return Ok(format!(
                        "{{ let _callee = ({}).clone(); let _spread_args = {}; tish_construct(&_callee, &_spread_args[..]) }}",
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
                    "tish_construct(&({}).clone(), &[{}])",
                    callee_expr, args_vec
                )
            }
            Expr::NativeModuleLoad { spec, export_name, .. } => {
                self.native_module_rust_init(spec.as_ref(), export_name.as_ref())
                    .ok_or_else(|| CompileError {
                        message: if crate::resolve::is_builtin_native_spec(spec.as_ref()) {
                            format!(
                                "Built-in module '{}' does not export '{}'. Add --feature {} when compiling.",
                                spec.as_ref(),
                                export_name.as_ref(),
                                spec.as_ref().strip_prefix("tish:").unwrap_or(spec.as_ref())
                            )
                        } else {
                            format!(
                                "Native module '{}' not found. Add it as a dependency and ensure package.json has tish.module.",
                                spec.as_ref()
                            )
                        },
                        span: None,
                    })?
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
            Expr::New { callee, args, .. } => {
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
            Expr::LogicalAssign { name, value, .. } => {
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
            Expr::Await { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs { Self::collect_expr_idents(e, idents); }
            }
            Expr::JsxElement { props, children, .. } => {
                for p in props {
                    match p {
                        tishlang_ast::JsxProp::Attr { value: tishlang_ast::JsxAttrValue::Expr(e), .. } | tishlang_ast::JsxProp::Spread(e) => Self::collect_expr_idents(e, idents),
                        _ => {}
                    }
                }
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        Self::collect_expr_idents(e, idents);
                    }
                }
            }
            Expr::JsxFragment { children, .. } => {
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        Self::collect_expr_idents(e, idents);
                    }
                }
            }
            Expr::NativeModuleLoad { .. } => {}
            Expr::Literal { .. } => {}
        }
    }
    
    /// Collect variable names that are assigned to in a statement/body (target of =, +=, ++, etc).
    fn collect_assigned_idents_in_stmt(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::ExprStmt { expr, .. } => Self::collect_assigned_idents_in_expr(expr, names),
            Statement::VarDecl { .. } | Statement::VarDeclDestructure { .. } => {}
            Statement::Block { statements, .. } => {
                for s in statements {
                    Self::collect_assigned_idents_in_stmt(s, names);
                }
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                Self::collect_assigned_idents_in_expr(cond, names);
                Self::collect_assigned_idents_in_stmt(then_branch, names);
                if let Some(eb) = else_branch {
                    Self::collect_assigned_idents_in_stmt(eb, names);
                }
            }
            Statement::For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    Self::collect_assigned_idents_in_stmt(i, names);
                }
                if let Some(c) = cond {
                    Self::collect_assigned_idents_in_expr(c, names);
                }
                if let Some(u) = update {
                    Self::collect_assigned_idents_in_expr(u, names);
                }
                Self::collect_assigned_idents_in_stmt(body, names);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_assigned_idents_in_expr(iterable, names);
                Self::collect_assigned_idents_in_stmt(body, names);
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                Self::collect_assigned_idents_in_expr(cond, names);
                Self::collect_assigned_idents_in_stmt(body, names);
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                Self::collect_assigned_idents_in_expr(expr, names);
                for (case_expr, stmts) in cases {
                    if let Some(e) = case_expr {
                        Self::collect_assigned_idents_in_expr(e, names);
                    }
                    for s in stmts {
                        Self::collect_assigned_idents_in_stmt(s, names);
                    }
                }
                if let Some(stmts) = default_body {
                    for s in stmts {
                        Self::collect_assigned_idents_in_stmt(s, names);
                    }
                }
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                Self::collect_assigned_idents_in_stmt(body, names);
                if let Some(c) = catch_body {
                    Self::collect_assigned_idents_in_stmt(c, names);
                }
                if let Some(f) = finally_body {
                    Self::collect_assigned_idents_in_stmt(f, names);
                }
            }
            Statement::FunDecl { body, .. } => Self::collect_assigned_idents_in_stmt(body, names),
            Statement::Return { value, .. } => {
                if let Some(e) = value {
                    Self::collect_assigned_idents_in_expr(e, names);
                }
            }
            Statement::Throw { value, .. } => Self::collect_assigned_idents_in_expr(value, names),
            Statement::Break { .. } | Statement::Continue { .. } | Statement::Import { .. } | Statement::Export { .. } => {}
        }
    }

    fn collect_assigned_idents_in_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::Assign { name, value, .. } => {
                names.insert(name.to_string());
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::CompoundAssign { name, value, .. } => {
                names.insert(name.to_string());
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::LogicalAssign { name, value, .. } => {
                names.insert(name.to_string());
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::PostfixInc { name, .. } | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. } | Expr::PrefixDec { name, .. } => {
                names.insert(name.to_string());
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::collect_assigned_idents_in_expr(object, names);
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                Self::collect_assigned_idents_in_expr(object, names);
                Self::collect_assigned_idents_in_expr(index, names);
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::Binary { left, right, .. } => {
                Self::collect_assigned_idents_in_expr(left, names);
                Self::collect_assigned_idents_in_expr(right, names);
            }
            Expr::Unary { operand, .. } => Self::collect_assigned_idents_in_expr(operand, names),
            Expr::Call { callee, args, .. } => {
                Self::collect_assigned_idents_in_expr(callee, names);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_assigned_idents_in_expr(e, names);
                        }
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::collect_assigned_idents_in_expr(callee, names);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_assigned_idents_in_expr(e, names);
                        }
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_assigned_idents_in_expr(object, names);
                if let MemberProp::Expr(e) = prop {
                    Self::collect_assigned_idents_in_expr(e, names);
                }
            }
            Expr::Index { object, index, .. } => {
                Self::collect_assigned_idents_in_expr(object, names);
                Self::collect_assigned_idents_in_expr(index, names);
            }
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                Self::collect_assigned_idents_in_expr(cond, names);
                Self::collect_assigned_idents_in_expr(then_branch, names);
                Self::collect_assigned_idents_in_expr(else_branch, names);
            }
            Expr::ArrowFunction { body, .. } => {
                match body {
                    ArrowBody::Expr(e) => Self::collect_assigned_idents_in_expr(e, names),
                    ArrowBody::Block(s) => Self::collect_assigned_idents_in_stmt(s, names),
                }
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::collect_assigned_idents_in_expr(e, names);
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => {
                            Self::collect_assigned_idents_in_expr(e, names);
                        }
                    }
                }
            }
            Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_assigned_idents_in_expr(left, names);
                Self::collect_assigned_idents_in_expr(right, names);
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::collect_assigned_idents_in_expr(e, names);
                }
            }
            Expr::JsxElement { props, children, .. } => {
                for p in props {
                    match p {
                        tishlang_ast::JsxProp::Attr {
                            value: tishlang_ast::JsxAttrValue::Expr(e),
                            ..
                        }
                        | tishlang_ast::JsxProp::Spread(e) => {
                            Self::collect_assigned_idents_in_expr(e, names);
                        }
                        _ => {}
                    }
                }
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        Self::collect_assigned_idents_in_expr(e, names);
                    }
                }
            }
            Expr::JsxFragment { children, .. } => {
                for c in children {
                    if let tishlang_ast::JsxChild::Expr(e) = c {
                        Self::collect_assigned_idents_in_expr(e, names);
                    }
                }
            }
            Expr::Ident { .. }
            | Expr::Literal { .. }
            | Expr::TypeOf { .. }
            | Expr::Await { .. }
            | Expr::NativeModuleLoad { .. } => {}
        }
    }

    /// Collect vars declared in the given statements (top-level only, no recursion into blocks).
    fn collect_block_var_names(statements: &[Statement], names: &mut HashSet<String>) {
        for s in statements {
            match s {
                Statement::VarDecl { name, .. } => {
                    names.insert(name.to_string());
                }
                Statement::VarDeclDestructure { pattern, .. } => {
                    Self::collect_destruct_names(pattern, names);
                }
                Statement::For {
                    init: Some(i),
                    ..
                } => {
                    if let Statement::VarDecl { name, .. } = i.as_ref() {
                        names.insert(name.to_string());
                    }
                    if let Statement::VarDeclDestructure { pattern, .. } = i.as_ref() {
                        Self::collect_destruct_names(pattern, names);
                    }
                }
                Statement::For { init: None, .. } => {}
                _ => {}
            }
        }
    }

    /// Collect variable names that are both captured and mutated by a closure body.
    /// block_vars: vars declared in the enclosing block (candidates for mutation).
    fn collect_mutated_captures_from_closure(
        params: &[FunParam],
        body: &Statement,
        block_vars: &HashSet<String>,
        result: &mut HashSet<String>,
    ) {
        let param_names: HashSet<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();
        let mut local_var_names = HashSet::new();
        Self::collect_local_var_names(body, &mut local_var_names);
        let mut referenced = HashSet::new();
        Self::collect_stmt_idents(body, &mut referenced);
        let mut assigned = HashSet::new();
        Self::collect_assigned_idents_in_stmt(body, &mut assigned);
        let outer_captured: HashSet<String> = referenced
            .difference(&param_names)
            .cloned()
            .collect::<HashSet<_>>()
            .difference(&local_var_names)
            .cloned()
            .collect();
        for v in outer_captured.intersection(&assigned) {
            if block_vars.contains(v) {
                result.insert(v.clone());
            }
        }
        // Recurse into nested fns
        Self::collect_mutated_captures_from_statements(body, block_vars, result);
    }

    fn collect_mutated_captures_from_arrow(
        params: &[FunParam],
        body: &ArrowBody,
        block_vars: &HashSet<String>,
        result: &mut HashSet<String>,
    ) {
        let param_names: HashSet<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();
        let mut local_var_names = HashSet::new();
        match body {
            ArrowBody::Expr(_) => {}
            ArrowBody::Block(s) => Self::collect_local_var_names(s, &mut local_var_names),
        }
        let mut referenced = HashSet::new();
        match body {
            ArrowBody::Expr(e) => Self::collect_expr_idents(e, &mut referenced),
            ArrowBody::Block(s) => Self::collect_stmt_idents(s, &mut referenced),
        }
        let mut assigned = HashSet::new();
        match body {
            ArrowBody::Expr(e) => Self::collect_assigned_idents_in_expr(e, &mut assigned),
            ArrowBody::Block(s) => Self::collect_assigned_idents_in_stmt(s, &mut assigned),
        }
        let outer_captured: HashSet<String> = referenced
            .difference(&param_names)
            .cloned()
            .collect::<HashSet<_>>()
            .difference(&local_var_names)
            .cloned()
            .collect();
        for v in outer_captured.intersection(&assigned) {
            if block_vars.contains(v) {
                result.insert(v.clone());
            }
        }
        match body {
            ArrowBody::Expr(e) => Self::collect_mutated_captures_from_expr(e, block_vars, result),
            ArrowBody::Block(s) => Self::collect_mutated_captures_from_statements(s, block_vars, result),
        }
    }

    fn collect_mutated_captures_from_expr(expr: &Expr, block_vars: &HashSet<String>, result: &mut HashSet<String>) {
        match expr {
            Expr::ArrowFunction { params, body, .. } => {
                Self::collect_mutated_captures_from_arrow(params, body, block_vars, result);
            }
            Expr::Call { callee, args, .. } => {
                Self::collect_mutated_captures_from_expr(callee, block_vars, result);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_mutated_captures_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::collect_mutated_captures_from_expr(callee, block_vars, result);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_mutated_captures_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_mutated_captures_from_expr(object, block_vars, result);
                if let MemberProp::Expr(e) = prop {
                    Self::collect_mutated_captures_from_expr(e, block_vars, result);
                }
            }
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                Self::collect_mutated_captures_from_expr(cond, block_vars, result);
                Self::collect_mutated_captures_from_expr(then_branch, block_vars, result);
                Self::collect_mutated_captures_from_expr(else_branch, block_vars, result);
            }
            Expr::Binary { left, right, .. }
            | Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_mutated_captures_from_expr(left, block_vars, result);
                Self::collect_mutated_captures_from_expr(right, block_vars, result);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::collect_mutated_captures_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => {
                            Self::collect_mutated_captures_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_mutated_captures_from_statements(
        stmt: &Statement,
        block_vars: &HashSet<String>,
        result: &mut HashSet<String>,
    ) {
        match stmt {
            Statement::FunDecl { params, body, .. } => {
                Self::collect_mutated_captures_from_closure(params, body, block_vars, result);
            }
            Statement::ExprStmt { expr, .. } => {
                Self::collect_mutated_captures_from_expr(expr, block_vars, result);
            }
            Statement::Block { statements, .. } => {
                for s in statements {
                    Self::collect_mutated_captures_from_statements(s, block_vars, result);
                }
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                Self::collect_mutated_captures_from_expr(cond, block_vars, result);
                Self::collect_mutated_captures_from_statements(then_branch, block_vars, result);
                if let Some(eb) = else_branch {
                    Self::collect_mutated_captures_from_statements(eb, block_vars, result);
                }
            }
            Statement::For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    Self::collect_mutated_captures_from_statements(i, block_vars, result);
                }
                if let Some(c) = cond {
                    Self::collect_mutated_captures_from_expr(c, block_vars, result);
                }
                if let Some(u) = update {
                    Self::collect_mutated_captures_from_expr(u, block_vars, result);
                }
                Self::collect_mutated_captures_from_statements(body, block_vars, result);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_mutated_captures_from_expr(iterable, block_vars, result);
                Self::collect_mutated_captures_from_statements(body, block_vars, result);
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                Self::collect_mutated_captures_from_expr(cond, block_vars, result);
                Self::collect_mutated_captures_from_statements(body, block_vars, result);
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                Self::collect_mutated_captures_from_expr(expr, block_vars, result);
                for (ce, stmts) in cases {
                    if let Some(e) = ce {
                        Self::collect_mutated_captures_from_expr(e, block_vars, result);
                    }
                    for s in stmts {
                        Self::collect_mutated_captures_from_statements(s, block_vars, result);
                    }
                }
                if let Some(stmts) = default_body {
                    for s in stmts {
                        Self::collect_mutated_captures_from_statements(s, block_vars, result);
                    }
                }
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                Self::collect_mutated_captures_from_statements(body, block_vars, result);
                if let Some(c) = catch_body {
                    Self::collect_mutated_captures_from_statements(c, block_vars, result);
                }
                if let Some(f) = finally_body {
                    Self::collect_mutated_captures_from_statements(f, block_vars, result);
                }
            }
            Statement::VarDecl { init: Some(e), .. } => {
                Self::collect_mutated_captures_from_expr(e, block_vars, result);
            }
            Statement::VarDeclDestructure { init, .. } => {
                Self::collect_mutated_captures_from_expr(init, block_vars, result);
            }
            Statement::Return { value: Some(e), .. } => {
                Self::collect_mutated_captures_from_expr(e, block_vars, result);
            }
            Statement::Throw { value, .. } => Self::collect_mutated_captures_from_expr(value, block_vars, result),
            _ => {}
        }
    }

    /// For a block, return var names that must be RefCell (captured and mutated by nested closures).
    fn collect_vars_mutated_by_nested_closures(statements: &[Statement]) -> HashSet<String> {
        let mut block_vars = HashSet::new();
        Self::collect_block_var_names(statements, &mut block_vars);
        let mut result = HashSet::new();
        for s in statements {
            Self::collect_mutated_captures_from_statements(s, &block_vars, &mut result);
        }
        result
    }

    /// Collect variable names declared in a statement (VarDecl, Destructure, For init).
    fn collect_local_var_names(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::VarDecl { name, .. } => { names.insert(name.to_string()); }
            Statement::VarDeclDestructure { pattern, .. } => {
                Self::collect_destruct_names(pattern, names);
            }
            Statement::Block { statements, .. } => {
                for s in statements { Self::collect_local_var_names(s, names); }
            }
            Statement::If { then_branch, else_branch, .. } => {
                Self::collect_local_var_names(then_branch, names);
                if let Some(eb) = else_branch { Self::collect_local_var_names(eb, names); }
            }
            Statement::For { init, body, .. } => {
                if let Some(i) = init { Self::collect_local_var_names(i, names); }
                Self::collect_local_var_names(body, names);
            }
            Statement::ForOf { body, .. } => Self::collect_local_var_names(body, names),
            Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                Self::collect_local_var_names(body, names);
            }
            Statement::Switch { cases, default_body, .. } => {
                for (_, stmts) in cases {
                    for s in stmts { Self::collect_local_var_names(s, names); }
                }
                if let Some(stmts) = default_body {
                    for s in stmts { Self::collect_local_var_names(s, names); }
                }
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                Self::collect_local_var_names(body, names);
                if let Some(c) = catch_body { Self::collect_local_var_names(c, names); }
                if let Some(f) = finally_body { Self::collect_local_var_names(f, names); }
            }
            Statement::FunDecl { body, .. } => Self::collect_local_var_names(body, names),
            _ => {}
        }
    }

    fn collect_destruct_names(pattern: &DestructPattern, names: &mut HashSet<String>) {
        match pattern {
            DestructPattern::Array(elements) => {
                for el in elements {
                    if let Some(DestructElement::Ident(n)) = el { names.insert(n.to_string()); }
                    if let Some(DestructElement::Pattern(p)) = el { Self::collect_destruct_names(p, names); }
                }
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    match &prop.value {
                        DestructElement::Ident(n) => { names.insert(n.to_string()); }
                        DestructElement::Pattern(p) => Self::collect_destruct_names(p, names),
                        DestructElement::Rest(n) => { names.insert(n.to_string()); }
                    }
                }
            }
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
            Statement::Break { .. } | Statement::Continue { .. } | Statement::Import { .. } | Statement::Export { .. } => {}
        }
    }

    fn emit_arrow_function(
        &mut self,
        params: &[FunParam],
        body: &tishlang_ast::ArrowBody,
        span: Span,
    ) -> Result<String, CompileError> {
        // Build the arrow function as a Value::Function closure
        let mut code = String::new();
        code.push_str("{\n");
        
        // Find which identifiers are actually referenced in the body
        let referenced = Self::collect_referenced_idents(body);
        // Exclude the arrow's own parameters - they're not outer captures
        let param_names: HashSet<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();

        // Exclude variables declared inside the arrow body (locals)
        let mut local_var_names = HashSet::new();
        match body {
            ArrowBody::Expr(_) => {}
            ArrowBody::Block(stmt) => Self::collect_local_var_names(stmt, &mut local_var_names),
        }

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
            .filter(|name| referenced.contains(name) && !param_names.contains(name) && !local_var_names.contains(name))
            .collect();

        // Outer vars that are assigned in the body need RefCell; read-only get Value binding
        let mut assigned_in_body = HashSet::new();
        match body {
            ArrowBody::Expr(e) => Self::collect_assigned_idents_in_expr(e, &mut assigned_in_body),
            ArrowBody::Block(s) => Self::collect_assigned_idents_in_stmt(s, &mut assigned_in_body),
        }
        let mutable_outer_vars: Vec<String> = outer_vars.iter().filter(|v| assigned_in_body.contains(*v)).cloned().collect();
        let read_only_outer_vars: Vec<String> = outer_vars.iter().filter(|v| !assigned_in_body.contains(*v)).cloned().collect();

        // Track which vars are already RefCell-wrapped (from outer closure) to avoid double-wrapping
        let already_wrapped = self.refcell_wrapped_vars.clone();

        // Wrap outer captures in Rc<RefCell<>> and use _ref suffix
        for outer_param in &outer_params {
            let param_escaped = Self::escape_ident(outer_param);
            let ref_name = format!("{}_ref", param_escaped);
            if already_wrapped.contains(outer_param) {
                code.push_str(&format!("    let {} = {}.clone();\n", ref_name, param_escaped));
            } else {
                code.push_str(&format!("    let {} = std::rc::Rc::new(RefCell::new({}.clone()));\n", ref_name, param_escaped));
            }
        }
        for outer_var in &outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            let ref_name = format!("{}_ref", var_escaped);
            if already_wrapped.contains(outer_var) {
                code.push_str(&format!("    let {} = {}.clone();\n", ref_name, var_escaped));
            } else {
                code.push_str(&format!("    let {} = std::rc::Rc::new(RefCell::new({}.clone()));\n", ref_name, var_escaped));
            }
        }
        // Only clone builtins that are actually referenced (clone so outer scope can still use, e.g. process for PORT)
        for builtin in &["console", "Math", "JSON", "Date", "Uint8Array", "AudioContext", "process", "setTimeout", "clearTimeout", "Promise", "RegExp", "Polars"] {
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

        // Make captured outer params available as plain Values (from _ref RefCells)
        for outer_param in &outer_params {
            let param_escaped = Self::escape_ident(outer_param);
            code.push_str(&format!("        let {} = (*{}_ref.borrow()).clone();\n", param_escaped, param_escaped));
        }
        // Mutable outer vars: capture RefCell so assignments use borrow_mut
        for outer_var in &mutable_outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            code.push_str(&format!("        let {} = {}_ref.clone();\n", var_escaped, var_escaped));
        }
        // Read-only outer vars: Value binding from borrow
        for outer_var in &read_only_outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            code.push_str(&format!("        let {} = (*{}_ref.borrow()).clone();\n", var_escaped, var_escaped));
        }

        // Make captured functions available
        for func_name in &referenced_funcs {
            let escaped = Self::escape_ident(func_name);
            code.push_str(&format!("        let {} = (*{}_ref.borrow()).clone();\n", escaped, escaped));
        }

        // Extract parameters from args
        let current_param_names: Vec<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();
        for (i, p) in params.iter().enumerate() {
            match p {
                FunParam::Simple(tp) => {
                    code.push_str(&format!(
                        "        let mut {} = args.get({}).cloned().unwrap_or(Value::Null);\n",
                        Self::escape_ident(tp.name.as_ref()),
                        i
                    ));
                }
                FunParam::Destructure { pattern, .. } => {
                    let tmp = format!("_formal_{}", i);
                    code.push_str(&format!(
                        "        let {} = args.get({}).cloned().unwrap_or(Value::Null);\n",
                        tmp, i
                    ));
                    let saved = std::mem::take(&mut self.output);
                    let saved_indent = self.indent;
                    self.indent = 8;
                    self.emit_destruct_bindings(pattern, &tmp, "let mut", span)?;
                    let frag = std::mem::replace(&mut self.output, saved);
                    self.indent = saved_indent;
                    code.push_str(&frag);
                }
            }
        }

        // Push current params for potential nested arrows
        self.outer_params_stack.push(current_param_names);
        // Push empty scope for variables declared inside this arrow function
        self.outer_vars_stack.push(Vec::new());

        // Mutable outer vars need to be in refcell_wrapped_vars so Assign/CompoundAssign emit borrow_mut
        let saved_refcell_vars = self.refcell_wrapped_vars.clone();
        for v in &mutable_outer_vars {
            self.refcell_wrapped_vars.insert(v.clone());
        }

        // Emit body based on type
        match body {
            tishlang_ast::ArrowBody::Expr(expr) => {
                let expr_code = self.emit_expr(expr)?;
                code.push_str(&format!("        {}\n", expr_code));
            }
            tishlang_ast::ArrowBody::Block(block_stmt) => {
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

    /// Emit an expression and return `(code, type)`.
    ///
    /// When `type` is a native type (`F64`, `Bool`, `String`, `Vec<T>`, …), `code`
    /// evaluates to a Rust value of that type directly — **not** a `Value`.
    /// When `type` is `RustType::Value`, `code` evaluates to a `Value`.
    ///
    /// This is the fast-path used by callers that want to propagate native types
    /// through arithmetic, indexing, and assignments.  For any expression this
    /// function cannot handle natively, it falls back to `emit_expr` and returns
    /// `RustType::Value`.
    fn emit_typed_expr(&mut self, expr: &Expr) -> Result<(String, RustType), CompileError> {
        match expr {
            // ── literals ─────────────────────────────────────────────────────────
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => Ok((format!("{}_f64", n), RustType::F64)),
                Literal::String(s) => Ok((format!("{:?}.to_string()", s.as_ref()), RustType::String)),
                Literal::Bool(b) => Ok((format!("{}", b), RustType::Bool)),
                Literal::Null => Ok(("Value::Null".to_string(), RustType::Value)),
            },

            // ── identifiers ──────────────────────────────────────────────────────
            Expr::Ident { name, .. } => {
                let escaped = Self::escape_ident(name.as_ref());
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    // RefCell-wrapped: unwrap via borrow and return Value
                    Ok((format!("(*{}.borrow()).clone()", escaped), RustType::Value))
                } else {
                    let var_type = self.type_context.get_type(name.as_ref());
                    if var_type.is_native() {
                        Ok((escaped.into_owned(), var_type))
                    } else {
                        Ok((escaped.into_owned(), RustType::Value))
                    }
                }
            }

            // ── binary expressions ───────────────────────────────────────────────
            Expr::Binary { left, op, right, span, .. } => {
                let (l, lt) = self.emit_typed_expr(left)?;
                let (r, rt) = self.emit_typed_expr(right)?;

                if let Some(result_ty) = RustType::result_type_of_binop(*op, &lt, &rt) {
                    // Both sides are compatible native types → emit native op.
                    let code = match op {
                        BinOp::Add => format!("({} + {})", l, r),
                        BinOp::Sub => format!("({} - {})", l, r),
                        BinOp::Mul => format!("({} * {})", l, r),
                        BinOp::Div => format!("({} / {})", l, r),
                        BinOp::Mod => format!("({} % {})", l, r),
                        BinOp::Pow => format!("({}).powf({})", l, r),
                        BinOp::Lt => format!("({} < {})", l, r),
                        BinOp::Le => format!("({} <= {})", l, r),
                        BinOp::Gt => format!("({} > {})", l, r),
                        BinOp::Ge => format!("({} >= {})", l, r),
                        BinOp::StrictEq => format!("({} == {})", l, r),
                        BinOp::StrictNe => format!("({} != {})", l, r),
                        BinOp::And => format!("({} && {})", l, r),
                        BinOp::Or => format!("({} || {})", l, r),
                        _ => unreachable!("result_type_of_binop covers all handled ops"),
                    };
                    return Ok((code, result_ty));
                }

                // Fall back: convert both sides to Value and use the runtime.
                let lv = if lt.is_native() { lt.to_value_expr(&l) } else { l };
                let rv = if rt.is_native() { rt.to_value_expr(&r) } else { r };
                let result = self.emit_binop(&lv, *op, &rv, *span)?;
                Ok((result, RustType::Value))
            }

            // ── array indexing ───────────────────────────────────────────────────
            Expr::Index { object, index, optional, .. } => {
                // Native fast path: `vec[i]` where vec is Vec<T> and i is numeric.
                if !optional {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if !self.refcell_wrapped_vars.contains(name.as_ref()) {
                            let obj_type = self.type_context.get_type(name.as_ref());
                            if let RustType::Vec(elem_type) = &obj_type {
                                let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
                                let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
                                let idx_usize = if idx_ty == RustType::F64 {
                                    format!("({}) as usize", idx_code)
                                } else {
                                    let iv = if idx_ty.is_native() {
                                        idx_ty.to_value_expr(&idx_code)
                                    } else {
                                        idx_code
                                    };
                                    format!(
                                        "{{ let _i = &{}; if let Value::Number(n) = _i {{ *n as usize }} else {{ panic!(\"array index must be a number\") }} }}",
                                        iv
                                    )
                                };
                                let elem_ty = *elem_type.clone();
                                return Ok((format!("{}[{}]", esc_obj, idx_usize), elem_ty));
                            }
                        }
                    }
                }
                // Value fallback: emit runtime code directly to avoid cycles
                // (emit_expr for !optional Index delegates here, so we must not call emit_expr(expr))
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let result = if *optional {
                    format!(
                        "{{ let o = {}.clone(); if matches!(o, Value::Null) {{ Value::Null }} else {{ \
                         tishlang_runtime::get_index(&o, &{}) }} }}",
                        obj, idx
                    )
                } else {
                    format!("tishlang_runtime::get_index(&{}, &{})", obj, idx)
                };
                Ok((result, RustType::Value))
            }

            // ── everything else: delegate to emit_expr ───────────────────────────
            _ => {
                let result = self.emit_expr(expr)?;
                Ok((result, RustType::Value))
            }
        }
    }

    /// Emit a condition expression as a Rust `bool`.
    ///
    /// Returns a `bool`-typed Rust expression when the condition can be
    /// determined to be native (e.g. `i < N` where both are `f64`), otherwise
    /// falls back to `{value}.is_truthy()`.
    fn emit_cond_expr(&mut self, expr: &Expr) -> Result<String, CompileError> {
        let (code, ty) = self.emit_typed_expr(expr)?;
        if ty == RustType::Bool {
            Ok(code)
        } else if ty.is_native() {
            // Non-bool native type: convert to Value and use is_truthy
            Ok(format!("{}.is_truthy()", ty.to_value_expr(&code)))
        } else {
            Ok(format!("{}.is_truthy()", code))
        }
    }

    fn emit_binop(
        &self,
        l: &str,
        op: BinOp,
        r: &str,
        span: Span,
    ) -> Result<String, CompileError> {
        Ok(match op {
            BinOp::Add => format!("tishlang_runtime::ops::add(&{}, &{}).unwrap_or(Value::Null)", l, r),
            BinOp::Sub => format!("tishlang_runtime::ops::sub(&{}, &{}).unwrap_or(Value::Null)", l, r),
            BinOp::Mul => format!("tishlang_runtime::ops::mul(&{}, &{}).unwrap_or(Value::Null)", l, r),
            BinOp::Div => format!("tishlang_runtime::ops::div(&{}, &{}).unwrap_or(Value::Null)", l, r),
            BinOp::Mod => format!("tishlang_runtime::ops::modulo(&{}, &{}).unwrap_or(Value::Null)", l, r),
            BinOp::Pow => format!(
                "Value::Number({{ let Value::Number(a) = &({}) else {{ panic!() }}; \
                 let Value::Number(b) = &({}) else {{ panic!() }}; a.powf(*b) }})",
                l, r
            ),
            BinOp::StrictEq => format!("Value::Bool({}.strict_eq(&{}))", l, r),
            BinOp::StrictNe => format!("Value::Bool(!{}.strict_eq(&{}))", l, r),
            BinOp::Lt => format!("tishlang_runtime::ops::lt(&{}, &{})", l, r),
            BinOp::Le => format!("tishlang_runtime::ops::le(&{}, &{})", l, r),
            BinOp::Gt => format!("tishlang_runtime::ops::gt(&{}, &{})", l, r),
            BinOp::Ge => format!("tishlang_runtime::ops::ge(&{}, &{})", l, r),
            BinOp::And => format!("Value::Bool({}.is_truthy() && {}.is_truthy())", l, r),
            BinOp::Or => format!("Value::Bool({}.is_truthy() || {}.is_truthy())", l, r),
            BinOp::BitAnd => Self::emit_bitwise_binop(l, r, "&"),
            BinOp::BitOr => Self::emit_bitwise_binop(l, r, "|"),
            BinOp::BitXor => Self::emit_bitwise_binop(l, r, "^"),
            BinOp::Shl => Self::emit_bitwise_binop(l, r, "<<"),
            BinOp::Shr => Self::emit_bitwise_binop(l, r, ">>"),
            BinOp::In => format!("tish_in_operator(&{}, &{})", l, r),
            BinOp::Eq | BinOp::Ne => {
                return Err(CompileError::new("Loose equality not supported", Some(span)))
            }
        })
    }
}
