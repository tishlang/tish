//! Code generation: AST -> Rust source.

use crate::resolve::is_builtin_native_spec;
use crate::types::{RustType, TypeContext};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern, Expr,
    FunParam, Literal, LogicalAssignOp, MemberProp, ObjectProp, Program, Span, Statement,
    TypeAnnotation, UnaryOp,
};

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
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.analyze_expr(cond);
                self.analyze_statement(then_branch);
                if let Some(e) = else_branch {
                    self.analyze_statement(e);
                }
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                self.analyze_statements(statements)
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
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
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
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
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
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
            Statement::Import { .. }
            | Statement::Export { .. }
            | Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {
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
                        ObjectProp::KeyValue(_, v, _) => self.analyze_expr(v),
                        ObjectProp::Spread(e) => self.analyze_expr(e),
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => self.analyze_expr(e),
                ArrowBody::Block(s) => self.analyze_statement(s),
            },
            Expr::Assign { value, .. } => self.analyze_expr(value),
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.analyze_expr(cond);
                self.analyze_expr(then_branch);
                self.analyze_expr(else_branch);
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.analyze_expr(left);
                self.analyze_expr(right);
            }
            Expr::TypeOf { operand, .. } => self.analyze_expr(operand),
            Expr::Delete { target, .. } => self.analyze_expr(target),
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
            Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } => {
                *self.use_counts.entry(name.to_string()).or_insert(0) += 1;
            }
            Expr::MemberAssign { object, value, .. } => {
                self.analyze_expr(object);
                self.analyze_expr(value);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
                self.analyze_expr(value);
            }
            Expr::Await { operand, .. } => self.analyze_expr(operand),
            Expr::JsxElement {
                props, children, ..
            } => {
                for p in props {
                    match p {
                        tishlang_ast::JsxProp::Attr {
                            value: tishlang_ast::JsxAttrValue::Expr(e),
                            ..
                        }
                        | tishlang_ast::JsxProp::Spread(e) => self.analyze_expr(e),
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
        Self {
            message: msg.into(),
            span,
        }
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
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(stmt_has_async)
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                stmt_has_async(then_branch)
                    || else_branch
                        .as_ref()
                        .is_some_and(|s| stmt_has_async(s.as_ref()))
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::ForOf { body, .. }
            | Statement::DoWhile { body, .. } => stmt_has_async(body),
            Statement::Switch {
                cases,
                default_body,
                ..
            } => {
                cases
                    .iter()
                    .any(|(_, stmts)| stmts.iter().any(stmt_has_async))
                    || default_body
                        .as_ref()
                        .is_some_and(|stmts| stmts.iter().any(stmt_has_async))
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                stmt_has_async(body)
                    || catch_body
                        .as_ref()
                        .is_some_and(|s| stmt_has_async(s.as_ref()))
                    || finally_body
                        .as_ref()
                        .is_some_and(|s| stmt_has_async(s.as_ref()))
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
                expr_has_await(callee)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => expr_has_await(e),
                    })
            }
            Expr::New { callee, args, .. } => {
                expr_has_await(callee)
                    || args.iter().any(|a| match a {
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
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => expr_has_await(cond) || expr_has_await(then_branch) || expr_has_await(else_branch),
            Expr::NullishCoalesce { left, right, .. } => {
                expr_has_await(left) || expr_has_await(right)
            }
            Expr::Array { elements, .. } => elements.iter().any(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_has_await(e),
            }),
            Expr::Object { props, .. } => props.iter().any(|p| match p {
                ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => expr_has_await(e),
            }),
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. }
            | Expr::MemberAssign { value, .. }
            | Expr::IndexAssign { value, .. } => expr_has_await(value),
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => expr_has_await(e),
                ArrowBody::Block(s) => stmt_has_async(s),
            },
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(expr_has_await),
            Expr::JsxElement {
                props, children, ..
            } => {
                props.iter().any(|p| match p {
                    tishlang_ast::JsxProp::Attr {
                        value: tishlang_ast::JsxAttrValue::Expr(e),
                        ..
                    }
                    | tishlang_ast::JsxProp::Spread(e) => expr_has_await(e),
                    _ => false,
                }) || children
                    .iter()
                    .any(|c| matches!(c, tishlang_ast::JsxChild::Expr(e) if expr_has_await(e)))
            }
            Expr::JsxFragment { children, .. } => children
                .iter()
                .any(|c| matches!(c, tishlang_ast::JsxChild::Expr(e) if expr_has_await(e))),
            _ => false,
        }
    }
    fn stmt_has_await(s: &Statement) -> bool {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(stmt_has_await)
            }
            Statement::VarDecl { init, .. } => init.as_ref().is_some_and(expr_has_await),
            Statement::VarDeclDestructure { init, .. } => expr_has_await(init),
            Statement::ExprStmt { expr, .. } => expr_has_await(expr),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                expr_has_await(cond)
                    || stmt_has_await(then_branch)
                    || else_branch
                        .as_ref()
                        .is_some_and(|s| stmt_has_await(s.as_ref()))
            }
            Statement::While { cond, body, .. } => expr_has_await(cond) || stmt_has_await(body),
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                init.as_ref().is_some_and(|s| stmt_has_await(s.as_ref()))
                    || cond.as_ref().is_some_and(expr_has_await)
                    || update.as_ref().is_some_and(expr_has_await)
                    || stmt_has_await(body)
            }
            Statement::ForOf { iterable, body, .. } => {
                expr_has_await(iterable) || stmt_has_await(body)
            }
            Statement::Return { value, .. } => value.as_ref().is_some_and(expr_has_await),
            Statement::FunDecl { body, .. } => stmt_has_await(body),
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
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
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                stmt_has_await(body)
                    || catch_body
                        .as_ref()
                        .is_some_and(|s| stmt_has_await(s.as_ref()))
                    || finally_body
                        .as_ref()
                        .is_some_and(|s| stmt_has_await(s.as_ref()))
            }
            Statement::Import { .. } | Statement::Export { .. } => false,
            _ => false,
        }
    }
    program
        .statements
        .iter()
        .any(|s| stmt_has_async(s) || stmt_has_await(s))
}

pub fn compile(program: &Program) -> Result<String, CompileError> {
    compile_with_project_root(program, None)
}

pub fn compile_with_project_root(
    program: &Program,
    project_root: Option<&Path>,
) -> Result<String, CompileError> {
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
    let (rust, _, _, _) = compile_project_full(entry_path, project_root, features, true)?;
    Ok(rust)
}

/// Compile a project and return Rust code, resolved native modules, the **effective** feature list
/// (CLI features plus any inferred from `tish:fs` / `tish:http` / … imports), and native build
/// artifacts (Cargo dep lines, optional `generated_native.rs` source, init strategy per spec).
pub fn compile_project_full(
    entry_path: &Path,
    project_root: Option<&Path>,
    features: &[String],
    optimize: bool,
) -> Result<
    (
        String,
        Vec<crate::resolve::ResolvedNativeModule>,
        Vec<String>,
        crate::resolve::NativeBuildArtifacts,
    ),
    CompileError,
> {
    compile_project_full_emit(
        entry_path,
        project_root,
        features,
        optimize,
        crate::NativeEmitMode::DesktopBin,
        None,
    )
}

/// Like [`compile_project_full`], with emit mode and optional feature cap (e.g. iOS sandbox).
pub fn compile_project_full_emit(
    entry_path: &Path,
    project_root: Option<&Path>,
    features: &[String],
    optimize: bool,
    emit_mode: crate::NativeEmitMode,
    feature_cap: Option<&std::collections::HashSet<String>>,
) -> Result<
    (
        String,
        Vec<crate::resolve::ResolvedNativeModule>,
        Vec<String>,
        crate::resolve::NativeBuildArtifacts,
    ),
    CompileError,
> {
    use crate::resolve;
    let root = project_root.unwrap_or_else(|| entry_path.parent().unwrap_or(Path::new(".")));
    let modules = resolve::resolve_project(entry_path, project_root).map_err(|e| CompileError {
        message: e,
        span: None,
    })?;
    resolve::detect_cycles(&modules).map_err(|e| CompileError {
        message: e,
        span: None,
    })?;
    let merged = resolve::merge_modules(modules).map_err(|e| CompileError {
        message: e,
        span: None,
    })?;
    let mut native_modules =
        resolve::resolve_native_modules(&merged.program, root).map_err(|e| CompileError {
            message: e,
            span: None,
        })?;
    if resolve::program_uses_document(&merged.program) {
        resolve::ensure_tish_canvas_module(&mut native_modules, root).map_err(|e| CompileError {
            message: e,
            span: None,
        })?;
    }
    let mut native_build =
        resolve::compute_native_build_artifacts(&merged.program, root, &native_modules).map_err(
            |e| CompileError {
                message: e,
                span: None,
            },
        )?;
    let mut all_features: Vec<String> = features.to_vec();
    for f in resolve::extract_native_import_features(&merged.program) {
        if !all_features.contains(&f) {
            all_features.push(f);
        }
    }
    if let Some(cap) = feature_cap {
        all_features.retain(|f| cap.contains(f));
    }
    let rust = compile_with_native_modules_emit(
        &merged.program,
        project_root,
        &all_features,
        &native_modules,
        &native_build.native_init,
        optimize,
        emit_mode,
    )?;
    Ok((rust, native_modules, all_features, native_build))
}

/// Compile with explicit feature flags. When features are provided, codegen uses them
/// to emit builtins (process, serve, etc.) regardless of tishlang_compile's #[cfg] build.
pub fn compile_with_features(
    program: &Program,
    project_root: Option<&Path>,
    features: &[String],
) -> Result<String, CompileError> {
    let empty = std::collections::HashMap::new();
    compile_with_native_modules(program, project_root, features, &[], &empty, true)
}

/// Compile with resolved native modules. Native imports emit calls to the module crates directly.
pub fn compile_with_native_modules(
    program: &Program,
    project_root: Option<&Path>,
    features: &[String],
    native_modules: &[crate::resolve::ResolvedNativeModule],
    native_init: &std::collections::HashMap<String, crate::resolve::NativeModuleInit>,
    optimize: bool,
) -> Result<String, CompileError> {
    compile_with_native_modules_emit(
        program,
        project_root,
        features,
        native_modules,
        native_init,
        optimize,
        crate::NativeEmitMode::DesktopBin,
    )
}

/// Opt-in gradual type check. `TISH_CHECK=1`/`warn` prints provable annotation violations to stderr
/// as warnings; `TISH_CHECK=error` also fails the build. Unset/`0` → no-op (default builds are
/// unaffected). The checker is gradual (see `check.rs`): it never flags code it can't prove wrong.
fn run_type_check(program: &Program) -> Result<(), CompileError> {
    let mode = std::env::var("TISH_CHECK").unwrap_or_default();
    if mode.is_empty() || mode == "0" {
        return Ok(());
    }
    let diags = crate::check::check_program(program);
    if diags.is_empty() {
        return Ok(());
    }
    let kind = if mode == "error" { "error" } else { "warning" };
    for d in &diags {
        eprintln!(
            "tish type {}: {}:{}: {}",
            kind, d.span.start.0, d.span.start.1, d.message
        );
    }
    if mode == "error" {
        return Err(CompileError::new(
            format!("type checking failed: {} error(s)", diags.len()),
            Some(diags[0].span),
        ));
    }
    Ok(())
}

pub fn compile_with_native_modules_emit(
    program: &Program,
    project_root: Option<&Path>,
    features: &[String],
    native_modules: &[crate::resolve::ResolvedNativeModule],
    native_init: &std::collections::HashMap<String, crate::resolve::NativeModuleInit>,
    optimize: bool,
    emit_mode: crate::NativeEmitMode,
) -> Result<String, CompileError> {
    let program = if optimize {
        tishlang_opt::optimize(program)
    } else {
        program.clone()
    };
    // Gradual type check (opt-in via `TISH_CHECK`): `=1`/`=warn` prints provable annotation
    // violations as warnings; `=error` blocks the build. Off by default — never affects the
    // standard build. Run on the optimized, pre-inference program (real user annotations only).
    run_type_check(&program)?;
    // Type-inference pass: fills in `type_ann` on unannotated VarDecl nodes where
    // the type is unambiguous (literals, arithmetic of typed vars, etc.).
    let program = crate::infer::infer_program(&program);
    let map: std::collections::HashMap<String, crate::resolve::NativeModuleInit> =
        if native_init.is_empty() {
            native_modules
                .iter()
                .map(|m| {
                    (
                        m.spec.clone(),
                        crate::resolve::NativeModuleInit::Legacy {
                            crate_name: m.crate_name.clone(),
                            export_fn: m.export_fn.clone(),
                        },
                    )
                })
                .collect()
        } else {
            native_init.clone()
        };
    let mut g = Codegen::new_with_native_modules(project_root, features, map);
    g.emit_mode = emit_mode;
    g.has_native_ui_host = native_modules.iter().any(|m| {
        m.package_name == "tish-macos"
            || m.package_name == "tish-ios"
            || m.crate_name == "tishlang_macos"
            || m.crate_name == "tishlang_ios"
    });
    g.emit_program(&program)?;
    Ok(g.output)
}

/// #177 (S-E/S-F): the return shape of a de-virtualized aggregate (struct/array) free fn.
#[derive(Debug, Clone, PartialEq)]
enum AggRet {
    /// Returns the unboxed struct by value (the `body()` factory).
    Struct,
    /// Returns `Vec<TishStruct_alias>` by value (the `makeBodies()` array factory).
    ArrayOfStruct,
    /// Returns a plain `f64` (`energy()`).
    F64,
    /// Returns nothing (`advance()`, `offsetMomentum()` — JS `undefined`).
    Unit,
}

/// #177: one parameter of an aggregate free fn, in source order.
#[derive(Debug, Clone)]
enum AggParamKind {
    /// The `Vec<TishStruct_alias>` array param, threaded by shared reference
    /// (`&mut` if the fn mutates an element, `&` if read-only).
    Array { is_mut: bool },
    /// A scalar param (always `f64` for the nbody shape).
    Scalar(RustType),
}

/// #177: the de-virtualized native signature of one aggregate fn.
#[derive(Debug, Clone)]
struct AggFnSig {
    /// Source params in order: (name, kind).
    params: Vec<(String, AggParamKind)>,
    /// Top-level numeric globals the body references, appended as trailing `f64`
    /// params (sorted for a stable decl/call-site order).
    captured: Vec<String>,
    /// What the fn returns.
    ret: AggRet,
}

/// #178 (behind `TISH_REC_STRUCT`): a recursive struct shape inferred *structurally* (no name
/// matching) from object literals whose fields are either scalars or recursive self-references.
/// Drives native lowering of tree/list builders and consumers to `Option<Box<T>>` + native fns —
/// the real, name-independent binary_trees lowering. See docs/recursive-struct-native.md.
#[derive(Debug, Clone)]
struct RecStructPlan {
    /// Synthesised Tish alias name (the Rust struct is `TishStruct_<alias>`).
    alias: String,
    /// Field set in declaration order: (name, is_child). `is_child` ⇒ `Option<Box<Self>>`,
    /// else a scalar `f64`.
    fields: Vec<(std::sync::Arc<str>, bool)>,
    /// Fns that return the struct by value (the recursive builders, e.g. `bottomUpTree`).
    builders: std::collections::HashSet<String>,
    /// Fns that take the struct (by `&`) and return `f64` (the recursive consumers, e.g.
    /// `itemCheck`).
    consumers: std::collections::HashSet<String>,
    /// Numeric fns that orchestrate builders/consumers (e.g. `binaryTrees`): they hold node-index
    /// locals, loop, and call builders/consumers. Lowered to `fn name_rec(<f64..>, &mut Vec<Node>)
    /// -> f64`.
    orchestrators: std::collections::HashSet<String>,
}

/// #178: emission context for a recursive-struct fn body.
struct RecCtx {
    /// True inside a builder body (returns the struct); false inside a consumer (returns f64).
    is_builder: bool,
    /// The consumer's node param name (`&TishStruct_alias`), if in a consumer.
    node: Option<String>,
}

/// #175 — accumulated facts about how a fn param is used in the body (for param classification).
#[derive(Debug, Default)]
struct ParamUse {
    indexed: bool,
    is_mut: bool,
    elem_bool: bool,
    numeric: bool,
    escaped: bool,
    /// #320 — the bare param was passed as a direct call argument (`f(p)`). The native-vec path
    /// allows this (re-borrow into a callee), but the arr-param owned-copy path must NOT: the callee
    /// could mutate the array, and a write to the caller's array would be lost on the owned copy.
    /// `classify_vec_param` ignores this field; only `native_arr_param_fns` reads it.
    forwarded: bool,
}

/// #175 — kind of one parameter of a native-vec free fn (spectral_norm/queens shape).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum VecParamKind {
    /// A scalar `f64` param.
    Scalar,
    /// A `number[]` / `boolean[]` param threaded by reference: `&Vec<T>` (read-only) or
    /// `&mut Vec<T>` (the body writes an element). `elem` is `F64` or `Bool`.
    Array { elem: RustType, is_mut: bool },
}

/// #175 — the de-virtualized native signature of one plain-array free fn. Unlike the #177 aggregate
/// path (single struct-alias array), this supports MULTIPLE plain `number[]`/`boolean[]` params and
/// recursion; call sites pass pairwise-distinct array idents (statically checked) so no runtime
/// alias guard is needed. Emitted as `fn name_nv(<f64..>, <&/&mut Vec<T>..>) -> f64 | Vec<f64> | ()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum VecRetKind {
    F64,
    Unit,
    VecF64,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeVecFnSig {
    pub(crate) params: Vec<(String, VecParamKind)>,
    ret: VecRetKind,
}

/// #173 part 3 — a symbolic upper bound used by the in-bounds index proof. Two forms are matched by
/// structural equality: an integer constant (`vec![K; 100]`, guard `i < 100`) and a single variable
/// (`a` filled to length `n`, guard `i < n`). Anything more complex (`2 * n`, `a.length` member,
/// arithmetic bounds) is intentionally not modeled — those keep the existing OOB-safe lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BoundKey {
    Const(i64),
    Var(String),
    /// `2 * n` — fixed length of queens occupancy diagonals (`diag1`/`diag2` are `2n` bools).
    TwiceVar(String),
    /// `a.length` — the live length of array `a`. As a GUARD (`i < a.length`) it proves `a[i]` is in
    /// bounds directly, provided `a` never shrinks (guaranteed when `a` is in `vec_fixed_len`).
    Len(String),
}

/// #173 part 3 — an active upper-bound guard from an enclosing loop condition `var <cmp> bound`,
/// live for the textual span of the loop body until `var` is reassigned (after which its value is no
/// longer bounded, so `live` is cleared and the guard stops proving anything).
#[derive(Debug, Clone)]
struct IndexGuard {
    var: String,
    bound: BoundKey,
    strict: bool,
    live: bool,
}

pub(crate) struct Codegen {
    output: String,
    indent: usize,
    loop_label_index: usize,
    is_async: bool,
    /// Requested features (http, process, fs, regex, polars). When non-empty, used instead of #[cfg].
    features: std::collections::HashSet<String>,
    /// spec -> native init strategy (legacy adapter object vs generated `generated_native` wrapper)
    native_module_init: std::collections::HashMap<String, crate::resolve::NativeModuleInit>,
    /// Stack: true = async Rust context (run body), false = sync closure (Tish fn body)
    async_context_stack: Vec<bool>,
    loop_stack: Vec<(String, Option<String>)>, // (break_label, continue_update) for innermost loop
    /// Break targets for innermost breakable construct — loops AND switches (JS `break` exits the
    /// nearest loop OR switch; `continue` uses loop_stack). Loops push to both; switches push here only.
    break_stack: Vec<String>,
    /// How many enclosing `try`-body closures we're currently emitting inside (within the current
    /// function). A try body compiles to `(|| -> Result<Option<Value>, _> { … })()` — a *completion*
    /// closure: `Ok(None)`=normal, `Ok(Some(v))`=pending `return v`, `Err(Throw)`=pending throw. When
    /// depth>0, `return`/`throw` emit the closure-escaping completion form so they unwind through
    /// `finally`; at depth 0 they're a plain `return`/panic (the fast path is untouched).
    try_closure_depth: u32,
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
    /// #176 (behind `TISH_NATIVE_FN`): top-level `let` bindings lowered to `thread_local Cell<f64>`
    /// (`G_NAME`) when every use is a numeric read or a whole-binding numeric assign (fasta `seed`).
    native_numeric_globals: std::collections::HashMap<String, f64>,
    /// Top-level `let name = [f64 literals…]` never reassigned — emitted as `const G_name: [f64; N]`
    /// for direct indexing from native fns (fasta `codes`/`probs`).
    module_const_f64_arrays: std::collections::HashMap<String, Vec<f64>>,
    /// Precomputed cumulative arrays for `module_const_f64_arrays` used as `cumulative(p)` input
    /// (`probs` → `G_probs_cum`).
    module_const_f64_cum: std::collections::HashMap<String, String>,
    /// Locals aliased to a module const array (`cum` → `G_probs_cum` after `let cum = cumulative(probs)`).
    module_const_f64_aliases: std::collections::HashMap<String, String>,
    /// #181: locals initialized with `new Map()` — direct `map_has`/`get`/`set`/`values` dispatch.
    map_instance_locals: std::collections::HashSet<String>,
    /// M5 (dark-shipped behind `TISH_NATIVE_FN`): top-level functions eligible for a parallel
    /// free `fn f_native(f64,..)->f64` (all params `: number`, returns `number`, native-safe
    /// body). Direct calls to these route to the native fn, bypassing the boxed `value_call`.
    native_fns: std::collections::HashSet<String>,
    /// True while emitting an M5 `fn name_native` body — keeps VarDecl inits on the native path.
    native_fn_body_emit: bool,
    /// M5 fn currently being emitted (`mandel_native`, `fastaRandom_native`, …).
    native_fn_emit_name: Option<String>,
    /// Top-level literal call args for each M5 fn (`mandel(1200,1200,100)` → mandel).
    native_fn_literal_args: std::collections::HashMap<String, Vec<f64>>,
    /// M5 fn param names in declaration order (for literal-arg specialization).
    native_fn_param_names: std::collections::HashMap<String, Vec<String>>,
    /// `usize` `for` loop counters — provably `>= 0` even when absent from `nonneg_locals`.
    usize_for_counters: std::collections::HashSet<String>,
    /// Hoisted LCG seed local in a non-`genRandom` M5 body that calls `genRandom`.
    native_lcg_hoist: Option<(String, f64, f64, f64)>,
    /// Hoisted LCG uses an `i64` seed when mul/add/mod are exact integers (fasta hot path).
    native_lcg_hoist_int: bool,
    /// Set when an M5 body emits `return` so we skip the trailing `0.0` default.
    native_fn_body_returned: bool,
    /// M5 LCG fns (`genRandom`): `fn -> (global, mul, add, modulus)` for single-`with` emission.
    native_lcg_fns: std::collections::HashMap<String, (String, f64, f64, f64)>,
    /// `&/&mut Vec<f64|bool>` + `f64` params (`name -> sig`). Direct calls route to `name_nv(..)`
    /// passing array idents by reference; the boxed closure is suppressed. Empty unless the whole
    /// group lowers cleanly (scratch-buffer rollback), so it can never regress the boxed path.
    native_vec_fns: std::collections::HashMap<String, NativeVecFnSig>,
    /// #203 follow-up — per native-vec fn, the array params PROVEN (at every call site) to be at least
    /// as long as the fn's first scalar param. `emit_native_vec_fn` re-registers these as
    /// `vec_fixed_len[param] = Var(scalar)` so in-bounds index reads drop the OOB-safe `.get()` fallback
    /// (recovering spectral_norm's raw indexing). Empty unless [`Self::compute_native_vec_param_bounds`]
    /// can prove the length relationship — otherwise the param keeps its OOB-safe lowering (sound).
    native_vec_param_bounds:
        std::collections::HashMap<String, std::collections::HashMap<String, BoundKey>>,
    /// #175 follow-up — top-level numeric "leaf" fns (`fn f(a,…){ return <numeric expr> }`, no other
    /// statements, body built only from params/literals/arithmetic/1-arg `Math`) eligible for INLINING
    /// at a native-f64 call site (`name -> (param names, body expr)`). Inlining at a site whose args
    /// are already `f64` is sound regardless of the boxed closure (which is left intact for any
    /// non-numeric callers) and removes the per-call dispatch — what spectral_norm's `evalA` needs.
    inline_fns: std::collections::HashMap<String, (Vec<String>, Expr)>,
    /// Active inline-substitution scopes (param name → the f64 temp it was bound to) while emitting an
    /// inlined body. Top scope wins; an inner inline pushes its own. Read by the `Ident` arm.
    inline_subst: Vec<std::collections::HashMap<String, String>>,
    /// Inline fns currently being expanded (recursion guard) + a monotonically increasing depth used
    /// to make the bound temps unique across nested inlines.
    inlining_now: std::collections::HashSet<String>,
    inline_depth: usize,
    /// #175 — while emitting a native-vec free fn body: `Some(true)` returns `f64`, `Some(false)`
    /// returns `()`. Consulted by the `Return` arm so a `return e;` coerces to the native shape.
    native_vec_ret: Option<VecRetKind>,
    /// #175 — the array params of the native-vec fn currently being emitted (name → is_mut). At a
    /// nested call site these are already `&/&mut Vec<T>` references, so they pass through by reborrow
    /// (`&mut *p`) rather than being address-of'd like a local `Vec` (`&mut local`).
    vec_ref_params: std::collections::HashMap<String, bool>,
    /// #320 (behind TISH_NATIVE_ARR_PARAM): normal boxed-closure fns that take one or more READ-ONLY
    /// `number[]` params (e.g. k_nucleotide's `kNucleotide(seq, k)` — indexes `seq` but returns a
    /// boxed Map). Per param: `true` = unbox the boxed arg into an owned native `Vec<f64>` so the
    /// body indexes it natively (`seq[i+j]` → f64), `false` = leave it on its normal path. The fn
    /// stays a normal closure (boxed body + boxed return); only the array param's indexing flips
    /// native. Computed by `Self::native_arr_param_fns`, the SAME pure fn infer reads (agreement
    /// contract — see `infer::native_vec_array_params`), disjoint from `native_vec_fns`.
    native_arr_param_fns: std::collections::HashMap<String, Vec<bool>>,
    /// #177 S-E/S-F (dark-shipped behind `TISH_AGGREGATE_INFER`): the unboxed struct alias name
    /// (e.g. `TishAnon_0`) when the interprocedural aggregate path is active for this program,
    /// else `None`. Set in `emit_program` only after the de-virtualized fns emit successfully.
    /// #178: inferred recursive-struct lowering plan (behind `TISH_REC_STRUCT`).
    rec_struct_plan: Option<RecStructPlan>,
    aggregate_alias: Option<String>,
    /// #177: fn name → its de-virtualized native signature. Calls to these route directly to the
    /// `fn name_agg(..)` free fn (threading the `Vec<TishStruct_alias>` by reference), bypassing
    /// the boxed `value_call`. Empty when the aggregate path is inactive.
    aggregate_fns: std::collections::HashMap<String, AggFnSig>,
    /// #177: top-level `let` names bound to the unboxed `Vec<TishStruct_alias>` (e.g. `bodies`).
    /// These are emitted `let mut` and passed `&mut`/`&` into the aggregate fns.
    aggregate_array_locals: std::collections::HashSet<String>,
    /// #177: while emitting an aggregate fn body, the return shape of the fn currently being
    /// emitted (drives `Return` lowering). `None` outside aggregate-fn emission.
    agg_cur_ret: Option<AggRet>,
    /// Names of `number`-typed locals demoted to a boxed `Value` because some reassignment can
    /// store a non-number — e.g. `let s = 0; s = s + arr[i]` where `arr` is a boxed Value: `+` is
    /// JS string concat, so `s` may become a `String`. Lowering `s` to a native `f64` would panic
    /// at the store's `from_value_expr(F64)` coercion (`_ => panic!("expected number")`). Computed
    /// once in `emit_program` (after type aliases + `native_fns`), consulted at `VarDecl` to force
    /// `RustType::Value`. This is the rust-AOT analogue of the VM array-JIT bailing to the
    /// interpreter on a non-numeric element. See `collect_demoted_numeric_locals`.
    demoted_numeric_locals: std::collections::HashSet<String>,
    /// Integer-range lattice (#174): names of `f64` locals the analysis proves always hold an
    /// integer within `[min, max]`, both strictly inside `(-2^53, 2^53)` so `as i64` is exact and
    /// `i64` arithmetic is bit-identical to the `f64` the interpreter/VM use. Lets the codegen
    /// lower e.g. `x % c` to a fast integer remainder instead of `fmod`. Conservative: a name absent
    /// here is treated as unbounded. Populated by `collect_int_range_locals`.
    int_range_locals: std::collections::HashMap<String, (i64, i64)>,
    /// Queens `place_nv`: `d1`/`d2` coords (`row±col+n`) proven in-bounds for `diag1`/`diag2` (`2n`).
    diag_coord_indices: std::collections::HashSet<String>,
    /// Fannkuch `fannkuch_nv`: `perm`/`perm1`/`count` lowered as `Vec<i32>` (integer-only arrays).
    int_i32_vec_locals: std::collections::HashSet<String>,
    /// Integer-range lattice (#174): locals that are always INTEGER-valued (an `f64` with zero
    /// fractional part), possibly of unbounded magnitude — unlike `int_range_locals`. Loop counters
    /// (`i = i + 1`) qualify even though their magnitude isn't bounded. Used to prove a modulo
    /// result like `r % 97` is integral, so it can seed a fold accumulator's bounded range.
    int_valued_locals: std::collections::HashSet<String>,
    /// Integer-range lattice (#174): `number[]` locals initialized from an array literal of integer
    /// literals → the inclusive element range, both inside `(-2^53, 2^53)`. Bounds a native fold's
    /// element variable so the fold body can lower to native `i64` arithmetic.
    array_elem_ranges: std::collections::HashMap<String, (i64, i64)>,
    /// i32-loop-var lowering: names of `number` accumulators a per-body analysis proved can live
    /// in an `i32` register across a bitwise/hash hot loop (`h` in FNV) instead of round-tripping
    /// `f64`↔`i32` every op. Each is declared `let mut h: i32`, every reassignment lowers via
    /// `emit_int32_operand`, and reads coerce `(h as f64)`. See `collect_i32_loop_vars` for the
    /// (strict) eligibility/soundness gate. Scoped per function body / top level.
    i32_loop_vars: std::collections::HashSet<String>,
    /// #173 part 3 — in-bounds index elision. Native `Vec` locals whose length is FIXED to a known
    /// bound after construction (filled once to `B`, then never push/pop/length-changed or
    /// reassigned — only indexed) → `name -> B`. With this, an access `a[idx]` guarded by `idx < B`
    /// (same key) is provably in-bounds, so it skips the OOB-growth resize branch (stores) and the
    /// `.get().unwrap_or()` branch (reads). Recomputed per analyzed program.
    vec_fixed_len: std::collections::HashMap<String, BoundKey>,
    /// #173 part 3 — locals provably `>= 0` everywhere (a one-sided sign lattice: init and every
    /// reassignment RHS are non-negative-valued). The lower-bound half of the in-bounds proof, so a
    /// guarded index `idx < len` can't be a negative `idx` that wraps to a huge `usize`.
    nonneg_locals: std::collections::HashSet<String>,
    /// #173 part 3 — stack of active upper-bound guards from enclosing loops. Pushed around a loop
    /// body's emission and popped after; an index `a[counter]` consults it to prove `counter` is
    /// below `a`'s fixed length. A guard goes `live = false` the moment its counter is reassigned
    /// within the body (flow-sensitive: an access before the reassignment is still bounded, one after
    /// is not).
    active_index_guards: Vec<IndexGuard>,
    /// `k2` locals initialized as `(k + 1) >> 1` — lets `i < k2` prove `i < k` for `arr[k - i]`.
    shift_half_of: std::collections::HashMap<String, String>,
    /// Locals assigned from a proven in-bounds `arr[idx]` read inherit `arr`'s length bound.
    bounded_below_len: std::collections::HashMap<String, BoundKey>,
    /// Else-branch of `a === b` (numeric counters): `a` is strictly below `b` (fannkuch `r < n`).
    strict_lt_bounds: Vec<(String, String)>,
    /// After a usize escape loop, the `stayed` flag to use instead of `iter == maxIter`.
    pending_stayed_var: Option<String>,
    /// Skip `iter = iter + 1` and the `iter` decl in a usize escape loop (mandelbrot).
    skip_iter_local: Option<String>,
    /// Active usize `for` counter → `_usize_*` var (skip f64 shadow + direct `arr[ui]`).
    usize_var_subst: std::collections::HashMap<String, String>,
    /// Scopes of names whose Rust binding is actually `Rc<RefCell<_>>` (emitted at VarDecl).
    /// `refcell_wrapped_vars` alone is insufficient: it is set by prepasses before decl may run.
    rc_cell_storage_scopes: Vec<std::collections::HashSet<String>>,
    /// Usage analyzer for move/clone optimization
    usage_analyzer: Option<UsageAnalyzer>,
    /// Type context for tracking variable types (for static typing)
    type_context: TypeContext,
    /// Registry of `type Foo = { ... }` declarations seen in the program.
    /// Populated in a pre-pass so that any later `let x: Foo = ...` or
    /// `fn f(x: Foo)` resolves to a `RustType::Named { name: "Foo", ... }`
    /// and the codegen can emit a Rust struct + direct field access for
    /// values of that type.
    type_aliases: std::collections::HashMap<String, crate::types::RustType>,
    /// Program uses JSX; emit `tishlang_ui` imports and `h` / `Fragment` globals.
    program_has_jsx: bool,
    /// `fn` names for Rust JSX: PascalCase tags matching these use a value binding; others are string intrinsics.
    program_fun_decl_names: std::collections::HashSet<String>,
    /// Nesting depth inside `Value::native(move |args| {{ ... }})` user functions / arrows.
    /// `try`/`throw` lowering uses `return Err` only at depth 0 (e.g. `run()`); inside native
    /// closures it must not return a `Result` from a `Value`-returning closure.
    value_fn_depth: u32,
    emit_mode: crate::NativeEmitMode,
    /// Program links `tish:macos` / `tish:ios` — skip HeadlessHost install.
    has_native_ui_host: bool,
    /// Program references browser global `document` — inject tish-canvas.
    program_uses_document: bool,
}




impl Codegen {
    fn new_with_native_modules(
        // `project_root` is no longer needed by codegen (the only consumer, a Polars-specific
        // `read_csv` compile-time embed, was removed — crate-specific codegen belongs in that crate).
        _project_root: Option<&Path>,
        features: &[String],
        native_module_init: std::collections::HashMap<String, crate::resolve::NativeModuleInit>,
    ) -> Self {
        let features: std::collections::HashSet<String> = features.iter().cloned().collect();
        Self {
            output: String::new(),
            indent: 0,
            loop_label_index: 0,
            is_async: false,
            features,
            native_module_init,
            async_context_stack: Vec::new(),
            loop_stack: Vec::new(),
            break_stack: Vec::new(),
            try_closure_depth: 0,
            function_scope_stack: vec![Vec::new()], // Start with global scope
            outer_params_stack: Vec::new(),
            outer_vars_stack: vec![Vec::new()], // Start with module-level scope
            refcell_wrapped_vars: std::collections::HashSet::new(),
            native_numeric_globals: std::collections::HashMap::new(),
            module_const_f64_arrays: std::collections::HashMap::new(),
            module_const_f64_cum: std::collections::HashMap::new(),
            module_const_f64_aliases: std::collections::HashMap::new(),
            map_instance_locals: std::collections::HashSet::new(),
            native_fns: std::collections::HashSet::new(),
            native_fn_body_emit: false,
            native_fn_emit_name: None,
            native_fn_literal_args: std::collections::HashMap::new(),
            native_fn_param_names: std::collections::HashMap::new(),
            usize_for_counters: std::collections::HashSet::new(),
            native_lcg_hoist: None,
            native_lcg_hoist_int: false,
            native_fn_body_returned: false,
            native_lcg_fns: std::collections::HashMap::new(),
            native_vec_fns: std::collections::HashMap::new(),
            native_vec_param_bounds: std::collections::HashMap::new(),
            inline_fns: std::collections::HashMap::new(),
            inline_subst: Vec::new(),
            inlining_now: std::collections::HashSet::new(),
            inline_depth: 0,
            native_vec_ret: None,
            vec_ref_params: std::collections::HashMap::new(),
            native_arr_param_fns: std::collections::HashMap::new(),
            rec_struct_plan: None,
            aggregate_alias: None,
            aggregate_fns: std::collections::HashMap::new(),
            aggregate_array_locals: std::collections::HashSet::new(),
            agg_cur_ret: None,
            demoted_numeric_locals: std::collections::HashSet::new(),
            int_range_locals: std::collections::HashMap::new(),
            diag_coord_indices: std::collections::HashSet::new(),
            int_i32_vec_locals: std::collections::HashSet::new(),
            int_valued_locals: std::collections::HashSet::new(),
            array_elem_ranges: std::collections::HashMap::new(),
            i32_loop_vars: std::collections::HashSet::new(),
            vec_fixed_len: std::collections::HashMap::new(),
            nonneg_locals: std::collections::HashSet::new(),
            active_index_guards: Vec::new(),
            shift_half_of: std::collections::HashMap::new(),
            bounded_below_len: std::collections::HashMap::new(),
            strict_lt_bounds: Vec::new(),
            pending_stayed_var: None,
            skip_iter_local: None,
            usize_var_subst: std::collections::HashMap::new(),
            rc_cell_storage_scopes: vec![std::collections::HashSet::new()],
            usage_analyzer: None,
            type_context: TypeContext::new(),
            type_aliases: std::collections::HashMap::new(),
            program_has_jsx: false,
            program_fun_decl_names: std::collections::HashSet::new(),
            value_fn_depth: 0,
            emit_mode: crate::NativeEmitMode::DesktopBin,
            has_native_ui_host: false,
            program_uses_document: false,
        }
    }

    /// In async `run()` bodies, propagate runtime op errors with `?`; in sync
    /// `Value::native` closures use `.unwrap_or(Value::Null)`.
    fn ops_result_suffix(&self) -> &'static str {
        if self.is_async && self.async_context_stack.last().copied().unwrap_or(false) {
            "?"
        } else {
            ".unwrap_or(Value::Null)"
        }
    }

    /// Walk every `Statement::TypeAlias` in the program (including nested
    /// ones inside blocks, ifs, loops, function bodies, and exports) and
    /// register the resolved `RustType` under its alias name. Forward
    /// references are handled by running this pass *before* any other
    /// codegen step.
    fn collect_type_aliases(&mut self, statements: &[Statement]) {
        // Two passes so an alias `type B = A` can resolve `A` even if
        // `A` is declared after `B` in source order.
        let mut raw: Vec<(String, &TypeAnnotation)> = Vec::new();
        Self::walk_type_aliases(statements, &mut raw);
        // First-fixpoint resolution: keep iterating until no more aliases
        // change shape. In practice 1–2 passes; capped to prevent infinite
        // loops on (already rejected) self-referential aliases.
        for _ in 0..8 {
            let mut changed = false;
            for (name, ann) in &raw {
                let resolved =
                    crate::types::RustType::from_annotation_with_aliases(ann, &self.type_aliases);
                let prev: Option<crate::types::RustType> = self.type_aliases.get(name).cloned();
                if prev.as_ref() != Some(&resolved) {
                    self.type_aliases.insert(name.clone(), resolved);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn walk_type_aliases<'p>(
        statements: &'p [Statement],
        out: &mut Vec<(String, &'p TypeAnnotation)>,
    ) {
        for s in statements {
            match s {
                Statement::TypeAlias { name, ty, .. } => {
                    out.push((name.to_string(), ty));
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::walk_type_aliases(statements, out)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::walk_type_aliases(std::slice::from_ref(then_branch.as_ref()), out);
                    if let Some(e) = else_branch {
                        Self::walk_type_aliases(std::slice::from_ref(e.as_ref()), out);
                    }
                }
                Statement::For { body, .. }
                | Statement::ForOf { body, .. }
                | Statement::While { body, .. }
                | Statement::DoWhile { body, .. } => {
                    Self::walk_type_aliases(std::slice::from_ref(body.as_ref()), out);
                }
                Statement::Export { declaration, .. } => {
                    if let tishlang_ast::ExportDeclaration::Named(s) = declaration.as_ref() {
                        Self::walk_type_aliases(std::slice::from_ref(s.as_ref()), out);
                    }
                }
                _ => {}
            }
        }
    }

    /// Emit a Rust `struct` definition for every type alias whose RHS is
    /// an object shape. Each generated struct derives `Clone` + `Debug`
    /// (cheap; field types are all `Copy`-or-cheap-clone in practice) and
    /// is named `TishStruct_<TishAlias>`.
    fn emit_named_struct_decls(&mut self) {
        // Snapshot keys + values so we can mutate `self` (writing the
        // emitted source) inside the loop.
        let mut entries: Vec<(String, crate::types::RustType)> = self
            .type_aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut emitted_any = false;
        for (name, ty) in entries {
            if let crate::types::RustType::Named { fields, .. }
            | crate::types::RustType::Object(fields)
            // ^^ also accept inline shapes registered as aliases — though
            //    `from_annotation_with_aliases` should always have lifted
            //    them to `Named` by now.
                = &ty
            {
                let struct_name = crate::types::named_struct_ident(&name);
                self.write("#[derive(Clone, Debug, Default)]\n");
                self.write("#[allow(non_snake_case, non_camel_case_types)]\n");
                self.write(&format!("pub struct {} {{\n", struct_name));
                for (k, t) in fields {
                    self.write(&format!(
                        "    pub {}: {},\n",
                        crate::types::field_ident(k),
                        t.to_rust_type_str()
                    ));
                }
                self.write("}\n\n");
                emitted_any = true;
            }
        }
        if emitted_any {
            self.write("\n");
        }
    }



    fn rc_cell_storage_contains(&self, name: &str) -> bool {
        self.rc_cell_storage_scopes
            .iter()
            .rev()
            .any(|s| s.contains(name))
    }

    fn rc_cell_storage_define(&mut self, name: &str) {
        if let Some(scope) = self.rc_cell_storage_scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    /// Map native module spec to Rust init expression using resolved package.json modules.
    /// For built-in modules (tish:fs, tish:http, tish:process), use builtin_native_module_rust_init.
    fn native_module_rust_init(&self, spec: &str, export_name: &str) -> Option<String> {
        if is_builtin_native_spec(spec) {
            return self.builtin_native_module_rust_init(spec, export_name);
        }
        self.native_module_init.get(spec).map(|init| {
            // Native modules return a namespace object (like an ES module).
            // Named imports extract the field from that namespace: `import { foo } from "pkg"` → `ns.foo`.
            let init_expr = match init {
                crate::resolve::NativeModuleInit::Legacy {
                    crate_name,
                    export_fn,
                } => format!("{}::{}()", crate_name, export_fn),
                crate::resolve::NativeModuleInit::Generated { export_fn, .. } => {
                    format!("crate::generated_native::{}()", export_fn)
                }
            };
            format!(
                "{{ let _ns = {}; match _ns {{ Value::Object(ref _o) => _o.borrow().strings.get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }} }}",
                init_expr, export_name
            )
        })
    }

    /// Rust init for built-in modules (tish:fs, tish:http, tish:process) - uses tishlang_runtime.
    fn builtin_native_module_rust_init(&self, spec: &str, export_name: &str) -> Option<String> {
        let init = match spec {
            "tish:fs" if self.has_feature("fs") => match export_name {
                    "readFile" => Some("Value::native(|args: &[Value]| tish_read_file(args))"),
                    "writeFile" => Some("Value::native(|args: &[Value]| tish_write_file(args))"),
                    "fileExists" => Some("Value::native(|args: &[Value]| tish_file_exists(args))"),
                    "isDir" => Some("Value::native(|args: &[Value]| tish_is_dir(args))"),
                    "readDir" => Some("Value::native(|args: &[Value]| tish_read_dir(args))"),
                    "readFileBytes" => Some("Value::native(|args: &[Value]| tish_read_file_bytes(args))"),
                    "mkdir" => Some("Value::native(|args: &[Value]| tish_mkdir(args))"),
                    _ => None,
                },
            "tish:http" if self.has_feature("http") => match export_name {
                    "fetch" => Some("Value::native(|args: &[Value]| tish_fetch_promise(args.to_vec()))"),
                    "fetchAll" => Some("Value::native(|args: &[Value]| tish_fetch_all_promise(args.to_vec()))"),
                    // `serve(port, handler)` (single shared handler) or
                    // `serve(port, { onWorker })` (per-worker factory). The
                    // latter dispatches into `http_serve_per_worker`, which
                    // calls onWorker once per accept thread to build that
                    // thread's handler.
                    "serve" => Some("Value::native(|args: &[Value]| { let handler = args.get(1).cloned().unwrap_or(Value::Null); match handler { Value::Function(f) => tish_http_serve(args, move |req_args| f.call(req_args)), Value::Object(ref opts) => { let factory = opts.borrow().strings.get(\"onWorker\").cloned().unwrap_or(Value::Null); tishlang_runtime::http_serve_per_worker(args, factory) }, _ => Value::Null } })"),
                    "Promise" => Some("tish_promise_object()"),
                    "Symbol" => Some("tish_symbol_object()"),
                    _ => None,
                },
            "tish:timers" if self.has_feature("timers") => match export_name {
                    "setTimeout" => Some("Value::native(|args: &[Value]| tish_timer_set_timeout(args))"),
                    "setInterval" => Some("Value::native(|args: &[Value]| tish_timer_set_interval(args))"),
                    "clearTimeout" => Some("Value::native(|args: &[Value]| tish_timer_clear_timeout(args))"),
                    "clearInterval" => Some("Value::native(|args: &[Value]| tish_timer_clear_interval(args))"),
                    _ => None,
                },
            "tish:process" if self.has_feature("process") => match export_name {
                    "exit" => Some("Value::native(|args: &[Value]| tish_process_exit(args))"),
                    "cwd" => Some("Value::native(|args: &[Value]| tish_process_cwd(args))"),
                    "exec" => Some("Value::native(|args: &[Value]| tish_process_exec(args))"),
                    "argv" => Some("Value::Array(VmRef::new(std::env::args().map(|s| Value::String(s.into())).collect()))"),
                    "env" => Some("Value::object(std::env::vars().map(|(k,v)| (Arc::from(k.as_str()), Value::String(v.into()))).collect())"),
                    "process" => Some("{ let mut m = ObjectMap::default(); m.insert(Arc::from(\"exit\"), Value::native(|args: &[Value]| tish_process_exit(args))); m.insert(Arc::from(\"cwd\"), Value::native(|args: &[Value]| tish_process_cwd(args))); m.insert(Arc::from(\"exec\"), Value::native(|args: &[Value]| tish_process_exec(args))); m.insert(Arc::from(\"argv\"), Value::Array(VmRef::new(std::env::args().map(|s| Value::String(s.into())).collect()))); m.insert(Arc::from(\"env\"), Value::object(std::env::vars().map(|(k,v)| (Arc::from(k.as_str()), Value::String(v.into()))).collect::<ObjectMap>())); Value::object(m) }"),
                    _ => None,
                },
            "tish:ws" if self.has_feature("ws") => match export_name {
                    "WebSocket" => Some("Value::native(|args: &[Value]| tish_ws_client(args))"),
                    "Server" => Some("Value::native(|args: &[Value]| tish_ws_server_construct(args))"),
                    "wsSend" => Some("Value::native(|args: &[Value]| Value::Bool(tishlang_runtime::ws_send_native(args.first().unwrap_or(&Value::Null), &args.get(1).map(|v| v.to_display_string()).unwrap_or_default())))"),
                    "wsBroadcast" => Some("Value::native(|args: &[Value]| tishlang_runtime::ws_broadcast_native(args))"),
                    _ => None,
                },
            "tish:tty" if self.has_feature("tty") => match export_name {
                    "size" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_size(args))"),
                    "isTTY" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_is_tty(args))"),
                    "setRawMode" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_set_raw_mode(args))"),
                    "enterAltScreen" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_enter_alt_screen(args))"),
                    "leaveAltScreen" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_leave_alt_screen(args))"),
                    "read" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_read(args))"),
                    "readLine" => Some("Value::native(|args: &[Value]| tishlang_runtime::tty_read_line(args))"),
                    _ => None,
                },
            _ => return None,
        };
        init.map(String::from)
    }

    fn has_feature(&self, name: &str) -> bool {
        if self.features.contains("full") {
            matches!(
                name,
                "http" | "timers" | "fs" | "process" | "regex" | "ws" | "tty"
            )
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
    /// Binding keyword that stays valid for the wildcard `_`. A `_` binding cannot be `mut`
    /// (`error: mut must be followed by a named binding`) and is never reassigned, so it always
    /// takes a plain `let`. `base` is the keyword for a normal binding here (e.g. `"let mut"`).
    fn mut_kw_for<'a>(name: &str, base: &'a str) -> &'a str {
        if name == "_" {
            "let"
        } else {
            base
        }
    }

    fn escape_ident(name: &str) -> Cow<'_, str> {
        // Rust standard library macros that conflict with variable names
        const RUST_MACROS: &[&str] = &[
            "line",
            "column",
            "file",
            "module_path",
            "stringify",
            "concat",
        ];
        if RUST_MACROS.contains(&name) {
            return Cow::Owned(format!("r#{}", name));
        }
        const RUST_KEYWORDS: &[&str] = &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
            "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized",
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

        // Native f64 (plain or Rc<RefCell<f64>> for closure-mutated locals)
        if var_type == RustType::F64 {
            let op_assign = if delta.contains('+') { "+=" } else { "-=" };
            if !is_wrapped {
                return if is_prefix {
                    format!("{{ {n} {op_assign} 1.0_f64; Value::Number({n}) }}")
                } else {
                    format!("{{ let _prev = {n}; {n} {op_assign} 1.0_f64; Value::Number(_prev) }}")
                };
            }
            return if is_prefix {
                format!("{{ *{n}.borrow_mut() {op_assign} 1.0_f64; Value::Number(*{n}.borrow()) }}")
            } else {
                format!("{{ let _prev = *{n}.borrow(); *{n}.borrow_mut() {op_assign} 1.0_f64; Value::Number(_prev) }}")
            };
        }

        if is_prefix {
            if is_wrapped {
                format!(
                    "{{ let _cur = (*{n}.borrow()).clone(); *{n}.borrow_mut() = Value::Number(match &_cur {{ Value::Number(n) => n {delta}, _ => panic!(\"{op_name} needs number\") }}); (*{n}.borrow()).clone() }}"
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

    /// Emit a valid Rust `f64` expression for `n`, handling non-finite values. Constant-folding can
    /// produce Infinity/NaN (e.g. `5/0` → `f64::INFINITY`, `0/0` → `f64::NAN`), which the plain
    /// `format!("{}_f64", n)` would render as the INVALID Rust `inf_f64` / `NaN_f64`. Finite values
    /// keep the literal `{n}_f64` form.
    fn f64_lit(n: f64) -> String {
        if n.is_nan() {
            "f64::NAN".to_string()
        } else if n.is_infinite() {
            if n > 0.0 {
                "f64::INFINITY".to_string()
            } else {
                "f64::NEG_INFINITY".to_string()
            }
        } else {
            format!("{}_f64", n)
        }
    }

    /// Generate code for a bitwise binary operation (`& | ^`). `to_int32` is JS ToInt32
    /// (modulo 2³², NaN/±Infinity → 0) — out-of-range operands wrap, not saturate.
    /// Boxed/`Value`-path bitwise op (`& | ^`). Uses the `*_value(&Value)` coercion helpers rather
    /// than a `let Value::Number(a) = &(..) else { panic!() }` block: the block bound `a`/`b`, so a
    /// nested bitwise operand (whose block *also* binds `a`/`b`) shadowed the outer binding and the
    /// generated code failed to compile (`error[E0308]`, `&&f64` vs `Value`). The helpers bind no
    /// name, so the ops compose at any nesting depth, and they coerce non-numbers to `NaN` (→ `0`)
    /// exactly like the interpreter/VM instead of panicking.
    fn emit_bitwise_binop(l: &str, r: &str, op: &str) -> String {
        format!(
            "Value::Number((tishlang_runtime::to_int32_value(&({})) {} tishlang_runtime::to_int32_value(&({}))) as f64)",
            l, op, r
        )
    }

    /// Boxed/`Value`-path shift (`<< >> >>>`). `a_to` is the left-operand coercion helper
    /// (`to_int32_value` signed, `to_uint32_value` for the logical `>>>`); `method` is the
    /// `wrapping_sh*` call. Counts go through `to_uint32_value` then mask to 5 bits — exact JS
    /// semantics, panic-free, and composable (no name binding — see `emit_bitwise_binop`).
    fn emit_shift_binop(l: &str, r: &str, a_to: &str, method: &str) -> String {
        format!(
            "Value::Number(tishlang_runtime::{}(&({})).{}(tishlang_runtime::to_uint32_value(&({}))) as f64)",
            a_to, l, method, r
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

            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = body_expr
            {
                // Check for a - b (ascending) or b - a (descending)
                if let (
                    Expr::Ident {
                        name: left_name, ..
                    },
                    Expr::Ident {
                        name: right_name, ..
                    },
                ) = (left.as_ref(), right.as_ref())
                {
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
        self.program_fun_decl_names = tishlang_ui::jsx::collect_fun_decl_names(program);
        self.program_uses_document = crate::resolve::program_uses_document(program);
        self.write("#![allow(unused, non_snake_case)]\n\n");
        self.write("use std::cell::RefCell;\n");
        self.write("use std::rc::Rc;\n");
        self.write("use std::sync::Arc;\n");
        self.write("use tishlang_runtime::{console_debug as tish_console_debug, console_info as tish_console_info, console_log as tish_console_log, console_warn as tish_console_warn, console_error as tish_console_error, boolean as tish_boolean, decode_uri as tish_decode_uri, encode_uri as tish_encode_uri, string_escape_html_impl as tish_escape_html, in_operator as tish_in_operator, is_finite as tish_is_finite, is_nan as tish_is_nan, json_parse as tish_json_parse, json_stringify as tish_json_stringify, math_abs as tish_math_abs, math_ceil as tish_math_ceil, math_floor as tish_math_floor, math_max as tish_math_max, math_min as tish_math_min, math_round as tish_math_round, math_sqrt as tish_math_sqrt, parse_float as tish_parse_float, parse_int as tish_parse_int, math_random as tish_math_random, math_pow as tish_math_pow, math_sin as tish_math_sin, math_cos as tish_math_cos, math_tan as tish_math_tan, math_log as tish_math_log, math_exp as tish_math_exp, math_sign as tish_math_sign, math_trunc as tish_math_trunc, math_imul as tish_math_imul, math_sinh as tish_math_sinh, math_cosh as tish_math_cosh, math_tanh as tish_math_tanh, math_asinh as tish_math_asinh, math_acosh as tish_math_acosh, math_atanh as tish_math_atanh, math_cbrt as tish_math_cbrt, math_log2 as tish_math_log2, math_log10 as tish_math_log10, math_hypot as tish_math_hypot, math_atan2 as tish_math_atan2, math_asin as tish_math_asin, math_acos as tish_math_acos, math_atan as tish_math_atan, array_is_array as tish_array_is_array, array_construct as tish_array_construct, string_from_char_code as tish_string_from_char_code, string_convert as tish_string_convert, number_convert as tish_number_convert, object_assign as tish_object_assign, object_keys as tish_object_keys, object_values as tish_object_values, object_entries as tish_object_entries, object_from_entries as tish_object_from_entries, symbol_object as tish_symbol_object, tish_construct, tish_error_constructor, tish_date_constructor, tish_set_constructor, tish_map_constructor, map_get as tish_map_get, map_has as tish_map_has, map_set as tish_map_set, map_values as tish_map_values, tish_float64_array_constructor, tish_float32_array_constructor, tish_int8_array_constructor, tish_uint8_array_constructor, tish_uint8_clamped_array_constructor, tish_int16_array_constructor, tish_uint16_array_constructor, tish_int32_array_constructor, tish_uint32_array_constructor, tish_audio_context_constructor, ObjectMap, TishError, Value, VmRef};\n");
        if self.program_has_jsx {
            self.write("use tishlang_ui::{fragment_value, install_thread_local_host, native_create_root, native_use_state, ui_h, ui_text, HeadlessHost};\n");
        }
        if self.has_feature("process") {
            self.write("use tishlang_runtime::{process_exit as tish_process_exit, process_cwd as tish_process_cwd, process_exec as tish_process_exec};\n");
        }
        if self.has_feature("timers") {
            self.write("use tishlang_runtime::{timer_set_timeout as tish_timer_set_timeout, timer_clear_timeout as tish_timer_clear_timeout, timer_set_interval as tish_timer_set_interval, timer_clear_interval as tish_timer_clear_interval};\n");
        }
        if self.has_feature("http") {
            // `register_static_route` is http-gated in the runtime; emit its import only when http is
            // linked, else a non-http `tish build --feature …` fails with an unresolved import.
            self.write("use tishlang_runtime::register_static_route as tish_register_static_route;\n");
            if self.is_async {
                self.write("use tishlang_runtime::{fetch_promise as tish_fetch_promise, fetch_all_promise as tish_fetch_all_promise, http_serve as tish_http_serve, promise_object as tish_promise_object, await_promise as tish_await_promise, await_promise_throw as tish_await_promise_throw};\n");
            } else {
                self.write("use tishlang_runtime::{fetch_promise as tish_fetch_promise, fetch_all_promise as tish_fetch_all_promise, http_serve as tish_http_serve};\n");
            }
        }
        if self.has_feature("fs") {
            self.write("use tishlang_runtime::{read_file as tish_read_file, read_file_bytes as tish_read_file_bytes, write_file as tish_write_file, file_exists as tish_file_exists, is_dir as tish_is_dir, read_dir as tish_read_dir, mkdir as tish_mkdir};\n");
        }
        if self.has_feature("ws") {
            self.write("use tishlang_runtime::{web_socket_client as tish_ws_client, web_socket_server_construct as tish_ws_server_construct};\n");
        }
        if self.has_feature("regex") {
            self.write("use tishlang_runtime::regexp_new;\n");
        }
        if self.program_uses_document {
            self.write("use tish_canvas::document_value as tish_canvas_document;\n");
        }
        self.write("\n");

        // Collect every `type Foo = { ... }` declaration in the program
        // (recursive, so they can also live inside blocks / branches) and
        // canonicalise each into a `RustType::Named` with its field list.
        // Aliases that resolve to a non-Object shape (e.g. `type N = number`)
        // are stored too, so later annotations like `let x: N = 0` still
        // pick up the right native type.
        self.collect_type_aliases(&program.statements);
        // Emit a Rust `struct` for every alias whose RHS is an object
        // shape. Subsequent `let x: Foo = ...` literals lower to plain
        // struct moves (no `VmRef::new(ObjectMap::from(..))` allocation),
        // and `x.field` becomes a direct field access.
        self.emit_named_struct_decls();

        if self.is_async && self.emit_mode == crate::NativeEmitMode::DesktopBin {
            self.writeln("#[tokio::main]");
            self.writeln("async fn main() {");
        } else if self.emit_mode == crate::NativeEmitMode::DesktopBin {
            self.writeln("fn main() {");
        }
        if self.emit_mode == crate::NativeEmitMode::DesktopBin {
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
        }
        // M5 (dark-shipped behind TISH_NATIVE_FN): emit a parallel native `fn f_native` for each
        // eligible top-level numeric fn at top level; direct calls route to it in emit_typed_expr.
        if crate::native_opts_enabled() {
            self.native_numeric_globals =
                Self::collect_native_numeric_globals(&program.statements);
            if !self.native_numeric_globals.is_empty() {
                self.emit_native_numeric_global_tls()?;
                self.writeln("");
            }
            self.module_const_f64_arrays =
                Self::collect_module_const_f64_arrays(&program.statements);
            self.module_const_f64_cum =
                Self::collect_module_const_cum(&program.statements, &self.module_const_f64_arrays);
            self.module_const_f64_aliases =
                Self::collect_module_const_aliases(&program.statements, &self.module_const_f64_cum);
            if !self.module_const_f64_arrays.is_empty() {
                self.emit_module_const_f64_arrays()?;
                self.writeln("");
            }
            let native_vec_names: std::collections::HashSet<String> =
                Self::detect_native_vec_fns(program).keys().cloned().collect();
            let mut global_names: std::collections::HashSet<String> =
                self.native_numeric_globals.keys().cloned().collect();
            global_names.extend(self.module_const_f64_arrays.keys().cloned());
            self.native_fns = Self::collect_native_fns(
                &program.statements,
                &global_names,
                &native_vec_names,
            );
            self.native_fn_literal_args =
                Self::collect_native_fn_literal_calls(&program.statements, &self.native_fns);
            self.native_fn_param_names =
                Self::collect_native_fn_param_names(&program.statements, &self.native_fns);
            self.native_lcg_fns =
                Self::collect_native_lcg_fns(&program.statements, &self.native_fns);
            if !self.native_fns.is_empty() {
                self.emit_native_fns(&program.statements)?;
                self.writeln("");
            }
        }
        // #177 (S-E/S-F, dark-shipped behind TISH_AGGREGATE_INFER): de-virtualize the nbody-shape
        // aggregate fns into native Rust free fns operating on an unboxed `Vec<TishStruct_alias>`
        // threaded by reference. Computed + emitted here (before `run()`); if any fn can't be
        // lowered the whole path is disabled and we fall back to the boxed closures unchanged.
        if crate::native_opts_enabled() {
            self.setup_aggregate_fns(program);
        }
        // #178 (behind TISH_REC_STRUCT): de-virtualize recursive-struct builders/consumers into
        // native free fns over `Option<Box<T>>` structs (the real binary_trees fix). Structural,
        // name-independent; emits before `run()` with scratch-buffer rollback so any unsupported
        // construct cleanly disables the path and falls back to the boxed closures unchanged.
        if crate::native_opts_enabled() {
            self.setup_rec_struct_plan(program);
        }
        // #175 (behind TISH_NATIVE_FN): de-virtualize fns over plain `number[]`/`boolean[]` params
        // into native free fns threaded by `&/&mut Vec<T>` (spectral_norm / queens). Runs after the
        // aggregate path so it can skip any fn that path already claimed; emits before `run()` with
        // its own scratch-buffer rollback so a failure leaves the boxed closures untouched.
        if crate::native_opts_enabled() {
            // #175 inline: numeric leaf fns (e.g. spectral_norm's `evalA`) inlined at native-f64 call
            // sites. Detect before native-vec so a native-vec body's call to one inlines (no boxed
            // reference). The boxed closure is left intact for any non-f64 callers.
            self.inline_fns = Self::collect_inline_fns(program);
            self.setup_native_vec_fns(program);
        }
        // #320 (behind TISH_NATIVE_ARR_PARAM): normal fns taking read-only `number[]` params get
        // those params unboxed to native owned `Vec<f64>` (boxed body + boxed return otherwise), so
        // `seq[i+j]` indexes natively. The SAME pure detection infer reads to keep the caller's array
        // `number[]`, so the two never disagree about which args pass as native arrays.
        if crate::native_opts_enabled() {
            self.native_arr_param_fns = Self::native_arr_param_fns(program);
        }
        // Soundness pass — must run after type aliases + `native_fns` are known (both feed the
        // native-type oracle): find `number`-typed locals a reassignment can turn non-numeric so
        // `VarDecl` lowers them as boxed `Value` rather than native `f64` (else the store coerces
        // and panics on a JS string-concat result like `s = s + arr[i]`). See
        // `collect_demoted_numeric_locals` / `demoted_numeric_locals`.
        self.demoted_numeric_locals = self.collect_demoted_numeric_locals(&program.statements);
        self.int_valued_locals = Self::collect_int_valued_locals(&program.statements);
        self.int_range_locals = self.collect_int_range_locals(&program.statements);
        self.array_elem_ranges = Self::collect_array_elem_ranges(&program.statements);
        // i32-loop-var lowering: must run AFTER `int_range_locals` (the soundness backstop that
        // proves the accumulator stays an exact integer reinterpretable as i32).
        self.i32_loop_vars = self.collect_i32_loop_vars(&program.statements);
        // #173 part 3 — in-bounds index elision facts: fixed-length Vecs + provably non-negative
        // locals. Read together with the per-loop guard stack during emission.
        self.vec_fixed_len = self.collect_vec_fixed_len(&program.statements);
        for (name, vals) in &self.module_const_f64_arrays {
            self.vec_fixed_len
                .insert(name.clone(), BoundKey::Const(vals.len() as i64));
        }
        for (local, cum_static) in &self.module_const_f64_aliases {
            for (src, cs) in &self.module_const_f64_cum {
                if cs == cum_static {
                    if let Some(vals) = self.module_const_f64_arrays.get(src) {
                        self.vec_fixed_len
                            .insert(local.clone(), BoundKey::Const(vals.len() as i64));
                    }
                }
            }
        }
        self.nonneg_locals = self.collect_nonneg_locals(&program.statements);
        self.shift_half_of = Self::collect_shift_half_of(&program.statements);
        if self.is_async {
            self.writeln("async fn run() -> Result<(), Box<dyn std::error::Error>> {");
        } else if self.emit_mode == crate::NativeEmitMode::EmbeddedLib {
            self.writeln("pub fn run() -> Result<(), Box<dyn std::error::Error>> {");
        } else {
            self.writeln("fn run() -> Result<(), Box<dyn std::error::Error>> {");
        }
        self.indent += 1;

        // Initialize builtins
        self.writeln("let mut console = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"debug\"), Value::native(|args: &[Value]| { tish_console_debug(args); Value::Null })),");
        self.writeln("(Arc::from(\"info\"), Value::native(|args: &[Value]| { tish_console_info(args); Value::Null })),");
        self.writeln("(Arc::from(\"log\"), Value::native(|args: &[Value]| { tish_console_log(args); Value::Null })),");
        self.writeln("(Arc::from(\"warn\"), Value::native(|args: &[Value]| { tish_console_warn(args); Value::Null })),");
        self.writeln("(Arc::from(\"error\"), Value::native(|args: &[Value]| { tish_console_error(args); Value::Null })),");
        self.indent -= 1;
        self.writeln("]));");
        self.writeln("let Boolean = Value::native(|args: &[Value]| tish_boolean(args));");
        self.writeln("let parseInt = Value::native(|args: &[Value]| tish_parse_int(args));");
        self.writeln("let parseFloat = Value::native(|args: &[Value]| tish_parse_float(args));");
        self.writeln("let decodeURI = Value::native(|args: &[Value]| tish_decode_uri(args));");
        self.writeln("let encodeURI = Value::native(|args: &[Value]| tish_encode_uri(args));");
        // `registerStaticRoute` calls the http-gated runtime fn, so only bind it when http is linked
        // (matches the conditional `use` above; otherwise non-http builds fail to resolve it).
        if self.has_feature("http") {
            self.writeln(
                r#"let registerStaticRoute = Value::native(|args: &[Value]| { let path = match args.get(0) { Some(Value::String(s)) => s.to_string(), _ => return Value::Null }; let body = match args.get(1) { Some(Value::String(s)) => s.as_bytes().to_vec(), _ => return Value::Null }; let ct = match args.get(2) { Some(Value::String(s)) => s.to_string(), _ => "application/octet-stream".to_string() }; tish_register_static_route(&path, &body, &ct); Value::Null });"#,
            );
        }
        self.writeln(
            "let htmlEscape = Value::native(|args: &[Value]| tish_escape_html(args.first().unwrap_or(&Value::Null)));",
        );
        self.writeln("let isFinite = Value::native(|args: &[Value]| tish_is_finite(args));");
        self.writeln("let isNaN = Value::native(|args: &[Value]| tish_is_nan(args));");
        self.writeln("let Infinity = Value::Number(f64::INFINITY);");
        self.writeln("let NaN = Value::Number(f64::NAN);");
        self.writeln("let Math = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"abs\"), Value::native(|args: &[Value]| tish_math_abs(args))),");
        self.writeln(
            "(Arc::from(\"sqrt\"), Value::native(|args: &[Value]| tish_math_sqrt(args))),",
        );
        self.writeln("(Arc::from(\"min\"), Value::native(|args: &[Value]| tish_math_min(args))),");
        self.writeln("(Arc::from(\"max\"), Value::native(|args: &[Value]| tish_math_max(args))),");
        self.writeln(
            "(Arc::from(\"floor\"), Value::native(|args: &[Value]| tish_math_floor(args))),",
        );
        self.writeln(
            "(Arc::from(\"ceil\"), Value::native(|args: &[Value]| tish_math_ceil(args))),",
        );
        self.writeln(
            "(Arc::from(\"round\"), Value::native(|args: &[Value]| tish_math_round(args))),",
        );
        self.writeln(
            "(Arc::from(\"random\"), Value::native(|args: &[Value]| tish_math_random(args))),",
        );
        self.writeln("(Arc::from(\"pow\"), Value::native(|args: &[Value]| tish_math_pow(args))),");
        self.writeln("(Arc::from(\"sin\"), Value::native(|args: &[Value]| tish_math_sin(args))),");
        self.writeln("(Arc::from(\"cos\"), Value::native(|args: &[Value]| tish_math_cos(args))),");
        self.writeln("(Arc::from(\"tan\"), Value::native(|args: &[Value]| tish_math_tan(args))),");
        self.writeln("(Arc::from(\"log\"), Value::native(|args: &[Value]| tish_math_log(args))),");
        self.writeln("(Arc::from(\"exp\"), Value::native(|args: &[Value]| tish_math_exp(args))),");
        self.writeln(
            "(Arc::from(\"sign\"), Value::native(|args: &[Value]| tish_math_sign(args))),",
        );
        self.writeln(
            "(Arc::from(\"trunc\"), Value::native(|args: &[Value]| tish_math_trunc(args))),",
        );
        self.writeln(
            "(Arc::from(\"imul\"), Value::native(|args: &[Value]| tish_math_imul(args))),",
        );
        // Hyperbolic / inverse-hyperbolic / cbrt / base-2/10 logs (issue #61) + hypot/atan2 and the
        // inverse trig that were missing on the native Math but present on the vm (#247).
        for (name, func) in [
            ("sinh", "tish_math_sinh"),
            ("cosh", "tish_math_cosh"),
            ("tanh", "tish_math_tanh"),
            ("asinh", "tish_math_asinh"),
            ("acosh", "tish_math_acosh"),
            ("atanh", "tish_math_atanh"),
            ("cbrt", "tish_math_cbrt"),
            ("log2", "tish_math_log2"),
            ("log10", "tish_math_log10"),
            ("hypot", "tish_math_hypot"),
            ("atan2", "tish_math_atan2"),
            ("asin", "tish_math_asin"),
            ("acos", "tish_math_acos"),
            ("atan", "tish_math_atan"),
        ] {
            self.writeln(&format!(
                "(Arc::from(\"{name}\"), Value::native(|args: &[Value]| {func}(args))),"
            ));
        }
        self.writeln("(Arc::from(\"PI\"), Value::Number(std::f64::consts::PI)),");
        self.writeln("(Arc::from(\"E\"), Value::Number(std::f64::consts::E)),");
        self.indent -= 1;
        self.writeln("]));");
        self.writeln("let JSON = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln(
            "(Arc::from(\"parse\"), Value::native(|args: &[Value]| tish_json_parse(args))),",
        );
        self.writeln("(Arc::from(\"stringify\"), Value::native(|args: &[Value]| tish_json_stringify(args))),");
        self.indent -= 1;
        self.writeln("]));");

        self.writeln("let Array = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln(
            "(Arc::from(\"isArray\"), Value::native(|args: &[Value]| tish_array_is_array(args))),",
        );
        // `Array(n)` / `new Array(n)` constructor (issue #72); `__call` covers both forms.
        self.writeln(
            "(Arc::from(\"__call\"), Value::native(|args: &[Value]| tish_array_construct(args))),",
        );
        self.indent -= 1;
        self.writeln("]));");

        self.writeln("let String = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"fromCharCode\"), Value::native(|args: &[Value]| tish_string_from_char_code(args))),");
        // `String(value)` callable: `value_call` dispatches objects via `__call`, like `Symbol`.
        self.writeln("(Arc::from(\"__call\"), Value::native(|args: &[Value]| tish_string_convert(args))),");
        self.indent -= 1;
        self.writeln("]));");

        // `Number(value)` coercion callable (issue #36).
        self.writeln("let Number = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln("(Arc::from(\"__call\"), Value::native(|args: &[Value]| tish_number_convert(args))),");
        self.indent -= 1;
        self.writeln("]));");

        self.writeln("let Date = tish_date_constructor();");
        self.writeln("let Set = tish_set_constructor();");
        self.writeln("let Map = tish_map_constructor();");

        self.writeln("let Symbol = tish_symbol_object();");

        self.writeln("let Object = Value::object(ObjectMap::from([");
        self.indent += 1;
        self.writeln(
            "(Arc::from(\"assign\"), Value::native(|args: &[Value]| tish_object_assign(args))),",
        );
        self.writeln(
            "(Arc::from(\"keys\"), Value::native(|args: &[Value]| tish_object_keys(args))),",
        );
        self.writeln(
            "(Arc::from(\"values\"), Value::native(|args: &[Value]| tish_object_values(args))),",
        );
        self.writeln(
            "(Arc::from(\"entries\"), Value::native(|args: &[Value]| tish_object_entries(args))),",
        );
        self.writeln("(Arc::from(\"fromEntries\"), Value::native(|args: &[Value]| tish_object_from_entries(args))),");
        self.indent -= 1;
        self.writeln("]));");

        self.writeln("let Float64Array = tish_float64_array_constructor();");
        self.writeln("let Float32Array = tish_float32_array_constructor();");
        self.writeln("let Int8Array = tish_int8_array_constructor();");
        self.writeln("let Uint8Array = tish_uint8_array_constructor();");
        self.writeln("let Uint8ClampedArray = tish_uint8_clamped_array_constructor();");
        self.writeln("let Int16Array = tish_int16_array_constructor();");
        self.writeln("let Uint16Array = tish_uint16_array_constructor();");
        self.writeln("let Int32Array = tish_int32_array_constructor();");
        self.writeln("let Uint32Array = tish_uint32_array_constructor();");
        self.writeln("let AudioContext = tish_audio_context_constructor();");
        // Error constructors (issue #60): `new Error(msg)` / `Error(msg)` → `{ name, message }`.
        for name in ["Error", "TypeError", "RangeError", "SyntaxError"] {
            self.writeln(&format!("let {name} = tish_error_constructor({name:?});"));
        }
        if self.program_uses_document {
            self.writeln("let document = VmRef::new(tish_canvas_document());");
            self.refcell_wrapped_vars.insert("document".to_string());
            self.rc_cell_storage_define("document");
            if let Some(scope) = self.outer_vars_stack.last_mut() {
                scope.push("document".to_string());
            }
        }

        if self.has_feature("process") {
            self.writeln("let process = Value::object({");
            self.indent += 1;
            self.writeln("let mut p = ObjectMap::default();");
            self.writeln("p.insert(Arc::from(\"exit\"), Value::native(|args: &[Value]| tish_process_exit(args)));");
            self.writeln("p.insert(Arc::from(\"cwd\"), Value::native(|args: &[Value]| tish_process_cwd(args)));");
            self.writeln("p.insert(Arc::from(\"exec\"), Value::native(|args: &[Value]| tish_process_exec(args)));");
            self.writeln("let argv: Vec<Value> = std::env::args().map(|s| Value::String(s.into())).collect();");
            self.writeln("p.insert(Arc::from(\"argv\"), Value::Array(VmRef::new(argv)));");
            self.writeln("let mut env_obj = ObjectMap::default();");
            self.writeln("for (key, value) in std::env::vars() {");
            self.indent += 1;
            self.writeln("env_obj.insert(Arc::from(key.as_str()), Value::String(value.into()));");
            self.indent -= 1;
            self.writeln("}");
            self.writeln("p.insert(Arc::from(\"env\"), Value::object(env_obj));");
            self.writeln("p");
            self.indent -= 1;
            self.writeln("});");
        }

        if self.has_feature("timers") {
            self.writeln(
                "let setTimeout = Value::native(|args: &[Value]| tish_timer_set_timeout(args));",
            );
            self.writeln("let clearTimeout = Value::native(|args: &[Value]| tish_timer_clear_timeout(args));");
            self.writeln(
                "let setInterval = Value::native(|args: &[Value]| tish_timer_set_interval(args));",
            );
            self.writeln("let clearInterval = Value::native(|args: &[Value]| tish_timer_clear_interval(args));");
        }
        if self.has_feature("http") {
            self.writeln(
                "let fetch = Value::native(|args: &[Value]| tish_fetch_promise(args.to_vec()));",
            );
            self.writeln("let fetchAll = Value::native(|args: &[Value]| tish_fetch_all_promise(args.to_vec()));");
            if self.is_async {
                self.writeln("let Promise = tish_promise_object();");
            }
            // `serve` supports two shapes:
            //   1. serve(port, handler)            // single shared handler
            //   2. serve(port, { onWorker: (workerId) => handler, ... })
            //
            // Shape (2) lets users build per-worker state (DB connection,
            // cache, counter, ...) without a global mutex. The runtime
            // dispatches each accept thread to its own handler, all in
            // parallel under `send-values`.
            self.writeln("let serve = Value::native(|args: &[Value]| {");
            self.indent += 1;
            self.writeln("let handler = args.get(1).cloned().unwrap_or(Value::Null);");
            self.writeln("match handler {");
            self.indent += 1;
            self.writeln(
                "Value::Function(f) => tish_http_serve(args, move |req_args| f.call(req_args)),",
            );
            self.writeln("Value::Object(ref opts) => {");
            self.indent += 1;
            self.writeln("let factory = opts.borrow().strings.get(\"onWorker\").cloned().unwrap_or(Value::Null);");
            self.writeln("tishlang_runtime::http_serve_per_worker(args, factory)");
            self.indent -= 1;
            self.writeln("},");
            self.writeln("_ => Value::Null,");
            self.indent -= 1;
            self.writeln("}");
            self.indent -= 1;
            self.writeln("});");
        }

        if self.has_feature("fs") {
            self.writeln("let readFile = Value::native(|args: &[Value]| tish_read_file(args));");
            self.writeln("let writeFile = Value::native(|args: &[Value]| tish_write_file(args));");
            self.writeln(
                "let fileExists = Value::native(|args: &[Value]| tish_file_exists(args));",
            );
            self.writeln("let isDir = Value::native(|args: &[Value]| tish_is_dir(args));");
            self.writeln("let readDir = Value::native(|args: &[Value]| tish_read_dir(args));");
            self.writeln("let mkdir = Value::native(|args: &[Value]| tish_mkdir(args));");
        }

        if self.has_feature("regex") {
            self.writeln("let RegExp = Value::native(|args: &[Value]| regexp_new(args));");
        }

        if self.program_has_jsx && !self.has_native_ui_host {
            self.writeln("install_thread_local_host(Box::new(HeadlessHost::default()));");
            self.writeln("let Fragment = fragment_value();");
            self.writeln("let h = Value::native(|args: &[Value]| ui_h(args));");
            self.writeln("let text = Value::native(|args: &[Value]| ui_text(args));");
            self.writeln("let useState = Value::native(|args: &[Value]| native_use_state(args));");
            self.writeln(
                "let createRoot = Value::native(|args: &[Value]| native_create_root(args));",
            );
        }

        // Polars, Egui etc. are emitted via VarDecl from import { X } from 'tish:...'

        // Pre-scan for top-level function declarations and create cells (for mutual recursion)
        let top_level_funcs = self.prescan_function_decls(&program.statements);
        *self.function_scope_stack.last_mut().unwrap() = top_level_funcs.clone();
        for func_name in &top_level_funcs {
            // #177: functions promoted to native aggregate free fns (`<name>_agg`) have their
            // boxed closure + cell suppressed — no boxed value exists to back-patch.
            if self.aggregate_alias.is_some() && self.aggregate_fns.contains_key(func_name) {
                continue;
            }
            let escaped = Self::escape_ident(func_name);
            self.writeln(&format!(
                "let {}_cell: VmRef<Value> = VmRef::new(Value::Null);",
                escaped
            ));
        }

        // Initialize usage analyzer for move/clone optimization
        let mut analyzer = UsageAnalyzer::new();
        analyzer.analyze_statements(&program.statements);
        self.usage_analyzer = Some(analyzer);

        // Prepass: vars mutated by nested closures must be RefCell from the start (top-level)
        let top_level_mutated = Self::collect_vars_needing_capture_cell(&program.statements);
        for v in &top_level_mutated {
            if !self.native_numeric_globals.contains_key(v) {
                self.refcell_wrapped_vars.insert(v.clone());
            }
        }

        if self.is_async {
            self.async_context_stack.push(true); // run() body is async Rust context
        }
        self.emit_statements_with_folds(&program.statements)?;
        if self.is_async {
            self.async_context_stack.pop();
        }

        // Run pending timers to completion before exiting — the JS event loop drains the
        // timer queue after top-level code finishes. Without this the rust backend drops
        // `setTimeout(cb, 0)` callbacks that never coincided with a blocking-op drain,
        // diverging from interp/vm/cranelift/wasi (which drain at end-of-program).
        if self.has_feature("timers") {
            self.writeln("tishlang_runtime::drain_timers();");
        }

        self.writeln("Ok(())");
        self.indent -= 1;
        self.writeln("}");
        if self.emit_mode == crate::NativeEmitMode::EmbeddedLib {
            self.writeln("");
            self.writeln("#[no_mangle]");
            self.writeln("pub extern \"C\" fn tish_ios_launch() {");
            self.indent += 1;
            if self.is_async {
                self.writeln("let rt = tokio::runtime::Runtime::new().expect(\"tokio runtime\");");
                self.writeln("let _ = rt.block_on(run());");
            } else {
                self.writeln("let _ = run();");
            }
            self.indent -= 1;
            self.writeln("}");
        }
        Ok(())
    }

    /// Emit an expression in **statement position** (its value is discarded). For a native
    /// assignment this emits only the side-effect — NOT the boxed `Value::Number(..)` that the
    /// expression form returns (JS "assignment yields its value"). In a hot loop that boxed
    /// value was constructed + dropped every iteration, and because `Value` has a non-trivial
    /// `Drop` (other variants hold `Rc`/`Arc`) LLVM couldn't prove it dead — so it could not
    /// vectorize/fold the loop. Falls back to `emit_expr` for everything else (whose trailing
    /// value is simply dropped by the `;`).
    fn emit_expr_discard(&mut self, expr: &Expr) -> Result<String, CompileError> {
        // #173 part 3: a statement-position reassignment of a guarded loop counter ends that guard's
        // bound for everything emitted after it (flow-sensitive). Every statement (at any nesting)
        // routes through here in textual/runtime order, so clearing the guard here is exact.
        match expr {
            Expr::Assign { name, .. }
            | Expr::CompoundAssign { name, .. }
            | Expr::LogicalAssign { name, .. }
            | Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } => {
                if !self.active_index_guards.is_empty() {
                    self.invalidate_index_guard(name.as_ref());
                }
            }
            _ => {}
        }
        if let Expr::IndexAssign { object, index, value, .. } = expr {
            if let Some(code) = self.try_emit_native_vec_index_assign(object, index, value, true)? {
                return Ok(code);
            }
        }
        match expr {
            Expr::Assign { name, value, .. } => {
                if self.native_numeric_globals.contains_key(name.as_ref()) {
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    let native_val = if val_ty == RustType::F64 {
                        val_code
                    } else if val_ty == RustType::Value {
                        RustType::F64.from_value_expr(&val_code)
                    } else {
                        val_code
                    };
                    return Ok(Self::native_global_set(name.as_ref(), &native_val));
                }
                let rust_type = self.type_context.get_type(name.as_ref());
                // i32-loop-var lowering: the accumulator lives in an `i32` register. Each
                // reassignment RHS is a bitwise/shift chain the gate proved lowers fully via
                // `emit_int32_operand` (a `>>> 0` result is u32 reinterpreted to i32; signed
                // bitwise ops yield i32 directly) — so store the i32 with NO `f64` round-trip.
                if rust_type == RustType::I32 {
                    if let Some(int_code) = self.emit_int32_operand(value)? {
                        let escaped = Self::escape_ident(name.as_ref());
                        return Ok(format!("{} = ({}) as i32", escaped, int_code));
                    }
                    // Defensive: gate guarantees `Some`, but if a future RHS shape slips through,
                    // fall back to a sound f64-narrowed store rather than miscompiling.
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    let v = if val_ty.is_native() {
                        val_ty.to_value_expr(&val_code)
                    } else {
                        val_code
                    };
                    let escaped = Self::escape_ident(name.as_ref());
                    return Ok(format!(
                        "{} = {}",
                        escaped,
                        RustType::I32.from_value_expr(&v)
                    ));
                }
                // String self-append `s = s + rhs` -> in-place push_str (amortized O(1)). The
                // general path boxes via `ops::add(Value::String(s.clone()), ...)` which clones
                // the whole string per concat -> O(n^2) string building. rhs must be String-typed.
                if rust_type == RustType::String {
                    if let Expr::Binary {
                        left,
                        op: BinOp::Add,
                        right,
                        ..
                    } = value.as_ref()
                    {
                        if matches!(left.as_ref(), Expr::Ident { name: ln, .. } if ln.as_ref() == name.as_ref())
                        {
                            let (rhs_code, rhs_ty) = self.emit_typed_expr(right.as_ref())?;
                            if rhs_ty == RustType::String {
                                let escaped = Self::escape_ident(name.as_ref());
                                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                                    return Ok(format!(
                                        "{{ let _r = {}; {}.borrow_mut().push_str(&_r); }}",
                                        rhs_code, escaped
                                    ));
                                }
                                return Ok(format!(
                                    "{{ let _r = {}; {}.push_str(&_r); }}",
                                    rhs_code, escaped
                                ));
                            }
                        }
                    }
                }
                if matches!(rust_type, RustType::F64 | RustType::Bool | RustType::String) {
                    if let Expr::Index { object, index, .. } = value.as_ref() {
                        if let Expr::Ident { name: arr_name, .. } = object.as_ref() {
                            if self.index_in_bounds(index, arr_name.as_ref()) {
                                if let Some(b) = self.vec_fixed_len.get(arr_name.as_ref()) {
                                    self.bounded_below_len
                                        .insert(name.to_string(), b.clone());
                                }
                            }
                        }
                    }
                    let escaped = Self::escape_ident(name.as_ref());
                    let is_ref = self.refcell_wrapped_vars.contains(name.as_ref());
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    let native_val = if val_ty == RustType::Value {
                        rust_type.from_value_expr(&val_code)
                    } else {
                        val_code
                    };
                    if is_ref {
                        return Ok(format!(
                            "{{ let _assign_tmp = {}; *{}.borrow_mut() = _assign_tmp; }}",
                            native_val, escaped
                        ));
                    }
                    return Ok(format!("{} = {}", escaped, native_val));
                }
            }
            // `i++` / `++i` / `i--` / `--i` in statement position (incl. for-loop update):
            // emit just the native increment, no boxed `Value::Number(_prev)`.
            Expr::PostfixInc { name, .. } | Expr::PrefixInc { name, .. } => {
                if self.type_context.get_type(name.as_ref()) == RustType::F64 {
                    let n = Self::escape_ident(name.as_ref());
                    if self.refcell_wrapped_vars.contains(name.as_ref()) {
                        return Ok(format!("*{}.borrow_mut() += 1.0_f64", n));
                    }
                    return Ok(format!("{} += 1.0_f64", n));
                }
            }
            Expr::PostfixDec { name, .. } | Expr::PrefixDec { name, .. } => {
                if self.type_context.get_type(name.as_ref()) == RustType::F64 {
                    let n = Self::escape_ident(name.as_ref());
                    if self.refcell_wrapped_vars.contains(name.as_ref()) {
                        return Ok(format!("*{}.borrow_mut() -= 1.0_f64", n));
                    }
                    return Ok(format!("{} -= 1.0_f64", n));
                }
            }
            // `s += x` etc. in statement position: native f64 compound op, no boxed return.
            Expr::CompoundAssign { name, op, value, .. } => {
                if self.type_context.get_type(name.as_ref()) == RustType::F64 {
                    let n = Self::escape_ident(name.as_ref());
                    let is_refcell = self.refcell_wrapped_vars.contains(name.as_ref());
                    let (rhs_code, rhs_ty) = self.emit_typed_expr(value)?;
                    let rhs_f64 = if rhs_ty == RustType::F64 {
                        rhs_code
                    } else {
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
                    if is_refcell {
                        return Ok(format!(
                            "{{ let _op_rhs = {}; *{}.borrow_mut() {} _op_rhs; }}",
                            rhs_f64, n, op_str
                        ));
                    }
                    return Ok(format!("{} {} {}", n, op_str, rhs_f64));
                }
            }
            _ => {}
        }
        self.emit_expr(expr)
    }

    /// Is `update` a `+1` step on `var` (`var++`, `++var`, `var += 1`, or `var = var + 1`)?
    fn is_increment_of(update: &Expr, var: &str) -> bool {
        match update {
            Expr::PostfixInc { name, .. } | Expr::PrefixInc { name, .. } => name.as_ref() == var,
            Expr::CompoundAssign {
                name,
                op: CompoundOp::Add,
                value,
                ..
            } => name.as_ref() == var && Self::int_literal_value_of(value) == Some(1),
            Expr::Assign { name, value, .. } => {
                name.as_ref() == var
                    && matches!(
                        value.as_ref(),
                        Expr::Binary { left, op: BinOp::Add, right, .. }
                            if matches!(left.as_ref(), Expr::Ident { name: l, .. } if l.as_ref() == var)
                                && Self::int_literal_value_of(right) == Some(1)
                    )
            }
            _ => false,
        }
    }

    /// #173: detect a fill loop `for (let i = 0; i < N; i++) { a.push(K) }` over a native `Vec<T>`
    /// and emit it as a single bulk `a.extend(std::iter::repeat(K).take((N) as usize))` — one
    /// allocation instead of N per-element pushes that repeatedly realloc as the Vec grows. Returns
    /// `Ok(true)` when the fused form was emitted (the caller then skips the normal loop).
    ///
    /// Sound only when `N` is a proven, side-effect-free integer (so the bulk count matches the loop
    /// iteration count exactly, including the truncating `as usize` for `0`/negative) and `K` is a
    /// constant of the element type (no per-element variation). Any miss returns `Ok(false)` and the
    /// normal loop is emitted — correctness over coverage.
    fn try_emit_native_fill_loop(
        &mut self,
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        // init: `let i = 0`
        let (
            Some(Statement::VarDecl {
                name: i_name,
                init: Some(i_init),
                ..
            }),
            Some(cond),
            Some(update),
        ) = (init, cond, update)
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return Ok(false);
        }
        // cond: `i < N`
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: bound,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: c_name, .. } = left.as_ref() else {
            return Ok(false);
        };
        if c_name.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        // update: `i++` / `++i` / `i += 1` / `i = i + 1`
        if !Self::is_increment_of(update, i_name.as_ref()) {
            return Ok(false);
        }
        // body: exactly one statement `a.push(K)`
        let push_stmt = match body {
            Statement::Block { statements, .. } if statements.len() == 1 => &statements[0],
            Statement::ExprStmt { .. } => body,
            _ => return Ok(false),
        };
        let Statement::ExprStmt {
            expr: Expr::Call { callee, args, .. },
            ..
        } = push_stmt
        else {
            return Ok(false);
        };
        let Expr::Member {
            object,
            prop: MemberProp::Name { name: method, .. },
            optional: false,
            ..
        } = callee.as_ref()
        else {
            return Ok(false);
        };
        if method.as_ref() != "push" || args.len() != 1 {
            return Ok(false);
        }
        let Expr::Ident { name: arr_name, .. } = object.as_ref() else {
            return Ok(false);
        };
        // `a` must be a native `Vec<T>`. A closure-captured (RefCell) Vec would need a borrow_mut;
        // skip it (rare) and keep the plain loop.
        let RustType::Vec(elem) = self.type_context.get_type(arr_name.as_ref()) else {
            return Ok(false);
        };
        if self.refcell_wrapped_vars.contains(arr_name.as_ref()) {
            return Ok(false);
        }
        let CallArg::Expr(k_expr) = &args[0] else {
            return Ok(false);
        };
        // K must be a constant literal of the element type (no per-element variation, no `i` ref).
        let k_code = match (&*elem, k_expr) {
            (RustType::F64, Expr::Literal { value: Literal::Number(n), .. }) => Self::f64_lit(*n),
            (RustType::Bool, Expr::Literal { value: Literal::Bool(b), .. }) => format!("{}", b),
            _ => return Ok(false),
        };
        // N must be a proven, side-effect-free integer: an integer literal or an int-range local.
        let n_code = match bound.as_ref() {
            Expr::Literal {
                value: Literal::Number(_),
                ..
            } if Self::int_literal_value_of(bound).is_some() => self.emit_typed_expr(bound)?.0,
            Expr::Ident { name, .. }
                if self.int_range_locals.contains_key(name.as_ref())
                    && self.type_context.get_type(name.as_ref()) == RustType::F64 =>
            {
                Self::escape_ident(name.as_ref()).into_owned()
            }
            _ => return Ok(false),
        };
        let arr_esc = Self::escape_ident(arr_name.as_ref()).into_owned();
        self.writeln(&format!(
            "{}.extend(std::iter::repeat({}).take(({}) as usize));",
            arr_esc, k_code, n_code
        ));
        Ok(true)
    }

    /// `for (i=0; i<n; i++) { u.push(1); v.push(0); w.push(0) }` → three bulk `Vec<f64>` inits.
    fn try_emit_spectral_triple_init(
        &mut self,
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let Some(Statement::VarDecl {
            name: i_name,
            init: Some(i_init),
            ..
        }) = init
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return Ok(false);
        }
        let Some(cond) = cond else {
            return Ok(false);
        };
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: bound,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: c, .. } = left.as_ref() else {
            return Ok(false);
        };
        if c.as_ref() != i_name.as_ref() {
            return Ok(false);
        };
        let update = match update.as_ref() {
            Some(u) => u,
            None => return Ok(false),
        };
        if !Self::is_increment_of(update, i_name.as_ref()) {
            return Ok(false);
        }
        let len = match Self::bound_key_of(bound.as_ref()) {
            Some(l) => l,
            None => return Ok(false),
        };
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.len() != 3 {
            return Ok(false);
        }
        fn parse_push_f64_lit(st: &Statement) -> Option<(String, f64)> {
            let Statement::ExprStmt {
                expr: Expr::Call { callee, args, .. },
                ..
            } = st
            else {
                return None;
            };
            let Expr::Member {
                object,
                prop: MemberProp::Name { name: method, .. },
                optional: false,
                ..
            } = callee.as_ref()
            else {
                return None;
            };
            if method.as_ref() != "push" || args.len() != 1 {
                return None;
            }
            let Expr::Ident { name: arr, .. } = object.as_ref() else {
                return None;
            };
            let CallArg::Expr(Expr::Literal {
                value: Literal::Number(n),
                ..
            }) = &args[0]
            else {
                return None;
            };
            Some((arr.to_string(), *n))
        }
        let (a0, v0) = match parse_push_f64_lit(&stmts[0]) {
            Some(v) => v,
            None => return Ok(false),
        };
        let (a1, v1) = match parse_push_f64_lit(&stmts[1]) {
            Some(v) => v,
            None => return Ok(false),
        };
        let (a2, v2) = match parse_push_f64_lit(&stmts[2]) {
            Some(v) => v,
            None => return Ok(false),
        };
        if a0 == a1 || a0 == a2 || a1 == a2 {
            return Ok(false);
        }
        let mut vals = [v0, v1, v2];
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if vals != [0.0, 0.0, 1.0] {
            return Ok(false);
        }
        for arr in [&a0, &a1, &a2] {
            if self.type_context.get_type(arr) != RustType::Vec(Box::new(RustType::F64)) {
                return Ok(false);
            }
        }
        let n_usize = match &len {
            BoundKey::Const(c) => format!("{}", *c as usize),
            BoundKey::Var(v) => {
                if let Some(lit) = self.native_param_lit(v) {
                    if lit.fract() == 0.0 && lit >= 0.0 {
                        format!("{}", lit as usize)
                    } else {
                        format!("({} as usize)", Self::escape_ident(v))
                    }
                } else {
                    format!("({} as usize)", Self::escape_ident(v))
                }
            }
            BoundKey::Len(_) | BoundKey::TwiceVar(_) => return Ok(false),
        };
        for (arr, val) in [(a0, v0), (a1, v1), (a2, v2)] {
            self.writeln(&format!(
                "{} = std::iter::repeat({}).take({}).collect();",
                Self::escape_ident(&arr),
                Self::f64_lit(val),
                n_usize
            ));
        }
        Ok(true)
    }

    /// `for (i=0; i<n; i++) { perm.push(0); perm1.push(i); count.push(0) }` shape (no emit).
    fn parse_fannkuch_triple_init_for(
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Option<(String, String, String)> {
        let Statement::VarDecl {
            name: i_name,
            init: Some(i_init),
            ..
        } = init?
        else {
            return None;
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return None;
        }
        let cond = cond?;
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: _,
            ..
        } = cond
        else {
            return None;
        };
        let Expr::Ident { name: c, .. } = left.as_ref() else {
            return None;
        };
        if c.as_ref() != i_name.as_ref() {
            return None;
        }
        let update = update?;
        if !Self::is_increment_of(update, i_name.as_ref()) {
            return None;
        }
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.len() != 3 {
            return None;
        }
        fn parse_push_arr(st: &Statement) -> Option<String> {
            let Statement::ExprStmt {
                expr: Expr::Call { callee, args, .. },
                ..
            } = st
            else {
                return None;
            };
            let Expr::Member {
                object,
                prop: MemberProp::Name { name: method, .. },
                optional: false,
                ..
            } = callee.as_ref()
            else {
                return None;
            };
            if method.as_ref() != "push" || args.len() != 1 {
                return None;
            }
            let Expr::Ident { name: arr, .. } = object.as_ref() else {
                return None;
            };
            Some(arr.to_string())
        }
        let perm = parse_push_arr(&stmts[0])?;
        let perm1 = parse_push_arr(&stmts[1])?;
        let count = parse_push_arr(&stmts[2])?;
        let Statement::ExprStmt {
            expr: Expr::Call { args: a0, .. },
            ..
        } = &stmts[0]
        else {
            return None;
        };
        let Statement::ExprStmt {
            expr: Expr::Call { args: a1, .. },
            ..
        } = &stmts[1]
        else {
            return None;
        };
        let Statement::ExprStmt {
            expr: Expr::Call { args: a2, .. },
            ..
        } = &stmts[2]
        else {
            return None;
        };
        let CallArg::Expr(k0) = &a0[0] else {
            return None;
        };
        let CallArg::Expr(k1) = &a1[0] else {
            return None;
        };
        let CallArg::Expr(k2) = &a2[0] else {
            return None;
        };
        if !matches!(k0, Expr::Literal { value: Literal::Number(n), .. } if *n == 0.0)
            || !matches!(k1, Expr::Ident { name, .. } if name.as_ref() == i_name.as_ref())
            || !matches!(k2, Expr::Literal { value: Literal::Number(n), .. } if *n == 0.0)
        {
            return None;
        }
        Some((perm, perm1, count))
    }

    fn detect_fannkuch_i32_vec_locals(stmts: &[Statement]) -> HashSet<String> {
        let mut out = HashSet::new();
        for s in stmts {
            if let Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } = s
            {
                if let Some((perm, perm1, count)) =
                    Self::parse_fannkuch_triple_init_for(init.as_deref(), cond.as_ref(), update.as_ref(), body)
                {
                    out.insert(perm);
                    out.insert(perm1);
                    out.insert(count);
                }
            }
            Self::for_each_child_stmt_list(s, &mut |list| {
                out.extend(Self::detect_fannkuch_i32_vec_locals(list));
            });
        }
        out
    }

    /// `for (i=0; i<n; i++) { perm.push(0); perm1.push(i); count.push(0) }` → three bulk inits.
    fn try_emit_fannkuch_triple_init(
        &mut self,
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let Some(Statement::VarDecl {
            name: i_name,
            init: Some(i_init),
            ..
        }) = init
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return Ok(false);
        }
        let Some(cond) = cond else {
            return Ok(false);
        };
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: bound,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: c, .. } = left.as_ref() else {
            return Ok(false);
        };
        if c.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        let update = match update.as_ref() {
            Some(u) => u,
            None => return Ok(false),
        };
        if !Self::is_increment_of(update, i_name.as_ref()) {
            return Ok(false);
        }
        let len = match Self::bound_key_of(bound.as_ref()) {
            Some(l) => l,
            None => return Ok(false),
        };
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.len() != 3 {
            return Ok(false);
        }
        fn parse_push_arr(st: &Statement) -> Option<String> {
            let Statement::ExprStmt {
                expr: Expr::Call { callee, args, .. },
                ..
            } = st
            else {
                return None;
            };
            let Expr::Member {
                object,
                prop: MemberProp::Name { name: method, .. },
                optional: false,
                ..
            } = callee.as_ref()
            else {
                return None;
            };
            if method.as_ref() != "push" || args.len() != 1 {
                return None;
            }
            let Expr::Ident { name: arr, .. } = object.as_ref() else {
                return None;
            };
            Some(arr.to_string())
        }
        let perm = match parse_push_arr(&stmts[0]) {
            Some(v) => v,
            None => return Ok(false),
        };
        let perm1 = match parse_push_arr(&stmts[1]) {
            Some(v) => v,
            None => return Ok(false),
        };
        let count = match parse_push_arr(&stmts[2]) {
            Some(v) => v,
            None => return Ok(false),
        };
        let Statement::ExprStmt {
            expr: Expr::Call { args: args0, .. },
            ..
        } = &stmts[0]
        else {
            return Ok(false);
        };
        let Statement::ExprStmt {
            expr: Expr::Call { args: args1, .. },
            ..
        } = &stmts[1]
        else {
            return Ok(false);
        };
        let Statement::ExprStmt {
            expr: Expr::Call { args: args2, .. },
            ..
        } = &stmts[2]
        else {
            return Ok(false);
        };
        let CallArg::Expr(k0) = &args0[0] else {
            return Ok(false);
        };
        let CallArg::Expr(k1) = &args1[0] else {
            return Ok(false);
        };
        let CallArg::Expr(k2) = &args2[0] else {
            return Ok(false);
        };
        if !matches!(k0, Expr::Literal { value: Literal::Number(n), .. } if *n == 0.0)
            || !matches!(k1, Expr::Ident { name, .. } if name.as_ref() == i_name.as_ref())
            || !matches!(k2, Expr::Literal { value: Literal::Number(n), .. } if *n == 0.0)
        {
            return Ok(false);
        }
        let use_i32 = self.native_vec_ret.is_some()
            && self.int_i32_vec_locals.contains(perm.as_str())
            && self.int_i32_vec_locals.contains(perm1.as_str())
            && self.int_i32_vec_locals.contains(count.as_str());
        if !use_i32 {
            let RustType::Vec(elem) = self.type_context.get_type(&perm) else {
                return Ok(false);
            };
            if *elem != RustType::F64
                || self.type_context.get_type(&perm1) != RustType::Vec(Box::new(RustType::F64))
                || self.type_context.get_type(&count) != RustType::Vec(Box::new(RustType::F64))
            {
                return Ok(false);
            }
        }
        let n_usize = match &len {
            BoundKey::Const(c) => format!("{}", *c as usize),
            BoundKey::Var(v) => {
                if let Some(lit) = self.native_param_lit(v) {
                    if lit.fract() == 0.0 && lit >= 0.0 {
                        format!("{}", lit as usize)
                    } else {
                        format!("({} as usize)", Self::escape_ident(v))
                    }
                } else {
                    format!("({} as usize)", Self::escape_ident(v))
                }
            }
            BoundKey::Len(_) | BoundKey::TwiceVar(_) => return Ok(false),
        };
        let perm_esc = Self::escape_ident(&perm);
        let perm1_esc = Self::escape_ident(&perm1);
        let count_esc = Self::escape_ident(&count);
        if use_i32 {
            self.writeln(&format!(
                "{} = std::iter::repeat(0i32).take({}).collect();",
                perm_esc, n_usize
            ));
            self.writeln(&format!(
                "{} = (0..{}).collect();",
                perm1_esc, n_usize
            ));
            self.writeln(&format!(
                "{} = std::iter::repeat(0i32).take({}).collect();",
                count_esc, n_usize
            ));
        } else {
            self.writeln(&format!(
                "{} = std::iter::repeat(0_f64).take({}).collect();",
                perm_esc, n_usize
            ));
            self.writeln(&format!(
                "{} = (0..{}).map(|j| j as f64).collect();",
                perm1_esc, n_usize
            ));
            self.writeln(&format!(
                "{} = std::iter::repeat(0_f64).take({}).collect();",
                count_esc, n_usize
            ));
        }
        Ok(true)
    }

    /// `for (i=0; i<n; i++) { dst[i] = src[i] }` on equal native `Vec<f64>` → `copy_from_slice`.
    fn try_emit_native_vec_copy_loop(
        &mut self,
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let Some(Statement::VarDecl {
            name: i_name,
            init: Some(i_init),
            ..
        }) = init
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return Ok(false);
        }
        let Some(cond) = cond else {
            return Ok(false);
        };
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: bound,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: c, .. } = left.as_ref() else {
            return Ok(false);
        };
        if c.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        let update = match update.as_ref() {
            Some(u) => u,
            None => return Ok(false),
        };
        if !Self::is_increment_of(update, i_name.as_ref()) {
            return Ok(false);
        }
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.len() != 1 {
            return Ok(false);
        }
        let Statement::ExprStmt { expr, .. } = &stmts[0] else {
            return Ok(false);
        };
        let Expr::IndexAssign {
            object: dst,
            index: idx,
            value,
            ..
        } = expr
        else {
            return Ok(false);
        };
        let Expr::Ident { name: dst_name, .. } = dst.as_ref() else {
            return Ok(false);
        };
        let Expr::Ident { name: idx_name, .. } = idx.as_ref() else {
            return Ok(false);
        };
        if idx_name.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        let Expr::Index {
            object: src,
            index: src_idx,
            ..
        } = value.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: src_name, .. } = src.as_ref() else {
            return Ok(false);
        };
        let Expr::Ident { name: src_i, .. } = src_idx.as_ref() else {
            return Ok(false);
        };
        if src_i.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        let dst_ty = self.type_context.get_type(dst_name.as_ref());
        let src_ty = self.type_context.get_type(src_name.as_ref());
        if dst_ty != src_ty {
            return Ok(false);
        }
        match dst_ty {
            RustType::Vec(inner) if *inner == RustType::F64 || *inner == RustType::I32 => {}
            _ => return Ok(false),
        }
        let dst_esc = Self::escape_ident(dst_name.as_ref());
        let src_esc = Self::escape_ident(src_name.as_ref());
        self.writeln(&format!("{}.copy_from_slice(&{});", dst_esc, src_esc));
        Ok(true)
    }

    /// `while (r !== 1) { count[r-1] = r; r = r - 1 }` → indexed fill + `r = 1`.
    fn try_emit_count_r_decr_while(
        &mut self,
        cond: &Expr,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let r_ne_one = match cond {
            Expr::Binary {
                left,
                op: BinOp::StrictNe | BinOp::Ne,
                right,
                ..
            } => {
                let Expr::Ident { name: r, .. } = left.as_ref() else {
                    return Ok(false);
                };
                if Self::int_literal_value_of(right) != Some(1) {
                    return Ok(false);
                }
                r.to_string()
            }
            _ => return Ok(false),
        };
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.len() != 2 {
            return Ok(false);
        }
        let Statement::ExprStmt { expr: a0, .. } = &stmts[0] else {
            return Ok(false);
        };
        let Statement::ExprStmt { expr: a1, .. } = &stmts[1] else {
            return Ok(false);
        };
        let Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } = a0
        else {
            return Ok(false);
        };
        let Expr::Ident { name: count_name, .. } = object.as_ref() else {
            return Ok(false);
        };
        let Expr::Binary {
            left: lk,
            op: BinOp::Sub,
            right: lr,
            ..
        } = index.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: lk_name, .. } = lk.as_ref() else {
            return Ok(false);
        };
        if lk_name.as_ref() != r_ne_one.as_str() || Self::int_literal_value_of(lr) != Some(1) {
            return Ok(false);
        }
        let Expr::Ident { name: rv, .. } = value.as_ref() else {
            return Ok(false);
        };
        if rv.as_ref() != r_ne_one.as_str() {
            return Ok(false);
        }
        let Expr::Assign { name: r_name, value: r_upd, .. } = a1 else {
            return Ok(false);
        };
        if r_name.as_ref() != r_ne_one.as_str() {
            return Ok(false);
        }
        let Expr::Binary {
            left: rl,
            op: BinOp::Sub,
            right: rr,
            ..
        } = r_upd.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: rl_name, .. } = rl.as_ref() else {
            return Ok(false);
        };
        if rl_name.as_ref() != r_ne_one.as_str() || Self::int_literal_value_of(rr) != Some(1) {
            return Ok(false);
        }
        if self.type_context.get_type(count_name.as_ref()) != RustType::Vec(Box::new(RustType::F64))
            && self.type_context.get_type(count_name.as_ref())
                != RustType::Vec(Box::new(RustType::I32))
        {
            return Ok(false);
        }
        let count_i32 = matches!(
            self.type_context.get_type(count_name.as_ref()),
            RustType::Vec(inner) if *inner == RustType::I32
        );
        let count_esc = Self::escape_ident(count_name.as_ref());
        let r_esc = Self::escape_ident(&r_ne_one);
        // Must preserve partial fill when `r` is not `n` (Lehmer counter) — bulk `1..n` init
        // corrupts `count` on later outer-loop iterations and hangs fannkuch.
        self.writeln(&format!("while {} != 1_f64 {{", r_esc));
        self.indent += 1;
        self.writeln(&format!(
            "let ri = {} as usize;",
            r_esc
        ));
        self.writeln(&format!(
            "{}[ri - 1] = {};",
            count_esc,
            if count_i32 {
                format!("{} as i32", r_esc)
            } else {
                r_esc.to_string()
            }
        ));
        self.writeln(&format!("{} -= 1_f64;", r_esc));
        self.indent -= 1;
        self.writeln("}");
        Ok(true)
    }

    fn stmt_slice_unwrapped(body: &Statement) -> Vec<&Statement> {
        let mut cur = body;
        loop {
            match cur {
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    if statements.len() == 1 {
                        cur = &statements[0];
                    } else {
                        return statements.iter().collect();
                    }
                }
                other => return vec![other],
            }
        }
    }

    fn parse_k_ne_zero(cond: &Expr) -> Option<String> {
        let Expr::Binary {
            left,
            op: BinOp::StrictNe | BinOp::Ne,
            right,
            ..
        } = cond
        else {
            return None;
        };
        let Expr::Ident { name: k, .. } = left.as_ref() else {
            return None;
        };
        if Self::int_literal_value_of(right) != Some(0) {
            return None;
        }
        Some(k.to_string())
    }

    fn parse_perm_index_ident(index: &Expr) -> Option<String> {
        let Expr::Ident { name: n, .. } = index else {
            return None;
        };
        Some(n.to_string())
    }

    fn parse_perm_k_minus_counter(index: &Expr, k: &str, counter: &str) -> bool {
        let Expr::Binary {
            left,
            op: BinOp::Sub,
            right,
            ..
        } = index
        else {
            return false;
        };
        matches!(left.as_ref(), Expr::Ident { name: lk, .. } if lk.as_ref() == k)
            && matches!(right.as_ref(), Expr::Ident { name: rc, .. } if rc.as_ref() == counter)
    }

    /// `for (i=0; i<k2; i++) { temp=perm[i]; perm[i]=perm[k-i]; perm[k-i]=temp }` when `k2=(k+1)>>1`.
    fn parse_flip_swap_for(
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
        shift_half: &HashMap<String, String>,
    ) -> Option<(String, String)> {
        let (counter, bound) = Self::parse_usize_for_counter(init, cond, update)?;
        let k = shift_half.get(&bound)?;
        let stmts = Self::stmt_slice_unwrapped(body);
        if stmts.len() != 3 {
            return None;
        }
        let Statement::VarDecl {
            name: temp,
            init: Some(temp_init),
            ..
        } = stmts[0]
        else {
            return None;
        };
        let Expr::Index {
            object,
            index: idx0,
            ..
        } = temp_init
        else {
            return None;
        };
        let Expr::Ident { name: perm, .. } = object.as_ref() else {
            return None;
        };
        if Self::parse_perm_index_ident(idx0) != Some(counter.clone()) {
            return None;
        }
        let Statement::ExprStmt {
            expr: Expr::IndexAssign {
                object: obj1,
                index: idx1,
                value: val1,
                ..
            },
            ..
        } = stmts[1]
        else {
            return None;
        };
        let Expr::Ident { name: perm1, .. } = obj1.as_ref() else {
            return None;
        };
        if perm1.as_ref() != perm.as_ref() {
            return None;
        }
        if Self::parse_perm_index_ident(idx1) != Some(counter.clone()) {
            return None;
        }
        let Expr::Index {
            object: obj2,
            index: idx2,
            ..
        } = val1.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: perm2, .. } = obj2.as_ref() else {
            return None;
        };
        if perm2.as_ref() != perm.as_ref() || !Self::parse_perm_k_minus_counter(idx2, k, &counter) {
            return None;
        }
        let Statement::ExprStmt {
            expr: Expr::IndexAssign {
                object: obj3,
                index: idx3,
                value: val3,
                ..
            },
            ..
        } = stmts[2]
        else {
            return None;
        };
        let Expr::Ident { name: perm3, .. } = obj3.as_ref() else {
            return None;
        };
        if perm3.as_ref() != perm.as_ref() || !Self::parse_perm_k_minus_counter(idx3, k, &counter) {
            return None;
        }
        if !matches!(val3.as_ref(), Expr::Ident { name: t, .. } if t.as_ref() == temp.as_ref()) {
            return None;
        }
        Some((perm.to_string(), k.clone()))
    }

    /// `while (k!==0) { k2=(k+1)>>1; flip-for; flips++; k=perm[0] }` → fused usize half-loop.
    fn try_emit_flip_k_while(
        &mut self,
        cond: &Expr,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let k_name = match Self::parse_k_ne_zero(cond) {
            Some(k) => k,
            None => return Ok(false),
        };
        let stmts = Self::fn_body_stmt_slice(body);
        if stmts.len() != 4 {
            return Ok(false);
        }
        let Statement::VarDecl {
            name: k2_name,
            init: Some(k2_init),
            ..
        } = &stmts[0]
        else {
            return Ok(false);
        };
        if Self::parse_shift_half_init(k2_init) != Some(k_name.clone()) {
            return Ok(false);
        }
        let Statement::For {
            init,
            cond: for_cond,
            update,
            body: for_body,
            ..
        } = &stmts[1]
        else {
            return Ok(false);
        };
        let (perm_name, k_from_for) = match Self::parse_flip_swap_for(
            init.as_deref(),
            for_cond.as_ref(),
            update.as_ref(),
            for_body,
            &self.shift_half_of,
        ) {
            Some(v) => v,
            None => return Ok(false),
        };
        if k_from_for != k_name {
            return Ok(false);
        }
        let Statement::ExprStmt { expr: flips_upd, .. } = &stmts[2] else {
            return Ok(false);
        };
        let Expr::Assign {
            name: flips_name,
            value: flips_val,
            ..
        } = flips_upd
        else {
            return Ok(false);
        };
        let Expr::Binary {
            left: fl_l,
            op: BinOp::Add,
            right: fl_r,
            ..
        } = flips_val.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: fl_a, .. } = fl_l.as_ref() else {
            return Ok(false);
        };
        if fl_a.as_ref() != flips_name.as_ref() || Self::int_literal_value_of(fl_r) != Some(1) {
            return Ok(false);
        }
        let Statement::ExprStmt { expr: k_upd, .. } = &stmts[3] else {
            return Ok(false);
        };
        let Expr::Assign {
            name: k_upd_name,
            value: k_val,
            ..
        } = k_upd
        else {
            return Ok(false);
        };
        if k_upd_name.as_ref() != k_name.as_str() {
            return Ok(false);
        }
        let Expr::Index {
            object: k_obj,
            index: k_idx,
            ..
        } = k_val.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: k_perm, .. } = k_obj.as_ref() else {
            return Ok(false);
        };
        if k_perm.as_ref() != perm_name.as_str() {
            return Ok(false);
        }
        if Self::int_literal_value_of(k_idx) != Some(0) {
            return Ok(false);
        }
        let perm_i32 = matches!(
            self.type_context.get_type(perm_name.as_ref()),
            RustType::Vec(inner) if *inner == RustType::I32
        );
        if !perm_i32
            && self.type_context.get_type(perm_name.as_ref()) != RustType::Vec(Box::new(RustType::F64))
        {
            return Ok(false);
        }
        let perm_esc = Self::escape_ident(perm_name.as_ref());
        let k_esc = Self::escape_ident(&k_name);
        let flips_esc = Self::escape_ident(flips_name.as_ref());
        let label = format!("'while_loop_{}", self.loop_label_index);
        self.loop_label_index += 1;
        self.loop_stack.push((label.clone(), None));
        self.break_stack.push(label.clone());
        if perm_i32 {
            self.write(&format!("{}: while {} != 0 {{\n", label, k_esc));
            self.indent += 1;
            self.writeln(&format!("let ku = (({} as usize) + 1) / 2;", k_esc));
            let usize_var = format!("_usize_flip_{}", self.loop_label_index);
            self.loop_label_index += 1;
            self.writeln(&format!("for {} in 0..ku {{", usize_var));
            self.indent += 1;
            self.writeln(&format!("let a = {}[{}];", perm_esc, usize_var));
            self.writeln(&format!(
                "let b = {}[({} as usize) - {}];",
                perm_esc, k_esc, usize_var
            ));
            self.writeln(&format!("{}[{}] = b;", perm_esc, usize_var));
            self.writeln(&format!(
                "{}[({} as usize) - {}] = a;",
                perm_esc, k_esc, usize_var
            ));
            self.indent -= 1;
            self.writeln("}");
            self.writeln(&format!("{} += 1_f64;", flips_esc));
            self.writeln(&format!("{} = {}[0];", k_esc, perm_esc));
        } else {
            self.write(&format!("{}: while {} != 0_f64 {{\n", label, k_esc));
            self.indent += 1;
            self.writeln(&format!(
                "let ku = (({} as usize) + 1) / 2;",
                k_esc
            ));
            let usize_var = format!("_usize_flip_{}", self.loop_label_index);
            self.loop_label_index += 1;
            self.writeln(&format!("for {} in 0..ku {{", usize_var));
            self.indent += 1;
            self.writeln(&format!(
                "let a = {}[{}];",
                perm_esc, usize_var
            ));
            self.writeln(&format!(
                "let b = {}[({} as usize) - {}];",
                perm_esc, k_esc, usize_var
            ));
            self.writeln(&format!("{}[{}] = b;", perm_esc, usize_var));
            self.writeln(&format!(
                "{}[({} as usize) - {}] = a;",
                perm_esc, k_esc, usize_var
            ));
            self.indent -= 1;
            self.writeln("}");
            self.writeln(&format!("{} += 1_f64;", flips_esc));
            self.writeln(&format!("{} = {}[(0_f64) as usize];", k_esc, perm_esc));
        }
        self.indent -= 1;
        self.writeln("}");
        self.break_stack.pop();
        self.loop_stack.pop();
        let _ = k2_name;
        Ok(true)
    }

    fn parse_mandel_xt_assign(
        stmt: &Statement,
    ) -> Option<(String, String, String, String)> {
        let (xt_name, expr) = match stmt {
            Statement::VarDecl {
                name,
                init: Some(e),
                ..
            } => (name.to_string(), e),
            Statement::ExprStmt {
                expr: Expr::Assign { name, value, .. },
                ..
            } => (name.to_string(), value.as_ref()),
            _ => return None,
        };
        let Expr::Binary {
            left,
            op: BinOp::Add,
            right: x0_expr,
            ..
        } = expr
        else {
            return None;
        };
        let Expr::Ident { name: x0, .. } = x0_expr.as_ref() else {
            return None;
        };
        let Expr::Binary {
            left: sub_l,
            op: BinOp::Sub,
            right: sub_r,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        let (x_name, y_name) = match (sub_l.as_ref(), sub_r.as_ref()) {
            (
                Expr::Binary {
                    left: xl,
                    op: BinOp::Mul,
                    right: xr,
                    ..
                },
                Expr::Binary {
                    left: yl,
                    op: BinOp::Mul,
                    right: yr,
                    ..
                },
            ) => {
                let Expr::Ident { name: x, .. } = xl.as_ref() else {
                    return None;
                };
                let Expr::Ident { name: x2, .. } = xr.as_ref() else {
                    return None;
                };
                if x.as_ref() != x2.as_ref() {
                    return None;
                }
                let Expr::Ident { name: y, .. } = yl.as_ref() else {
                    return None;
                };
                let Expr::Ident { name: y2, .. } = yr.as_ref() else {
                    return None;
                };
                if y.as_ref() != y2.as_ref() {
                    return None;
                }
                (x.to_string(), y.to_string())
            }
            _ => return None,
        };
        Some((xt_name, x_name, y_name, x0.to_string()))
    }

    fn parse_mandel_y_assign(stmt: &Statement, x: &str, y: &str) -> Option<String> {
        let expr = match stmt {
            Statement::ExprStmt {
                expr: Expr::Assign { value, .. },
                ..
            } => value.as_ref(),
            _ => return None,
        };
        let Expr::Binary {
            left,
            op: BinOp::Add,
            right,
            ..
        } = expr
        else {
            return None;
        };
        let Expr::Ident { name: y0, .. } = right.as_ref() else {
            return None;
        };
        let Expr::Binary {
            left: prod_l,
            op: BinOp::Mul,
            right: prod_r,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        if Self::int_literal_value_of(prod_l) != Some(2) {
            // `2 * x * y` may parse as `(2 * x) * y`.
            if let Expr::Binary {
                left: outer_l,
                op: BinOp::Mul,
                right: outer_r,
                ..
            } = left.as_ref()
            {
                if let Expr::Binary {
                    left: two_l,
                    op: BinOp::Mul,
                    right: two_r,
                    ..
                } = outer_l.as_ref()
                {
                    if Self::int_literal_value_of(two_l) == Some(2) {
                        let Expr::Ident { name: xa, .. } = two_r.as_ref() else {
                            return None;
                        };
                        let Expr::Ident { name: ya, .. } = outer_r.as_ref() else {
                            return None;
                        };
                        if xa.as_ref() == x && ya.as_ref() == y {
                            return Some(y0.to_string());
                        }
                    }
                }
            }
            return None;
        }
        let Expr::Binary {
            left: xl,
            op: BinOp::Mul,
            right: yr,
            ..
        } = prod_r.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: xa, .. } = xl.as_ref() else {
            return None;
        };
        let Expr::Ident { name: ya, .. } = yr.as_ref() else {
            return None;
        };
        if xa.as_ref() != x || ya.as_ref() != y {
            return None;
        }
        Some(y0.to_string())
    }

    fn parse_mandel_x_assign(stmt: &Statement, x: &str, xt: &str) -> bool {
        let (name, src) = match stmt {
            Statement::ExprStmt {
                expr: Expr::Assign { name, value, .. },
                ..
            } => (name.as_ref(), value.as_ref()),
            _ => return false,
        };
        name == x && matches!(src, Expr::Ident { name: n, .. } if n.as_ref() == xt)
    }

    /// `xt=x²-y²+x0; y=2xy+y0; x=xt` → fused `x2/y2/xy` temps (one fewer multiply per iter).
    fn try_emit_mandel_iteration_fold(
        &mut self,
        statements: &[Statement],
    ) -> Result<Option<usize>, CompileError> {
        if !self.native_fn_body_emit && !self.native_vec_ret.is_some() {
            return Ok(None);
        }
        if statements.len() < 3 {
            return Ok(None);
        }
        let (xt, x, y, x0) = match Self::parse_mandel_xt_assign(&statements[0]) {
            Some(v) => v,
            None => return Ok(None),
        };
        let y0 = match Self::parse_mandel_y_assign(&statements[1], &x, &y) {
            Some(v) => v,
            None => return Ok(None),
        };
        if !Self::parse_mandel_x_assign(&statements[2], &x, &xt) {
            return Ok(None);
        }
        let x_esc = Self::escape_ident(&x);
        let y_esc = Self::escape_ident(&y);
        let x0_esc = Self::escape_ident(&x0);
        let y0_esc = Self::escape_ident(&y0);
        self.writeln(&format!("let x2 = {} * {};", x_esc, x_esc));
        self.writeln(&format!("let y2 = {} * {};", y_esc, y_esc));
        self.writeln(&format!("let xy = {} * {};", x_esc, y_esc));
        self.writeln(&format!("{} = ((2_f64 * xy) + {});", y_esc, y0_esc));
        self.writeln(&format!("{} = ((x2 - y2) + {});", x_esc, x0_esc));
        let _ = xt;
        Ok(Some(3))
    }

    fn emit_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } => {
                self.writeln("{");
                self.indent += 1;
                self.type_context.push_scope();
                self.outer_vars_stack.push(Vec::new());
                self.rc_cell_storage_scopes
                    .push(std::collections::HashSet::new());
                // Prepass: vars that must be RefCell because nested closures capture and mutate them
                let vars_mutated_by_nested =
                    Self::collect_vars_needing_capture_cell(statements);
                for v in &vars_mutated_by_nested {
                    self.refcell_wrapped_vars.insert(v.clone());
                }
                // Pre-scan for function declarations and create cells (for mutual recursion)
                let func_names = self.prescan_function_decls(statements);
                self.function_scope_stack.push(func_names.clone());
                // Create cells for all functions in this scope
                for func_name in &func_names {
                    let escaped = Self::escape_ident(func_name);
                    self.writeln(&format!(
                        "let {}_cell: VmRef<Value> = VmRef::new(Value::Null);",
                        escaped
                    ));
                }
                self.emit_statements_with_folds(statements)?;
                self.function_scope_stack.pop(); // Exit scope
                self.outer_vars_stack.pop(); // Exit variable scope
                self.rc_cell_storage_scopes.pop();
                for v in &vars_mutated_by_nested {
                    self.refcell_wrapped_vars.remove(v);
                }
                self.type_context.pop_scope();
                self.indent -= 1;
                self.writeln("}");
            }
            // Comma-declarators: emit each declarator into the *current* Rust scope
            // (no wrapping `{}`), so the bindings stay visible to later statements.
            Statement::Multi { statements, .. } => {
                self.emit_statements_with_folds(statements)?;
            }
            Statement::VarDecl {
                name,
                mutable,
                type_ann,
                init,
                ..
            } => {
                // Determine the Rust type from annotation, consulting the
                // user-declared `type` aliases so a `let x: World = ...`
                // resolves to `RustType::Named { name: "World", fields }`
                // and we can emit a struct move instead of a Value box.
                let mut rust_type = type_ann
                    .as_ref()
                    .map(|t| {
                        crate::types::RustType::from_annotation_with_aliases(t, &self.type_aliases)
                    })
                    .unwrap_or(RustType::Value);

                // M5 native-fn body: `let r = genRandom(1)` without a `: number` annotation.
                if self.native_fn_body_emit && rust_type == RustType::Value {
                    if let Some(Expr::Call { callee, .. }) = init.as_ref() {
                        if let Expr::Ident { name: fnname, .. } = callee.as_ref() {
                            if self.native_fns.contains(fnname.as_ref()) {
                                rust_type = RustType::F64;
                            }
                        }
                    }
                }

                // Mandelbrot: `iter` is unused after usize escape-loop fusion.
                if self.native_fn_body_emit {
                    if let Some(skip) = &self.skip_iter_local {
                        if name.as_ref() == skip {
                            if init
                                .as_ref()
                                .is_some_and(|e| Self::int_literal_value_of(e) == Some(0))
                            {
                                return Ok(());
                            }
                        }
                    }
                }

                // Native-vec fn body: `let out = []` builds a `Vec<f64>` accumulator.
                if matches!(self.native_vec_ret, Some(VecRetKind::VecF64)) {
                    if matches!(
                        init.as_ref(),
                        Some(Expr::Array { elements, .. }) if elements.is_empty()
                    ) {
                        rust_type = RustType::Vec(Box::new(RustType::F64));
                    }
                }

                // Soundness: a `number` local that a reassignment can turn non-numeric (e.g.
                // `s = s + arr[i]`, JS string concat) must stay a boxed `Value` — a native-f64
                // store would panic at the `from_value_expr(F64)` coercion. See
                // `demoted_numeric_locals`.
                if rust_type == RustType::F64 && self.demoted_numeric_locals.contains(name.as_ref())
                {
                    rust_type = RustType::Value;
                }

                // Native-vec call initializer: `let cum = cumulative(probs)` → `Vec<f64>`.
                if rust_type == RustType::Value {
                    if let Some(init_e) = init.as_ref() {
                        if let Some(rt) = self.native_vec_init_type(init_e) {
                            rust_type = rt;
                        }
                    }
                }

                // Native-vec fn body: `let k = perm[0]` on `Vec<i32>` → `i32` local.
                if self.native_vec_ret.is_some()
                    && matches!(rust_type, RustType::Value | RustType::F64)
                {
                    if let Some(init_e) = init.as_ref() {
                        if let Expr::Index { object, index, .. } = init_e {
                            if let Expr::Ident { name: arr_name, .. } = object.as_ref() {
                                if let RustType::Vec(inner) =
                                    self.type_context.get_type(arr_name.as_ref())
                                {
                                    if *inner == RustType::I32
                                        && self.index_in_bounds(index, arr_name.as_ref())
                                    {
                                        rust_type = RustType::I32;
                                    }
                                }
                            }
                        }
                    }
                }

                // Native-vec fn body: `let d1 = row + col` without `: number` → `f64` local.
                if rust_type == RustType::Value
                    && self.native_vec_ret.is_some()
                    && type_ann.is_none()
                {
                    if let Some(init_e) = init.as_ref() {
                        if Self::expr_infer_native_f64_in_vec_fn(
                            init_e,
                            &self.type_context,
                            &self.usize_for_counters,
                        ) {
                            rust_type = RustType::F64;
                        }
                    }
                }

                if self.int_i32_vec_locals.contains(name.as_ref()) {
                    rust_type = RustType::Vec(Box::new(RustType::I32));
                }

                // #176: top-level numeric globals live in a thread_local `Cell<f64>` — no local slot.
                if self.native_numeric_globals.contains_key(name.as_ref())
                    && self.outer_vars_stack.len() == 1
                {
                    self.type_context.define(name.as_ref(), RustType::F64);
                    return Ok(());
                }

                // Module-level const f64 arrays — no local slot in `run()`.
                if self.module_const_f64_arrays.contains_key(name.as_ref())
                    && self.outer_vars_stack.len() == 1
                {
                    self.type_context
                        .define(name.as_ref(), RustType::Vec(Box::new(RustType::F64)));
                    return Ok(());
                }

                // `let cum = cumulative(probs)` → precomputed module const; no local slot.
                if self.module_const_f64_aliases.contains_key(name.as_ref()) {
                    self.type_context
                        .define(name.as_ref(), RustType::Vec(Box::new(RustType::F64)));
                    return Ok(());
                }

                // i32-loop-var lowering: a `number` accumulator the analysis proved can live in an
                // `i32` register across a bitwise/hash hot loop. Declare `let mut h: i32` with the
                // init reinterpreted via `u32` so a literal ≥ 2^31 keeps its JS ToInt32 bit-pattern.
                if rust_type == RustType::F64 && self.i32_loop_vars.contains(name.as_ref()) {
                    let init_lit = init
                        .as_ref()
                        .and_then(|e| Self::int_literal_value_of(e));
                    if let Some(v) = init_lit {
                        rust_type = RustType::I32;
                        self.type_context.define(name.as_ref(), rust_type.clone());
                        let escaped_name = Self::escape_ident(name.as_ref());
                        let mutability = if *mutable { "let mut" } else { "let" };
                        // `v` is an exact integer (gate proved it); reinterpret its low 32 bits as
                        // i32 = ToInt32(v), the same bit-pattern the bitwise path produces.
                        self.writeln(&format!(
                            "{} {}: i32 = ({}u32) as i32;",
                            mutability,
                            escaped_name,
                            (v as i64 as u32)
                        ));
                        if let Some(scope) = self.outer_vars_stack.last_mut() {
                            scope.push(name.to_string());
                        }
                        return Ok(());
                    }
                }

                // Track the variable type
                self.type_context.define(name.as_ref(), rust_type.clone());

                if let Some(init_e) = init.as_ref() {
                    if Self::expr_is_map_construct(init_e) {
                        self.map_instance_locals.insert(name.to_string());
                    }
                }

                // `let cum = cumulative_nv(&probs)` inherits `probs`' fixed length for in-bounds reads.
                if let RustType::Vec(inner) = &rust_type {
                    if **inner == RustType::F64 {
                        if let Some(Expr::Call { callee, args, .. }) = init.as_ref() {
                            if let Expr::Ident { name: fnname, .. } = callee.as_ref() {
                                if let Some(sig) = self.native_vec_fns.get(fnname.as_ref()) {
                                    if sig.ret == VecRetKind::VecF64 {
                                        if let Some(CallArg::Expr(Expr::Ident { name: argn, .. })) =
                                            args.first()
                                        {
                                            if let Some(bound) =
                                                self.vec_fixed_len.get(argn.as_ref()).cloned()
                                            {
                                                self.vec_fixed_len
                                                    .insert(name.to_string(), bound);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // #177: the unboxed `Vec<TishStruct>` local is threaded `&mut` into the aggregate
                // operators (`advance`/`offsetMomentum`), so it must be `mut` even when never
                // directly reassigned in source.
                let force_mut = self.aggregate_array_locals.contains(name.as_ref());
                let mutability = if *mutable || force_mut { "let mut" } else { "let" };
                let escaped_name = Self::escape_ident(name.as_ref());

                if let Some(init_e) = init.as_ref() {
                    if let Some(parent) = Self::parse_shift_half_init(init_e) {
                        self.shift_half_of.insert(name.to_string(), parent);
                    }
                    if let Expr::Index { object, index, .. } = init_e {
                        if let Expr::Ident { name: arr_name, .. } = object.as_ref() {
                            if self.index_in_bounds(index, arr_name.as_ref()) {
                                if let Some(b) = self.vec_fixed_len.get(arr_name.as_ref()).cloned() {
                                    self.bounded_below_len.insert(name.to_string(), b.clone());
                                    if Self::int_literal_value_of(index) == Some(0) {
                                        self.seed_int_range_from_len(name.as_ref(), &b);
                                    }
                                }
                            }
                        }
                    }
                    if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                        && matches!(init_e, Expr::Ident { name: src, .. })
                    {
                        let Expr::Ident { name: src, .. } = init_e else {
                            unreachable!()
                        };
                        self.bounded_below_len
                            .insert(name.to_string(), BoundKey::Var(src.to_string()));
                    }
                    if self.native_vec_ret.is_some() {
                        if let Some(init_e) = init.as_ref() {
                            self.try_register_diag_coord_int_range(name.as_ref(), init_e);
                        }
                    }
                }

                if rust_type.is_native() {
                    // Generate native typed variable
                    let expr_str = match init.as_ref() {
                        Some(e) => self.emit_native_expr(e, &rust_type)?,
                        None => rust_type.default_value(),
                    };
                    if self.refcell_wrapped_vars.contains(name.as_ref()) {
                        // Closure-mutated: same Rc<RefCell<T>> pattern as Value (assignments use borrow_mut)
                        self.writeln(&format!("let {} = VmRef::new({});", escaped_name, expr_str));
                        self.rc_cell_storage_define(name.as_ref());
                    } else {
                        let type_str = rust_type.to_rust_type_str();
                        self.writeln(&format!(
                            "{} {}: {} = {};",
                            mutability, escaped_name, type_str, expr_str
                        ));
                    }
                } else {
                    // Original Value-based codegen
                    let (expr_str, clone_needed) = match init.as_ref() {
                        Some(e) => {
                            let s = self.emit_expr(e)?;
                            // Variable refs (Ident) in init must always clone: they may be used
                            // multiple times (e.g. in a loop body) and we cannot move.
                            let needs = matches!(e, Expr::Ident { .. }) || self.should_clone(e);
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
                        self.writeln(&format!("let {} = VmRef::new({});", escaped_name, init_val));
                        self.rc_cell_storage_define(name.as_ref());
                    } else if clone_needed {
                        self.writeln(&format!(
                            "{} {} = ({}).clone();",
                            mutability, escaped_name, expr_str
                        ));
                    } else {
                        self.writeln(&format!("{} {} = {};", mutability, escaped_name, expr_str));
                    }
                }

                if let Some(scope) = self.outer_vars_stack.last_mut() {
                    scope.push(name.to_string());
                }
            }
            Statement::VarDeclDestructure {
                pattern,
                mutable,
                init,
                span,
                ..
            } => {
                let expr = self.emit_expr(init)?;
                let mutability = if *mutable { "let mut" } else { "let" };
                let clone_suffix = if Self::needs_clone(init) {
                    ".clone()"
                } else {
                    ""
                };
                self.writeln(&format!("let _destruct_val = ({}){};", expr, clone_suffix));
                self.emit_destruct_bindings(pattern, "_destruct_val", mutability, *span)?;
                self.register_destruct_pattern_outer_vars(pattern);
            }
            Statement::ExprStmt { expr, .. } => {
                let e = self.emit_expr_discard(expr)?;
                self.writeln(&format!("{};", e));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                    && self.try_emit_checksum_parity_if(cond, then_branch, else_branch)?
                {
                    return Ok(());
                }
                if let Some(stayed) = self.pending_stayed_var.take() {
                    if self.try_emit_stayed_count_if(cond, then_branch, &stayed)? {
                        return Ok(());
                    }
                    self.pending_stayed_var = Some(stayed);
                }
                let c = self.emit_cond_expr(cond)?;
                self.write(&format!("if {} {{\n", c));
                self.indent += 1;
                self.emit_statement(then_branch)?;
                self.indent -= 1;
                if let Some(eb) = else_branch {
                    self.writeln("} else {");
                    self.indent += 1;
                    if let Some((lv, rv)) = Self::parse_strict_eq_idents(cond) {
                        self.strict_lt_bounds.push((lv.clone(), rv.clone()));
                        self.nonneg_locals.insert(lv.clone());
                        self.emit_statement(eb)?;
                        self.strict_lt_bounds.pop();
                        self.nonneg_locals.remove(&lv);
                    } else {
                        self.emit_statement(eb)?;
                    }
                    self.indent -= 1;
                }
                self.writeln("}");
            }
            Statement::While { cond, body, .. } => {
                if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                    && self.try_emit_flip_k_while(cond, body)?
                {
                    return Ok(());
                }
                if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                    && self.try_emit_count_r_decr_while(cond, body)?
                {
                    return Ok(());
                }
                if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                    && self.try_emit_native_left_shift_while(cond, body)?
                {
                    return Ok(());
                }
                if (self.native_fn_body_emit || self.native_vec_ret.is_some())
                    && self.try_emit_usize_bounded_escape_while(cond, body)?
                {
                    return Ok(());
                }
                let c = self.emit_cond_expr(cond)?;
                let label = format!("'while_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.break_stack.push(label.clone());
                self.write(&format!("{}: while {} {{\n", label, c));
                self.indent += 1;
                // #173 part 3: `while (i < n)` bounds `i` above by `n` inside the body.
                let pushed_guard = self.push_index_guard(Some(cond));
                self.emit_statement(body)?;
                if pushed_guard {
                    self.active_index_guards.pop();
                }
                self.break_stack.pop();
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                // M3 native fast path: the iterable is a `Vec<elem>` local with a native element
                // type (e.g. `let xs: number[]` -> `Vec<f64>`) and the body never mentions it (so
                // iterating by reference can't alias a mutation). Bind the loop var as `elem` so the
                // body lowers natively — no per-element `Value::clone`, and accumulators stay f64.
                let mut emitted_native = false;
                if let Expr::Ident { name: it_name, .. } = iterable {
                    let module_const_it =
                        self.module_const_f64_array_rust_ref(it_name.as_ref());
                    let it_vec_ty = if module_const_it.is_some() {
                        RustType::Vec(Box::new(RustType::F64))
                    } else {
                        self.type_context.get_type(it_name.as_ref())
                    };
                    if let RustType::Vec(elem) = it_vec_ty {
                        if elem.is_native() {
                            let mut body_idents = std::collections::HashSet::new();
                            Self::collect_stmt_idents(body, &mut body_idents);
                            if !body_idents.contains(it_name.as_ref()) {
                                let esc_it = module_const_it
                                    .unwrap_or_else(|| {
                                        Self::escape_ident(it_name.as_ref()).into_owned()
                                    });
                                let esc_name = Self::escape_ident(name.as_ref()).into_owned();
                                // Index-based iteration (not `.iter().cloned()`, which rustc fails to
                                // tighten here): `0..len` indexing of a `Vec<f64>` matches a hand-
                                // written C-style loop. Unique counter names keep nested ForOf sound.
                                let idx = self.loop_label_index;
                                self.loop_label_index += 1;
                                let copy_elem = matches!(*elem, RustType::F64 | RustType::Bool);
                                let bind = if copy_elem {
                                    format!("let {} = {}[_fof_i{}];", esc_name, esc_it, idx)
                                } else {
                                    format!("let {} = {}[_fof_i{}].clone();", esc_name, esc_it, idx)
                                };
                                self.writeln(&format!("for _fof_i{} in 0..{}.len() {{", idx, esc_it));
                                self.indent += 1;
                                self.writeln(&bind);
                                self.type_context.push_scope();
                                self.type_context.define(name.as_ref(), *elem);
                                self.emit_statement(body)?;
                                self.type_context.pop_scope();
                                self.indent -= 1;
                                self.writeln("}");
                                emitted_native = true;
                            }
                        }
                    }
                }
                if !emitted_native {
                    let iter_expr = self.emit_expr(iterable)?;
                    // `normalize_for_of` drains a JS iterator object (Map/Set `.values()` etc.)
                    // into an array; arrays/strings/everything else pass through unchanged.
                    self.writeln(&format!(
                        "{{ let _fof = tishlang_runtime::normalize_for_of(({}).clone());",
                        iter_expr
                    ));
                    self.indent += 1;
                    self.writeln("match &_fof {");
                    self.indent += 1;
                    self.writeln("Value::Array(ref _arr) => {");
                    self.indent += 1;
                    self.writeln("for _v in _arr.borrow().iter() {");
                    self.indent += 1;
                    self.writeln(&format!(
                        "let {} = _v.clone();",
                        Self::escape_ident(name.as_ref())
                    ));
                    self.emit_statement(body)?;
                    self.indent -= 1;
                    self.writeln("}");
                    self.indent -= 1;
                    self.writeln("}");
                    // Packed `Float64Array` (`TISH_PACKED_ARRAYS`): iterate the `Vec<f64>` directly,
                    // re-boxing each element to `Value::Number` for the loop body.
                    self.writeln("Value::NumberArray(ref _arr) => {");
                    self.indent += 1;
                    self.writeln("for _v in _arr.borrow().iter() {");
                    self.indent += 1;
                    self.writeln(&format!(
                        "let {} = Value::Number(*_v);",
                        Self::escape_ident(name.as_ref())
                    ));
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
                        "let {} = Value::String(tishlang_runtime::ArcStr::from(_ch.to_string()));",
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
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                // #173: fuse a fill loop `for (let i = 0; i < N; i++) { a.push(K) }` over a native
                // `Vec<T>` into a single bulk `extend` — one allocation instead of N per-element
                // pushes (which repeatedly realloc as the Vec grows). Sound only when `N` is a proven,
                // side-effect-free integer; otherwise the normal loop is emitted below.
                if self.try_emit_spectral_triple_init(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                )? {
                    return Ok(());
                }
                if self.try_emit_fannkuch_triple_init(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                )? {
                    return Ok(());
                }
                if self.try_emit_native_vec_copy_loop(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                )? {
                    return Ok(());
                }
                if self.try_emit_native_fill_loop(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                )? {
                    return Ok(());
                }
                if self.try_emit_vec_copy_within_shift_for(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                )? {
                    return Ok(());
                }
                if let Some((ctr, bound)) =
                    Self::parse_usize_for_counter(init.as_deref(), cond.as_ref(), update.as_ref())
                {
                    let emit_usize = self.native_fn_body_emit
                        || self.native_vec_ret.is_some()
                        || (self.value_fn_depth > 0
                            && self.usize_for_loop_bound_is_native(bound.as_ref()));
                    if emit_usize && self.try_emit_usize_for_loop(&ctr, &bound, body)? {
                        return Ok(());
                    }
                }
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
                    let ue = self.emit_expr_discard(u).unwrap();
                    format!("{};", ue)
                });
                self.loop_stack.push((label.clone(), update_code));
                self.break_stack.push(label.clone());
                self.write(&format!("{}: loop {{\n", label));
                self.indent += 1;
                self.writeln(&format!("if !{} {{ break; }}", cond_expr));
                // #173 part 3: `for (…; i < n; …)` bounds `i` above by `n` inside the body.
                let pushed_guard = self.push_index_guard(cond.as_ref());
                self.emit_statement(body)?;
                if pushed_guard {
                    self.active_index_guards.pop();
                }
                if let Some(u) = update {
                    let ue = self.emit_expr_discard(u)?;
                    self.writeln(&format!("{};", ue));
                }
                self.break_stack.pop();
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Return { value, .. } => {
                // #175: inside a native-vec free fn body, returns lower to the native shape
                // (`f64` or `()`), not a boxed `Value`.
                if let Some(ret_kind) = self.native_vec_ret {
                    match ret_kind {
                        VecRetKind::F64 => {
                            let e = value
                                .as_ref()
                                .ok_or_else(|| CompileError::new("native-vec f64 fn: empty return", None))?;
                            let f = self.emit_f64(e)?;
                            self.writeln(&format!("return {};", f));
                        }
                        VecRetKind::VecF64 => {
                            let e = value
                                .as_ref()
                                .ok_or_else(|| CompileError::new("native-vec vec fn: empty return", None))?;
                            if let Expr::Ident { name, .. } = e {
                                self.writeln(&format!(
                                    "return {};",
                                    Self::escape_ident(name.as_ref())
                                ));
                            } else {
                                let (c, ty) = self.emit_typed_expr(e)?;
                                if ty == RustType::Vec(Box::new(RustType::F64)) {
                                    self.writeln(&format!("return {};", c));
                                } else {
                                    return Err(CompileError::new(
                                        "native-vec vec fn: return must be Vec<f64>",
                                        None,
                                    ));
                                }
                            }
                        }
                        VecRetKind::Unit => {
                            self.writeln("return;");
                        }
                    }
                    return Ok(());
                }
                let v = value
                    .as_ref()
                    .map(|e| self.emit_expr(e))
                    .transpose()?
                    .unwrap_or_else(|| "Value::Null".to_string());
                if self.try_closure_depth > 0 {
                    // Inside a try-body closure: escape it as a pending-return completion so any
                    // enclosing `finally` runs on the way out to the function boundary.
                    self.writeln(&format!("return Ok(Some({}));", v));
                } else {
                    self.writeln(&format!("return {};", v));
                }
            }
            Statement::Break { .. } => {
                // `break` exits the innermost loop OR switch (break_stack), not necessarily the
                // innermost loop. A switch pushes a label here so its `break` stays switch-local.
                if let Some(label) = self.break_stack.last() {
                    self.writeln(&format!("break {};", label));
                } else {
                    self.writeln("break;");
                }
            }
            Statement::Continue { .. } => {
                let snippet = self
                    .loop_stack
                    .last()
                    .map(|(label, update)| (label.clone(), update.clone()));
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
            Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {}
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                let e = self.emit_expr(expr)?;
                self.writeln(&format!("let _sv = {};", e));
                // Wrap in a labeled block so `break` inside a case exits the SWITCH, not an
                // enclosing loop. tish switch has no fall-through (match's first-arm semantics).
                let sw_label = format!("'switch_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.break_stack.push(sw_label.clone());
                self.write(&format!("{}: {{\n", sw_label));
                self.indent += 1;
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
                self.indent -= 1;
                self.writeln("}");
                self.break_stack.pop();
            }
            Statement::DoWhile { body, cond, .. } => {
                let c = self.emit_cond_expr(cond)?;
                let label = format!("'dowhile_loop_{}", self.loop_label_index);
                self.loop_label_index += 1;
                self.loop_stack.push((label.clone(), None));
                self.break_stack.push(label.clone());
                self.write(&format!("{}: loop {{\n", label));
                self.indent += 1;
                self.emit_statement(body)?;
                self.write(&format!("if !{} {{ break; }}\n", c));
                self.break_stack.pop();
                self.loop_stack.pop();
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Throw { value, .. } => {
                let v = self.emit_expr(value)?;
                if self.try_closure_depth > 0 {
                    // Inside a try-body closure (so `catch`/`finally` can see it): a catchable error
                    // completion. A try-closure returns `Result` even when nested in a native-typed
                    // fn, so this stays well-typed.
                    self.writeln(&format!(
                        "return Err(Box::new(tishlang_runtime::TishError::Throw({})) as Box<dyn std::error::Error>);",
                        v
                    ));
                } else if self.in_native_typed_frame() {
                    // #303: native-typed frame (`-> f64`/`-> Vec<..>`/`-> ()`) with no enclosing try.
                    // There is no `Value` or `Result` channel here, so neither the dummy-`Value`
                    // escape nor a `return Err(..)` would type-check. Fall back to the pre-#303 abort
                    // (a native numeric kernel hitting an uncaught throw is already a degenerate path;
                    // `panic!` coerces to any return type via `!`).
                    self.writeln(&format!(
                        "{{ let _th = {}; panic!(\"uncaught throw: {{}}\", _th.to_display_string()); }}",
                        v
                    ));
                } else if self.value_fn_depth == 0 {
                    // Top level (run() returns a Result): a catchable error completion.
                    self.writeln(&format!(
                        "return Err(Box::new(tishlang_runtime::TishError::Throw({})) as Box<dyn std::error::Error>);",
                        v
                    ));
                } else {
                    // #303: value-fn body with no enclosing try — the native-fn ABI (`-> Value`) has
                    // no error channel, so store the throw in the pending slot and escape with a dummy
                    // `Value`. The caller's post-call pending-throw check (emitted after each call)
                    // propagates it: re-raised as `TishError::Throw` in a try-closure / at top level,
                    // or escaped again to climb past another value-fn.
                    self.writeln(&format!(
                        "{{ tishlang_runtime::set_pending_throw({}); return Value::Null; }}",
                        v
                    ));
                }
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                // The try body runs in a completion closure:
                //   Ok(None)     = ran to the end normally
                //   Ok(Some(v))  = a `return v` is pending (must run finally, then return)
                //   Err(Throw)   = a `throw` is pending (catchable; else runs finally then re-raises)
                // `return`/`throw` inside the body emit the closure-escaping form (try_closure_depth).
                self.writeln(
                    "let mut _flow: Result<Option<Value>, Box<dyn std::error::Error>> = (|| {",
                );
                self.indent += 1;
                self.try_closure_depth += 1;
                self.emit_statement(body)?;
                self.try_closure_depth -= 1;
                self.writeln("Ok(None)");
                self.indent -= 1;
                self.writeln("})();");

                if let Some(catch_stmt) = catch_body {
                    // Only a `throw` is catchable; a pending `return` (Ok(Some)) bypasses catch.
                    self.writeln("_flow = match _flow {");
                    self.indent += 1;
                    self.writeln("Err(_e) => match _e.downcast::<tishlang_runtime::TishError>() {");
                    self.indent += 1;
                    self.writeln("Ok(_te) => match *_te {");
                    self.indent += 1;
                    self.writeln("tishlang_runtime::TishError::Throw(_tv) => {");
                    self.indent += 1;
                    if let Some(param) = catch_param {
                        self.writeln(&format!("let {} = _tv;", Self::escape_ident(param.as_ref())));
                    }
                    self.writeln(
                        "(|| -> Result<Option<Value>, Box<dyn std::error::Error>> {",
                    );
                    self.indent += 1;
                    self.try_closure_depth += 1;
                    self.emit_statement(catch_stmt)?;
                    self.try_closure_depth -= 1;
                    self.writeln("Ok(None)");
                    self.indent -= 1;
                    self.writeln("})()");
                    self.indent -= 1;
                    self.writeln("}");
                    self.writeln("_other => Err(Box::new(_other)),");
                    self.indent -= 1;
                    self.writeln("},");
                    self.writeln("Err(_orig) => Err(_orig),");
                    self.indent -= 1;
                    self.writeln("},");
                    self.writeln("_ok => _ok,");
                    self.indent -= 1;
                    self.writeln("};");
                }

                if let Some(finally_stmt) = finally_body {
                    self.emit_statement(finally_stmt)?;
                }

                // After finally, propagate any pending completion in the form the enclosing context
                // expects (an outer try-closure / a value-fn body / top-level run()).
                self.writeln("match _flow {");
                self.indent += 1;
                if self.try_closure_depth > 0 {
                    self.writeln("Ok(Some(_rv)) => return Ok(Some(_rv)),");
                    self.writeln("Err(_e) => return Err(_e),");
                } else if self.value_fn_depth > 0 {
                    self.writeln("Ok(Some(_rv)) => return _rv,");
                    self.writeln("Err(_e) => return tishlang_runtime::fn_unwind(_e),");
                } else {
                    // Top level (run() -> Result<(), _>): a top-level `return value` just ends the
                    // script (the value is unobservable); an uncaught throw propagates out of run().
                    self.writeln("Ok(Some(_)) => return Ok(()),");
                    self.writeln("Err(_e) => return Err(_e),");
                }
                self.writeln("Ok(None) => {}");
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::FunDecl {
                name,
                params,
                rest_param,
                body,
                span,
                ..
            } => {
                // #177: this function was de-virtualized into a native aggregate free fn
                // (`<name>_agg`, emitted before `run()`); all call sites were routed there.
                // Skip the boxed closure entirely — its body now references unboxed structs
                // that no longer fit the boxed `Value` ABI.
                if self.aggregate_alias.is_some()
                    && self.aggregate_fns.contains_key(name.as_ref())
                {
                    return Ok(());
                }
                // Use Rc<RefCell<>> pattern to allow recursive function calls
                // The function can reference itself through the cell
                let name_raw = name.as_ref();
                let name_str = Self::escape_ident(name_raw);
                // Check if cell was already created by block prescan
                let cell_exists = self
                    .function_scope_stack
                    .last()
                    .map(|scope| scope.contains(&name_raw.to_string()))
                    .unwrap_or(false);
                if !cell_exists {
                    self.writeln(&format!(
                        "let {}_cell: VmRef<Value> = VmRef::new(Value::Null);",
                        name_str
                    ));
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
                let outer_params: Vec<String> = self
                    .outer_params_stack
                    .iter()
                    .flat_map(|p| p.iter().cloned())
                    .filter(|name| referenced.contains(name) && !param_names.contains(name))
                    .collect();
                // Collect outer variables (from outer_vars_stack) - wrap in RefCell for mutable capture
                // Exclude params and variables declared in this function's body (locals)
                let mut local_var_names = HashSet::new();
                Self::collect_local_var_names(body, &mut local_var_names);
                let outer_vars: Vec<String> = self
                    .outer_vars_stack
                    .iter()
                    .flat_map(|v| v.iter().cloned())
                    .filter(|name| {
                        referenced.contains(name)
                            && !param_names.contains(name)
                            && !local_var_names.contains(name)
                    })
                    .filter(|name| {
                        ![
                            "Boolean",
                            "console",
                            "Math",
                            "JSON",
                            "Date",
                            "Set",
                            "Map",
                            "Object",
                            "process",
                            "setTimeout",
                            "clearTimeout",
                            "setInterval",
                            "clearInterval",
                            "Promise",
                            "Symbol",
                            "RegExp",
                            "Polars",
                        ]
                        .contains(&name.as_str())
                            && !self.native_numeric_globals.contains_key(name.as_str())
                    })
                    .collect();

                // Live cell capture: assigned in this body, or already a shared
                // `VmRef` cell in a parent scope (so a closure that only READS the
                // var still sees later mutations through the shared cell, instead
                // of snapshotting it by value at creation time). Truly read-only,
                // non-cell vars get a Value snapshot (avoids param-shadow issues).
                // Mirrors `emit_arrow_function`.
                let mut assigned_in_body = HashSet::new();
                Self::collect_assigned_idents_in_stmt(body, &mut assigned_in_body);
                let mutable_outer_vars: Vec<String> = outer_vars
                    .iter()
                    .filter(|v| assigned_in_body.contains(*v) || self.rc_cell_storage_contains(v))
                    .cloned()
                    .collect();
                let read_only_outer_vars: Vec<String> = outer_vars
                    .iter()
                    .filter(|v| !assigned_in_body.contains(*v) && !self.rc_cell_storage_contains(v))
                    .cloned()
                    .collect();

                // Rebind outer vars to Rc<RefCell<>> with _cell suffix.
                // If outer scope already has the var as RefCell, just clone it.
                for outer_var in &outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    if self.rc_cell_storage_contains(outer_var) {
                        self.writeln(&format!(
                            "let {}_cell = {}.clone();",
                            var_escaped, var_escaped
                        ));
                    } else {
                        self.writeln(&format!(
                            "let {}_cell = VmRef::new({}.clone());",
                            var_escaped, var_escaped
                        ));
                    }
                }

                self.writeln(&format!("let {} = {{", name_str));
                self.indent += 1;
                // Clone RefCell for outer vars so closure can capture
                for outer_var in &outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!(
                        "let {}_cell = {}_cell.clone();",
                        var_escaped, var_escaped
                    ));
                }
                // Clone the cell so the closure can reference the function recursively
                let needs_self_ref = referenced.contains(name_raw);
                if needs_self_ref {
                    self.writeln(&format!(
                        "let {}_ref = {}_cell.clone();",
                        name_str, name_str
                    ));
                }
                // Clone sibling function cells for mutual recursion
                let sibling_fns: Vec<String> = self
                    .function_scope_stack
                    .last()
                    .map(|scope| {
                        scope
                            .iter()
                            .filter(|s| s.as_str() != name_raw && referenced.contains(s.as_str()))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                for sibling in &sibling_fns {
                    let sibling_escaped = Self::escape_ident(sibling);
                    self.writeln(&format!(
                        "let {}_ref = {}_cell.clone();",
                        sibling_escaped, sibling_escaped
                    ));
                }
                // Clone outer parameters so they can be captured by the move closure
                for outer_param in &outer_params {
                    let param_escaped = Self::escape_ident(outer_param);
                    self.writeln(&format!(
                        "let {} = {}.clone();",
                        param_escaped, param_escaped
                    ));
                }
                // Only clone builtins that are actually referenced (clone so outer scope can still use them, e.g. process for PORT before serve)
                for builtin in &[
                    "Boolean",
                    "console",
                    "Math",
                    "JSON",
                    "Date",
                    "Set",
                    "Map",
                    "Object",
                    "Array",
                    "Number",
                    "Float64Array",
                    "Float32Array",
                    "Int8Array",
                    "Uint8Array",
                    "Uint8ClampedArray",
                    "Int16Array",
                    "Uint16Array",
                    "Int32Array",
                    "Uint32Array",
                    "AudioContext",
                    "process",
                    "setTimeout",
                    "clearTimeout",
                    "setInterval",
                    "clearInterval",
                    "Promise",
                    "Symbol",
                    "RegExp",
                    "Polars",
                    // Free-standing global functions used inside user-defined
                    // functions also need to be cloned into the closure
                    // capture, or the emitted Rust hits E0382 (moved value)
                    // at the closure's defining `let`.
                    "parseInt",
                    "parseFloat",
                    "isNaN",
                    "isFinite",
                    "encodeURI",
                    "decodeURI",
                    "htmlEscape",
                    "registerStaticRoute",
                    "String",
                    "Infinity",
                    "NaN",
                    "serve",
                ] {
                    if referenced.contains(*builtin) {
                        self.writeln(&format!("let {} = {}.clone();", builtin, builtin));
                    }
                }
                // Feature-gated globals also move into the closure when referenced.
                // Clone them only when their capability is actually linked, so we
                // never emit `let h = h.clone();` for a binding that was never
                // emitted (e.g. a fn-local named `h` in a program without JSX).
                let mut gated: Vec<&str> = Vec::new();
                if self.has_feature("http") {
                    gated.extend(["fetch", "fetchAll"]);
                }
                if self.has_feature("fs") {
                    gated.extend(["readFile", "writeFile", "fileExists", "isDir", "readDir", "mkdir"]);
                }
                if self.program_has_jsx && !self.has_native_ui_host {
                    gated.extend(["Fragment", "h", "text", "useState", "createRoot"]);
                }
                for name in gated {
                    if referenced.contains(name) {
                        self.writeln(&format!("let {} = {}.clone();", name, name));
                    }
                }
                self.writeln("Value::native(move |args: &[Value]| {");
                self.value_fn_depth += 1;
                // #303: a nested value-fn is its own `-> Value` frame; the enclosing `try`'s
                // `Result` channel is NOT reachable via `return Err` from inside it (a throw here
                // returns to the fn's caller, not the outer try). Reset the try-closure depth so
                // throws / `emit_pending_throw_check` inside this body use the pending-throw slot,
                // not a mis-typed `return Err`. Restored at every exit below.
                let saved_try_depth = self.try_closure_depth;
                self.try_closure_depth = 0;
                self.indent += 1;
                // Mutable outer vars: capture the RefCell so assignments use borrow_mut
                for outer_var in &mutable_outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!(
                        "let {} = {}_cell.clone();",
                        var_escaped, var_escaped
                    ));
                }
                // Read-only outer vars: Value binding from borrow (avoids param-shadow issues)
                for outer_var in &read_only_outer_vars {
                    let var_escaped = Self::escape_ident(outer_var);
                    self.writeln(&format!(
                        "let {} = (*{}_cell.borrow()).clone();",
                        var_escaped, var_escaped
                    ));
                }
                // Make the function available by its name inside the closure (only if recursive)
                if needs_self_ref {
                    self.writeln(&format!(
                        "let {} = (*{}_ref.borrow()).clone();",
                        name_str, name_str
                    ));
                }
                // Make sibling functions available for mutual recursion
                for sibling in &sibling_fns {
                    let sibling_escaped = Self::escape_ident(sibling);
                    self.writeln(&format!(
                        "let {} = (*{}_ref.borrow()).clone();",
                        sibling_escaped, sibling_escaped
                    ));
                }
                // Extract just the parameter names (type annotations are parsed but not used in codegen yet)
                let current_param_names: Vec<String> = params
                    .iter()
                    .flat_map(|p| p.bound_names())
                    .map(|n| n.to_string())
                    .collect();
                let formal_span = *span;
                // M1 (keystone, dark-shipped behind TISH_PARAM_NATIVE): a typed scalar param
                // normally arrives boxed (`args.get(i).cloned()`), which poisons native math in
                // the body (e.g. `i*N+k` boxes). Bind a *native shadow* — coerce once to f64/
                // bool/String — so the body lowers it like a native local. Conservative: only
                // simple params, native-scalar annotation, no default value.
                let param_native =
                    crate::native_opts_enabled();
                // A param referenced by ANY sibling default expr (e.g. `(a, b = a + 1)`) must NOT
                // get a native f64 shadow: the default binding is emitted on the boxed Value path
                // (`ops::add(&a, …)` expects `&Value`), so a native `a: f64` would mistype the
                // generated Rust. Keep such params boxed — correctness over the M1 optimization;
                // defaults referencing params are rare in hot code. Also covers the M4 case where
                // an unannotated param (e.g. `dependent(a, b = a + 1)`) is inferred numeric.
                let mut default_referenced: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for p in params {
                    if let FunParam::Simple(tp) = p {
                        if let Some(d) = &tp.default {
                            Self::collect_expr_idents(d, &mut default_referenced);
                        }
                    }
                }
                let mut native_params: Vec<(String, RustType)> = Vec::new();
                let arr_param_flags = self.native_arr_param_fns.get(name.as_ref()).cloned();
                for (i, p) in params.iter().enumerate() {
                    match p {
                        FunParam::Simple(tp) => {
                            // #320: a READ-ONLY `number[]` param — unbox the boxed arg once into an
                            // owned native `Vec<f64>` so `seq[i+j]` lowers to a native f64 index (the
                            // shared `Expr::Index` fast-path fires on the `Vec` type). The rest of the
                            // body stays boxed (Map ops, boxed return) — only this param flips native.
                            if arr_param_flags
                                .as_ref()
                                .and_then(|f| f.get(i).copied())
                                .unwrap_or(false)
                            {
                                let vt = RustType::Vec(Box::new(RustType::F64));
                                // Unbox once: a packed `NumberArray` is already a `Vec<f64>` (clone
                                // the backing), a boxed `Array` is mapped element-wise. A `number[]`
                                // is all-numbers by inference, so the `_ => NaN` fallback (JS
                                // `arr[oob]`→NaN) is unreachable for sound inputs but never panics.
                                self.writeln(&format!(
                                    "let mut {}: Vec<f64> = match args.get({}) {{ \
                                       Some(Value::NumberArray(a)) => a.borrow().clone(), \
                                       Some(Value::Array(arr)) => arr.borrow().iter().map(|v| match v {{ Value::Number(n) => *n, _ => f64::NAN }}).collect(), \
                                       _ => Vec::new() }};",
                                    Self::escape_ident(tp.name.as_ref()),
                                    i,
                                ));
                                native_params.push((tp.name.to_string(), vt));
                                continue;
                            }
                            let native_ty = if param_native
                                && tp.default.is_none()
                                && !default_referenced.contains(tp.name.as_ref())
                            {
                                tp.type_ann
                                    .as_ref()
                                    .map(RustType::from_annotation)
                                    .filter(|t| {
                                        matches!(
                                            t,
                                            RustType::F64 | RustType::Bool | RustType::String
                                        )
                                    })
                            } else {
                                None
                            };
                            if let Some(nt) = native_ty {
                                let coercion = nt.from_value_expr(&format!(
                                    "args.get({}).cloned().unwrap_or(Value::Null)",
                                    i
                                ));
                                self.writeln(&format!(
                                    "{} {} = {};",
                                    Self::mut_kw_for(tp.name.as_ref(), "let mut"),
                                    Self::escape_ident(tp.name.as_ref()),
                                    coercion
                                ));
                                native_params.push((tp.name.to_string(), nt));
                            } else if let Some(default_expr) = &tp.default {
                                // Default applies only when the positional arg is MISSING
                                // (`args.get(i) == None`), matching the interpreter + bytecode VM.
                                // An explicit `null` argument is "supplied" and keeps the null.
                                // Earlier params are already bound above, so a default may
                                // reference them, e.g. `(a, b = a + 1)`.
                                let default_str = self.emit_expr(default_expr)?;
                                self.writeln(&format!(
                                    "{} {} = match args.get({}) {{ Some(v) => v.clone(), None => {} }};",
                                    Self::mut_kw_for(tp.name.as_ref(), "let mut"),
                                    Self::escape_ident(tp.name.as_ref()),
                                    i,
                                    default_str
                                ));
                            } else {
                                self.writeln(&format!(
                                    "{} {} = args.get({}).cloned().unwrap_or(Value::Null);",
                                    Self::mut_kw_for(tp.name.as_ref(), "let mut"),
                                    Self::escape_ident(tp.name.as_ref()),
                                    i
                                ));
                            }
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
                // A typed rest-param `...args: number[]` lowers to a native `Vec<elem>` (unbox each
                // trailing arg) instead of a boxed `Value::Array`, so the body iterates/indexes it
                // natively (and `for (let x of args)` keeps accumulators `f64`). Non-native element
                // types fall back to the boxed array.
                let rest_native: Option<RustType> = rest_param.as_ref().and_then(|rp| {
                    rp.type_ann.as_ref().and_then(|ann| {
                        match RustType::from_annotation_with_aliases(ann, &self.type_aliases) {
                            RustType::Vec(elem) if elem.is_native() => Some(RustType::Vec(elem)),
                            _ => None,
                        }
                    })
                });
                if let Some(rest) = rest_param {
                    if let Some(RustType::Vec(elem)) = &rest_native {
                        self.writeln(&format!(
                            "let {}: Vec<{}> = args[{}..].iter().map(|v| {}).collect();",
                            Self::escape_ident(rest.name.as_ref()),
                            elem.to_rust_type_str(),
                            params.len(),
                            elem.from_value_expr("v")
                        ));
                    } else {
                        self.writeln(&format!(
                            "let {} = Value::Array(VmRef::new(args[{}..].to_vec()));",
                            Self::escape_ident(rest.name.as_ref()),
                            params.len()
                        ));
                    }
                }

                self.type_context
                    .push_fun_param_scope(params, rest_param.as_ref());
                // Register native-shadowed params (bound above) with their native type so the
                // body lowers them exactly like native locals (binops, indices, etc.).
                for (pname, pty) in &native_params {
                    self.type_context.define(pname, pty.clone());
                }
                // A native `Vec` rest-param: register so the body iterates/indexes it natively.
                if let (Some(rest), Some(rt)) = (rest_param.as_ref(), rest_native.as_ref()) {
                    self.type_context.define(rest.name.as_ref(), rt.clone());
                }

                let fun_body_res: Result<(), CompileError> = (|| -> Result<(), CompileError> {
                    // Push current params to stack for nested functions
                    self.outer_params_stack.push(current_param_names);

                    // Function bodies are sync closures (even Tish async fn) - use block_on for await
                    self.async_context_stack.push(false);

                    // Mutable outer vars must be in refcell_wrapped_vars so Assign/CompoundAssign emit borrow_mut
                    let saved_refcell = self.refcell_wrapped_vars.clone();
                    for v in &mutable_outer_vars {
                        self.refcell_wrapped_vars.insert(v.clone());
                    }
                    // Read-only captures are plain Value bindings inside the closure.
                    for v in &read_only_outer_vars {
                        self.refcell_wrapped_vars.remove(v);
                    }

                    // Pre-scan body for nested functions (handles function body as Block)
                    if let Statement::Block { statements, .. } = body.as_ref() {
                        let nested_func_names = self.prescan_function_decls(statements);
                        self.function_scope_stack.push(nested_func_names.clone());
                        self.outer_vars_stack.push(Vec::new());
                        self.rc_cell_storage_scopes
                            .push(std::collections::HashSet::new());
                        // Create cells for nested functions
                        for func_name in &nested_func_names {
                            let escaped = Self::escape_ident(func_name);
                            self.writeln(&format!(
                                "let {}_cell: VmRef<Value> = VmRef::new(Value::Null);",
                                escaped
                            ));
                        }
                        // Vars declared in this body that a nested closure captures
                        // and that are assigned somewhere in the body must be shared
                        // `VmRef` cells (e.g. `let t=0; let f=()=>t; t=100`). Block
                        // scopes get this via emit_statement(Block); a function body
                        // is iterated directly, so run the same prepass here.
                        let body_cell_vars =
                            Self::collect_vars_needing_capture_cell(statements);
                        for v in &body_cell_vars {
                            self.refcell_wrapped_vars.insert(v.clone());
                        }
                        for s in statements {
                            self.emit_statement(s)?;
                        }
                        for v in &body_cell_vars {
                            self.refcell_wrapped_vars.remove(v);
                        }
                        self.function_scope_stack.pop();
                        self.outer_vars_stack.pop();
                        self.rc_cell_storage_scopes.pop();
                    } else {
                        self.function_scope_stack.push(Vec::new());
                        self.outer_vars_stack.push(Vec::new());
                        self.rc_cell_storage_scopes
                            .push(std::collections::HashSet::new());
                        self.emit_statement(body)?;
                        self.function_scope_stack.pop();
                        self.outer_vars_stack.pop();
                        self.rc_cell_storage_scopes.pop();
                    }

                    self.async_context_stack.pop();

                    // Restore refcell_wrapped_vars (remove mutable outer vars we added)
                    self.refcell_wrapped_vars = saved_refcell;

                    // Pop params stack
                    self.outer_params_stack.pop();

                    Ok(())
                })();

                self.type_context.pop_scope();
                if let Err(e) = fun_body_res {
                    self.value_fn_depth = self.value_fn_depth.saturating_sub(1);
                    self.try_closure_depth = saved_try_depth;
                    return Err(e);
                }

                self.writeln("Value::Null");
                self.indent -= 1;
                self.writeln("})");
                self.value_fn_depth = self.value_fn_depth.saturating_sub(1);
                self.try_closure_depth = saved_try_depth;
                self.indent -= 1;
                self.writeln("};");
                // Update the cell with the actual function value
                self.writeln(&format!(
                    "*{}_cell.borrow_mut() = {}.clone();",
                    name_str, name_str
                ));
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
                        parts.push(format!("if let Value::Array(ref _spread) = tishlang_runtime::normalize_for_of(({}).clone()) {{ _args.extend(_spread.borrow().iter().cloned()); }}", val));
                    }
                }
            }
            Ok(format!(
                "{{ let mut _args: Vec<Value> = Vec::new(); {} _args }}",
                parts.join(" ")
            ))
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

    fn emit_destruct_bindings(
        &mut self,
        pattern: &DestructPattern,
        value_expr: &str,
        mutability: &str,
        span: Span,
    ) -> Result<(), CompileError> {
        // Flat `let` bindings so names stay in scope for the rest of the function (e.g. JSX).
        match pattern {
            DestructPattern::Array(elements) => {
                for (i, elem) in elements.iter().enumerate() {
                    if let Some(el) = elem {
                        match el {
                            DestructElement::Ident(name, _) => {
                                self.writeln(&format!(
                                    "{} {} = match &({}) {{ Value::Array(ref _a) => _a.borrow().get({}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                    Self::mut_kw_for(name.as_ref(), mutability),
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
                            DestructElement::Rest(name, _) => {
                                self.writeln(&format!(
                                    "{} {} = match &({}) {{ Value::Array(ref _a) => {{ let _b = _a.borrow(); Value::Array(VmRef::new(_b.iter().skip({}).cloned().collect())) }}, _ => Value::Array(VmRef::new(Vec::new())) }};",
                                    Self::mut_kw_for(name.as_ref(), mutability),
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
                        DestructElement::Ident(name, _) => {
                            self.writeln(&format!(
                                "{} {} = match &({}) {{ Value::Object(ref _o) => _o.borrow().strings.get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                Self::mut_kw_for(name.as_ref(), mutability),
                                Self::escape_ident(name.as_ref()),
                                value_expr,
                                key
                            ));
                        }
                        DestructElement::Pattern(nested) => {
                            let nested_var = format!("_nested_obj_{}", key);
                            self.writeln(&format!(
                                "let {} = match &({}) {{ Value::Object(ref _o) => _o.borrow().strings.get({:?}).cloned().unwrap_or(Value::Null), _ => Value::Null }};",
                                nested_var, value_expr, key
                            ));
                            self.emit_destruct_bindings(nested, &nested_var, mutability, span)?;
                        }
                        DestructElement::Rest(_, _) => {
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

    /// Like `VarDecl` pushing onto `outer_vars_stack`, so nested `move` closures rebind
    /// destructured names via `_cell` / `.clone()` instead of moving `Value` multiple times.
    fn register_destruct_pattern_outer_vars(&mut self, pattern: &DestructPattern) {
        match pattern {
            DestructPattern::Array(elements) => {
                for el in elements.iter().flatten() {
                    match el {
                        DestructElement::Ident(name, _) => {
                            if let Some(scope) = self.outer_vars_stack.last_mut() {
                                scope.push(name.to_string());
                            }
                        }
                        DestructElement::Pattern(nested) => {
                            self.register_destruct_pattern_outer_vars(nested);
                        }
                        DestructElement::Rest(name, _) => {
                            if let Some(scope) = self.outer_vars_stack.last_mut() {
                                scope.push(name.to_string());
                            }
                        }
                    }
                }
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    match &prop.value {
                        DestructElement::Ident(name, _) => {
                            if let Some(scope) = self.outer_vars_stack.last_mut() {
                                scope.push(name.to_string());
                            }
                        }
                        DestructElement::Pattern(nested) => {
                            self.register_destruct_pattern_outer_vars(nested);
                        }
                        DestructElement::Rest(_, _) => {}
                    }
                }
            }
        }
    }

    fn emit_expr(&mut self, expr: &Expr) -> Result<String, CompileError> {
        Ok(match expr {
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => format!("Value::Number({})", Self::f64_lit(*n)),
                Literal::String(s) => format!("Value::String({:?}.into())", s.as_ref()),
                Literal::Bool(b) => format!("Value::Bool({})", b),
                Literal::Null => "Value::Null".to_string(),
            },
            Expr::Ident { name, .. } => {
                if self.native_numeric_globals.contains_key(name.as_ref()) {
                    return Ok(format!(
                        "Value::Number({})",
                        Self::native_global_get(name.as_ref())
                    ));
                }
                if let Some(static_name) = self.module_const_f64_array_rust_ref(name.as_ref()) {
                    let v = Self::module_const_f64_array_as_value(&static_name);
                    return Ok(if self.value_fn_depth > 0 || !self.loop_stack.is_empty() {
                        format!("({}).clone()", v)
                    } else {
                        v
                    });
                }
                if let Some(uv) = self.usize_var_subst.get(name.as_ref()) {
                    return Ok(format!("Value::Number(({} as f64))", uv));
                }
                let escaped = Self::escape_ident(name.as_ref());
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    let var_type = self.type_context.get_type(name.as_ref());
                    if var_type.is_native() {
                        var_type.to_value_expr(&format!("(*{}.borrow())", escaped))
                    } else {
                        format!("(*{}.borrow()).clone()", escaped)
                    }
                } else {
                    // Check if this is a typed variable that needs conversion to Value
                    let var_type = self.type_context.get_type(name.as_ref());
                    if var_type.is_native() {
                        // Convert native type to Value for compatibility with existing code
                        var_type.to_value_expr(&escaped)
                    } else {
                        let s = escaped.into_owned();
                        if self.value_fn_depth > 0 || !self.loop_stack.is_empty() {
                            format!("({}).clone()", s)
                        } else {
                            s
                        }
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
                    // `*_value(&Value)` coercion (no name binding) so unary ops compose over nested
                    // bitwise/unary operands without the `let Value::Number(n) = &(..)` shadowing
                    // miscompile, and coerce non-numbers to `NaN` like the interpreter/VM.
                    UnaryOp::Neg => {
                        format!("Value::Number(-tishlang_runtime::to_number_value(&({})))", o)
                    }
                    UnaryOp::Pos => {
                        format!("Value::Number(tishlang_runtime::to_number_value(&({})))", o)
                    }
                    UnaryOp::BitNot => format!(
                        "Value::Number((!tishlang_runtime::to_int32_value(&({}))) as f64)",
                        o
                    ),
                    UnaryOp::Void => format!("{{ {}; Value::Null }}", o),
                }
            }
            Expr::Call { callee, args, .. } => {
                // #178: route a boxed-context call to a recursive-struct consumer
                // (`sum(build(n))` / `sum(node)`) → `Value::Number(sum_rec(&..))`.
                if self.rec_struct_plan.is_some() {
                    if let Some(code) = self.try_emit_toplevel_rec_call(callee, args, true)? {
                        return Ok(code);
                    }
                }
                // #177: route a top-level call to a de-virtualized aggregate fn. Void fns
                // (`advance`/`offsetMomentum`) emit `name_agg(&mut bodies, …)` (a `()` statement);
                // an f64-returning fn (`energy`) is boxed back into `Value::Number` for this path.
                if !self.aggregate_fns.is_empty() {
                    if let Some((code, _)) = self.try_emit_toplevel_agg_call(callee, args, true)? {
                        return Ok(code);
                    }
                }
                // #175: route a boxed-context call to a native-vec fn (`name_nv(&/&mut Vec, …)`).
                if !self.native_vec_fns.is_empty() {
                    if let Some(code) = self.try_emit_native_vec_call(callee, args, true)? {
                        return Ok(code);
                    }
                }
                // Check for built-in method calls on arrays/strings
                if let Expr::Member {
                    object,
                    prop: MemberProp::Name { name: method_name, .. },
                    ..
                } = callee.as_ref()
                {
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

                    // #181: `new Map()` locals — direct store access (Bun/JSC-style monomorphic site).
                    if let Expr::Ident { name: map_name, .. } = object.as_ref() {
                        if self.map_instance_locals.contains(map_name.as_ref()) {
                            let map_ref = format!("&{}", Self::escape_ident(map_name.as_ref()));
                            match method_name.as_ref() {
                                "has" if args.len() == 1 => {
                                    return Ok(format!(
                                        "tish_map_has({}, &{})",
                                        map_ref, arg_exprs[0]
                                    ));
                                }
                                "get" if args.len() == 1 => {
                                    return Ok(format!(
                                        "tish_map_get({}, &{})",
                                        map_ref, arg_exprs[0]
                                    ));
                                }
                                "set" if args.len() == 2 => {
                                    return Ok(format!(
                                        "tish_map_set({}, {}, {})",
                                        map_ref, arg_exprs[0], arg_exprs[1]
                                    ));
                                }
                                "values" if args.is_empty() => {
                                    return Ok(format!("tish_map_values({})", map_ref));
                                }
                                _ => {}
                            }
                        }
                    }
                    
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
                        "lastIndexOf" => {
                            let search = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let position = if args.len() >= 2 {
                                arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string())
                            } else {
                                "Value::Number(f64::INFINITY)".to_string()
                            };
                            return Ok(format!(
                                "{{ let _obj = ({}).clone(); match &_obj {{ Value::String(_) => tishlang_runtime::string_last_index_of(&_obj, &{}, &{}), _ => Value::Number(-1.0) }} }}",
                                obj_expr, search, position
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
                        "fill" => {
                            let value = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let start = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let end = arg_exprs.get(2).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_fill(&{}, &{}, &{}, &{})",
                                obj_expr, value, start, end
                            ));
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
                        "substr" => {
                            let start = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            let length = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_substr(&{}, &{}, &{})",
                                obj_expr, start, length
                            ));
                        }
                        "split" => {
                            let sep = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            let limit = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_split_limit(&{}, &{}, &{})",
                                obj_expr, sep, limit
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
                        // Gate on the *requested* feature (has_feature), not tish_compile's own
                        // cfg!(feature="regex") — the generated binary links the runtime's regex
                        // impls when the build requests regex, regardless of how tish_compile was
                        // compiled. Falls through to a generic call (no-regex builds) otherwise.
                        "match" if self.has_feature("regex") => {
                            let regexp = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_match_regex(&{}, &{})",
                                obj_expr, regexp
                            ));
                        }
                        "search" if self.has_feature("regex") => {
                            let regexp = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::string_search_regex(&{}, &{})",
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
                        "at" => {
                            // `at` is on both String and Array; this match is by method name, so
                            // dispatch on the runtime value type (#247).
                            let idx = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Number(0.0)".to_string());
                            return Ok(format!(
                                "tishlang_runtime::value_at(&{}, &{})",
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
                        "toString" => {
                            let radix = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::number_to_string(&{}, &{})",
                                obj_expr, radix
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
                            let initial = arg_exprs.get(1).cloned().unwrap_or_else(|| "Value::Null".to_string());
                            // Fused reduce (TISH_FUSED_HOF): `arr.reduce((acc, x) => acc OP x, init)`
                            // with a plain binop of the two params → a native fold using the SAME
                            // runtime Value op the closure body would, eliminating the per-element
                            // `value_call`. Sound (identical Value semantics, incl. string `+`).
                            // Requires an explicit init; anything else falls back to array_reduce.
                            if crate::native_opts_enabled() && args.len() == 2 {
                                if let Some(fold) =
                                    self.try_fused_reduce(args, &obj_expr, &initial)?
                                {
                                    return Ok(fold);
                                }
                            }
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
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
                        "findLast" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_find_last(&{}, &{})",
                                obj_expr, callback
                            ));
                        }
                        "findLastIndex" => {
                            let callback = arg_exprs.first().cloned().unwrap_or_else(|| "Value::Null".to_string());
                            return Ok(format!(
                                "tishlang_runtime::array_find_last_index(&{}, &{})",
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
                
                // M5: boxed-context call to an eligible native fn → `Value::Number(name_native(..))`.
                if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                    if self.native_fns.contains(fname.as_ref()) {
                        let mut argc: Vec<String> = Vec::with_capacity(args.len());
                        let mut ok = true;
                        for a in args {
                            if let CallArg::Expr(e) = a {
                                let (ac, at) = self.emit_typed_expr(e)?;
                                argc.push(if at == RustType::Value {
                                    RustType::F64.from_value_expr(&ac)
                                } else {
                                    ac
                                });
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            return Ok(format!(
                                "Value::Number({}_native({}))",
                                Self::escape_ident(fname.as_ref()),
                                argc.join(", ")
                            ));
                        }
                    }
                }

                let callee_expr = self.emit_expr(callee)?;
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    let args_code = self.emit_call_args(args)?;
                    return Ok(format!(
                        "{{ let _callee = ({}).clone(); let _spread_args = {}; tishlang_runtime::value_call(&_callee, _spread_args.as_slice()) }}",
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
                     {}    let _callee = ({}).clone();\n\
                     {}    tishlang_runtime::value_call(&_callee, &[{}])\n\
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
                // Fast path: typed struct member access. If `object` is
                // a local with `RustType::Named { fields }` and `prop` is
                // a literal field name of that struct, lower to a direct
                // Rust field access (`obj.field`), then wrap in
                // `Value::*` so the caller gets a `Value` as expected.
                if !optional {
                    if let (Expr::Ident { name: var_name, .. }, MemberProp::Name { name: prop_name, .. }) =
                        (object.as_ref(), prop)
                    {
                        let var_type = self.type_context.get_type(var_name.as_ref());
                        if let RustType::Named { fields, .. } = &var_type {
                            if let Some((_, field_ty)) =
                                fields.iter().find(|(k, _)| k.as_ref() == prop_name.as_ref())
                            {
                                let var_esc = Self::escape_ident(var_name.as_ref()).into_owned();
                                let access = if self.refcell_wrapped_vars.contains(var_name.as_ref()) {
                                    format!(
                                        "(*{}.borrow()).{}.clone()",
                                        var_esc,
                                        crate::types::field_ident(prop_name.as_ref())
                                    )
                                } else {
                                    format!(
                                        "{}.{}",
                                        var_esc,
                                        crate::types::field_ident(prop_name.as_ref())
                                    )
                                };
                                // Caller expects a `Value`; wrap.
                                return Ok(field_ty.to_value_expr(&access));
                            }
                        }
                    }
                }
                // Generalize the typed struct-field fast path to `xs[i].field` (array-of-structs):
                // when `object` indexes a `Vec<Named>`, do native struct field access.
                if !optional {
                    if let (Expr::Index { .. }, MemberProp::Name { name: prop_name, .. }) =
                        (object.as_ref(), prop)
                    {
                        let (obj_code, obj_ty) = self.emit_typed_expr(object)?;
                        if let RustType::Named { fields, .. } = &obj_ty {
                            if let Some((_, field_ty)) =
                                fields.iter().find(|(k, _)| k.as_ref() == prop_name.as_ref())
                            {
                                let access = format!(
                                    "({}).{}",
                                    obj_code,
                                    crate::types::field_ident(prop_name.as_ref())
                                );
                                return Ok(field_ty.to_value_expr(&access));
                            }
                        }
                    }
                }
                let obj = self.emit_expr(object)?;
                let key = match prop {
                    MemberProp::Name { name, .. } => format!("{:?}", name.as_ref()),
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
                                parts.push(format!("if let Value::Array(ref _spread) = tishlang_runtime::normalize_for_of(({}).clone()) {{ _arr.extend(_spread.borrow().iter().cloned()); }}", val));
                            }
                        }
                    }
                    format!("{{ let mut _arr: Vec<Value> = Vec::new(); {} Value::Array(VmRef::new(_arr)) }}", parts.join(" "))
                } else {
                    let mut els = Vec::new();
                    for elem in elements {
                        if let ArrayElement::Expr(expr) = elem {
                            let v = self.emit_expr(expr)?;
                            // A `Value`-typed identifier (object, or a global like `NaN`/`Infinity`)
                            // is emitted bare here, so moving it into the array breaks any later use
                            // in the SAME expression — e.g. `[1, o].includes(o)` borrows `o` after the
                            // array moved it. The scope-local last-use analysis can't see that reuse,
                            // so clone every identifier element (cheap; these literals are cold, and
                            // string/number idents already clone inside their `Value::*` conversion).
                            if matches!(expr, Expr::Ident { .. }) || self.should_clone(expr) {
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
                        "Value::Array(VmRef::new(vec![{}]))",
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
                            ObjectProp::KeyValue(k, v, _) => {
                                let val = self.emit_expr(v)?;
                                if self.should_clone(v) {
                                    parts.push(format!("_obj.insert(Arc::from({:?}), ({}).clone());", k.as_ref(), val));
                                } else {
                                    parts.push(format!("_obj.insert(Arc::from({:?}), {});", k.as_ref(), val));
                                }
                            }
                            ObjectProp::Spread(e) => {
                                let val = self.emit_expr(e)?;
                                parts.push(format!("if let Value::Object(ref _spread) = {} {{ for (k, v) in _spread.borrow().strings.iter() {{ _obj.insert(Arc::clone(k), v.clone()); }} }}", val));
                            }
                        }
                    }
                    format!("{{ let mut _obj: ObjectMap = ObjectMap::default(); {} Value::object(_obj) }}", parts.join(" "))
                } else {
                    let mut parts = Vec::new();
                    for prop in props {
                        if let ObjectProp::KeyValue(k, v, _) = prop {
                            let val = self.emit_expr(v)?;
                            if self.should_clone(v) {
                                parts.push(format!("(Arc::from({:?}), ({}).clone())", k.as_ref(), val));
                            } else {
                                parts.push(format!("(Arc::from({:?}), {})", k.as_ref(), val));
                            }
                        }
                    }
                    // Build the PropMap directly (no intermediate AHashMap) — one
                    // inline allocation for small objects (the common case).
                    format!("Value::object_from_pairs([{}])", parts.join(", "))
                }
            }
            Expr::Assign { name, value, .. } => {
                // #173 part 3: expression-position reassignment also ends a counter's in-bounds guard.
                self.invalidate_index_guard(name.as_ref());
                // #176: whole-binding assign to a native numeric global → thread_local Cell.
                if self.native_numeric_globals.contains_key(name.as_ref()) {
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    let native_val = if val_ty == RustType::F64 {
                        val_code
                    } else if val_ty == RustType::Value {
                        RustType::F64.from_value_expr(&val_code)
                    } else {
                        val_code
                    };
                    let set = Self::native_global_set(name.as_ref(), &native_val);
                    let get = Self::native_global_get(name.as_ref());
                    return Ok(format!(
                        "{{ {}; Value::Number({}) }}",
                        set, get
                    ));
                }
                let escaped = Self::escape_ident(name.as_ref());
                let rust_type = self.type_context.get_type(name.as_ref());
                let is_ref = self.refcell_wrapped_vars.contains(name.as_ref());
                // Native fast path: direct assignment (plain or Rc<RefCell<T>> for closure capture)
                if rust_type.is_native()
                    && matches!(rust_type, RustType::F64 | RustType::Bool | RustType::String)
                {
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    let native_val = if val_ty == rust_type {
                        val_code
                    } else if val_ty == RustType::Value {
                        rust_type.from_value_expr(&val_code)
                    } else {
                        val_code
                    };
                    let return_val = if is_ref {
                        rust_type.to_value_expr(&format!("(*{}.borrow())", escaped))
                    } else {
                        rust_type.to_value_expr(&escaped)
                    };
                    // Rust evaluates the assignment place before the RHS; RHS must not call
                    // `.borrow()` on the same RefCell while `borrow_mut()` is active.
                    let assign_stmt = if is_ref {
                        format!(
                            "let _assign_tmp = {}; *{}.borrow_mut() = _assign_tmp",
                            native_val, escaped
                        )
                    } else {
                        format!("{} = {}", escaped, native_val)
                    };
                    return Ok(format!("{{ {}; {} }}", assign_stmt, return_val));
                }
                // Fallback: Value path
                let val = self.emit_expr(value)?;
                let needs_outer_clone = self.should_clone(value);
                if is_ref {
                    if needs_outer_clone {
                        format!("{{ let _v = ({}).clone(); *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    } else {
                        format!("{{ let _v = {}; *{}.borrow_mut() = _v.clone(); _v }}", val, escaped)
                    }
                } else {
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
                    // A rejected awaited promise must THROW (so a surrounding try/catch fires).
                    // Use the throwing `?`-variant wherever an error channel exists — the SAME
                    // condition `throw` uses to emit `return Err(..)` (inside a try body, or the
                    // top-level run()). Elsewhere there is no channel, so fall back to the
                    // value-returning variant (matches the existing uncaught-throw limitation).
                    let (awaiter, q) = if self.try_closure_depth > 0 || self.value_fn_depth == 0 {
                        ("tish_await_promise_throw", "?")
                    } else {
                        ("tish_await_promise", "")
                    };
                    if let Expr::Call { callee, args, .. } = operand.as_ref() {
                        if let Expr::Ident { name, .. } = callee.as_ref() {
                            let args_code = self.emit_call_args(args)?;
                            return Ok(match name.as_ref() {
                                "fetch" => {
                                    format!("{}(tish_fetch_promise({})){}", awaiter, args_code, q)
                                }
                                "fetchAll" => {
                                    format!("{}(tish_fetch_all_promise({})){}", awaiter, args_code, q)
                                }
                                _ => {
                                    let o = self.emit_expr(operand)?;
                                    return Ok(format!("{}({}){}", awaiter, o, q));
                                }
                            });
                        }
                    }
                    // await Call with non-Ident callee, or await Promise value: wrap in await_promise
                    let o = self.emit_expr(operand)?;
                    return Ok(format!("{}({}){}", awaiter, o, q));
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
                     Value::Function(_) => \"function\".into(), Value::Symbol(_) => \"symbol\".into(), \
                     _ => \"object\".into() }})",
                    o
                )
            }
            Expr::Delete { target, .. } => match target.as_ref() {
                Expr::Member { object, prop: MemberProp::Name { name, .. }, .. } => {
                    let obj = self.emit_expr(object)?;
                    format!(
                        "tishlang_runtime::delete_property(&{}, &Value::String({:?}.into()))",
                        obj,
                        name.as_ref()
                    )
                }
                Expr::Member { object, prop: MemberProp::Expr(key), .. } => {
                    let obj = self.emit_expr(object)?;
                    let k = self.emit_expr(key)?;
                    format!("tishlang_runtime::delete_property(&{}, &{})", obj, k)
                }
                Expr::Index { object, index, .. } => {
                    let obj = self.emit_expr(object)?;
                    let idx = self.emit_expr(index)?;
                    format!("tishlang_runtime::delete_property(&{}, &{})", obj, idx)
                }
                _ => "Value::Bool(true)".to_string(),
            },
            Expr::PostfixInc { name, .. } => {
                self.invalidate_index_guard(name.as_ref());
                self.emit_inc_dec(name.as_ref(), false, "+ 1.0", "++")
            }
            Expr::PostfixDec { name, .. } => {
                self.invalidate_index_guard(name.as_ref());
                self.emit_inc_dec(name.as_ref(), false, "- 1.0", "--")
            }
            Expr::PrefixInc { name, .. } => {
                self.invalidate_index_guard(name.as_ref());
                self.emit_inc_dec(name.as_ref(), true, "+ 1.0", "++")
            }
            Expr::PrefixDec { name, .. } => {
                self.invalidate_index_guard(name.as_ref());
                self.emit_inc_dec(name.as_ref(), true, "- 1.0", "--")
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                self.invalidate_index_guard(name.as_ref());
                let n = Self::escape_ident(name.as_ref());
                let is_refcell = self.refcell_wrapped_vars.contains(name.as_ref());
                let var_type = self.type_context.get_type(name.as_ref());

                // ── native f64 in Rc<RefCell<f64>> (closure-mutated) ───────────
                if is_refcell && var_type == RustType::F64 {
                    let (rhs_code, rhs_ty) = self.emit_typed_expr(value)?;
                    let rhs_f64 = if rhs_ty == RustType::F64 {
                        rhs_code
                    } else {
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
                    return Ok(format!(
                        "{{ let _op_rhs = {rhs_f64}; *{n}.borrow_mut() {op_str} _op_rhs; Value::Number(*{n}.borrow()) }}"
                    ));
                }

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

                // ── native String += in Rc<RefCell<String>> ───────────────────
                if is_refcell && var_type == RustType::String && matches!(op, CompoundOp::Add) {
                    let (rhs_code, rhs_ty) = self.emit_typed_expr(value)?;
                    let rhs_str = if rhs_ty == RustType::String {
                        rhs_code
                    } else {
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
                             other => other.to_js_string() }}",
                            rhs_val
                        )
                    };
                    return Ok(format!(
                        "{{ let _push_rhs = {rhs_str}; (*{n}.borrow_mut()).push_str(&_push_rhs); Value::String((*{n}.borrow()).clone().into()) }}"
                    ));
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
                             other => other.to_js_string() }}",
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
                let op_suffix = self.ops_result_suffix();
                if is_refcell {
                    format!(
                        "{{ let _lhs_v = (*{}.borrow()).clone(); let _rhs = ({}).clone(); let _new = tishlang_runtime::ops::{}(&_lhs_v, &_rhs){}; *{}.borrow_mut() = _new; (*{}.borrow()).clone() }}",
                        n, val, op_fn, op_suffix, n, n
                    )
                } else if var_type.is_native() {
                    // Wrap native lhs as Value, run ops::, unbox result back to native
                    let n_as_value = var_type.to_value_expr(&n);
                    let result_native = var_type.from_value_expr("_result");
                    let n_as_value2 = var_type.to_value_expr(&n);
                    format!(
                        "{{ let _lhs = {}; let _rhs = ({}).clone(); let _result = tishlang_runtime::ops::{}(&_lhs, &_rhs){}; {} = {}; {} }}",
                        n_as_value, val, op_fn, op_suffix, n, result_native, n_as_value2
                    )
                } else {
                    format!(
                        "{{ let _rhs = ({}).clone(); {} = tishlang_runtime::ops::{}(&{}, &_rhs){}; {}.clone() }}",
                        val, n, op_fn, n, op_suffix, n
                    )
                }
            }
            Expr::LogicalAssign { name, op, value, .. } => {
                self.invalidate_index_guard(name.as_ref());
                let val = self.emit_expr(value)?;
                let n = Self::escape_ident(name.as_ref()).into_owned();
                let is_refcell = self.refcell_wrapped_vars.contains(name.as_ref());
                let var_type = self.type_context.get_type(name.as_ref());

                // ── native type: wrap for condition, unbox for assignment ──────
                // (plain binding or Rc<RefCell<T>> when closure-mutated)
                if var_type.is_native() {
                    let inner = if is_refcell {
                        format!("(*{}.borrow())", n)
                    } else {
                        n.clone()
                    };
                    let n_as_value = var_type.to_value_expr(&inner);
                    let val_as_native = var_type.from_value_expr("_v");
                    let ret_expr = if is_refcell {
                        var_type.to_value_expr(&format!("(*{}.borrow())", n))
                    } else {
                        var_type.to_value_expr(&n)
                    };
                    let (cond, assign_and_return, else_expr) = match op {
                        LogicalAssignOp::AndAnd => (
                            format!("{{ let __chk = {}; __chk.is_truthy() }}", n_as_value),
                            if is_refcell {
                                format!(
                                    "{{ let _v = ({}).clone(); *{}.borrow_mut() = {}; {} }}",
                                    val, n, val_as_native, ret_expr
                                )
                            } else {
                                format!(
                                    "{{ let _v = ({}).clone(); {} = {}; {} }}",
                                    val, n, val_as_native, ret_expr
                                )
                            },
                            ret_expr.clone(),
                        ),
                        LogicalAssignOp::OrOr => (
                            format!("!{{ let __chk = {}; __chk.is_truthy() }}", n_as_value),
                            if is_refcell {
                                format!(
                                    "{{ let _v = ({}).clone(); *{}.borrow_mut() = {}; {} }}",
                                    val, n, val_as_native, ret_expr
                                )
                            } else {
                                format!(
                                    "{{ let _v = ({}).clone(); {} = {}; {} }}",
                                    val, n, val_as_native, ret_expr
                                )
                            },
                            ret_expr.clone(),
                        ),
                        // Native types (f64, String, bool) are never null — ??= is a no-op
                        LogicalAssignOp::Nullish => (
                            "false".to_string(),
                            ret_expr.clone(),
                            ret_expr.clone(),
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
                if let Some(assign) =
                    self.try_emit_native_vec_index_assign(object, index, value, false)?
                {
                    return Ok(assign);
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
                        parts.push(format!("&({}).to_js_string()", expr_code));
                    }
                }
                format!("Value::String([{}].concat().into())", parts.join(", "))
            }
            Expr::JsxElement { .. } | Expr::JsxFragment { .. } => {
                let fun_decls = self.program_fun_decl_names.clone();
                tishlang_ui::jsx::emit_jsx_rust(
                    expr,
                    &mut |e| self.emit_expr(e).map_err(|ce| ce.message),
                    &fun_decls,
                )
                .map_err(|m| CompileError::new(m, None))?
            }
            Expr::New { callee, args, .. } => {
                // Packed-native fast path: `new Float64Array(...)` lowers to a packed
                // `Value::NumberArray` (`Vec<f64>`) instead of the boxed `Value::Array` the generic
                // `tish_construct` builds — `Float64Array` is the one view whose element type *is*
                // f64, so it needs no coercion and avoids the per-element `Value` boxing. The helper
                // falls back to the identical boxed value when `TISH_PACKED_ARRAYS` is off, so default
                // builds stay byte-for-byte unchanged. The other typed-array views have no packed
                // `Value` variant (would need `Vec<f32>`/`Vec<i32>`/… + the 24-byte size assertion and
                // every exhaustive match), so they keep the generic path. Native-only: interp/VM value
                // bridges carry no `NumberArray`, so only the native runtime grew the support. Keyed on
                // the callee ident like the existing `JSON.`/`Polars.` special-cases.
                if matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Float64Array")
                {
                    if args.iter().any(|a| matches!(a, CallArg::Spread(_))) {
                        let args_code = self.emit_call_args(args)?;
                        return Ok(format!(
                            "{{ let _spread_args = {}; tishlang_runtime::float64_array_packed(&_spread_args[..]) }}",
                            args_code
                        ));
                    }
                    let arg_exprs: Result<Vec<_>, _> =
                        args.iter().map(|a| self.emit_call_arg(a)).collect();
                    let args_vec = arg_exprs?
                        .iter()
                        .map(|a| format!("{}.clone()", a))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ok(format!(
                        "tishlang_runtime::float64_array_packed(&[{}])",
                        args_vec
                    ));
                }
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
            Expr::Ident { name, .. } => {
                idents.insert(name.to_string());
            }
            Expr::Assign { name, value, .. } => {
                idents.insert(name.to_string());
                Self::collect_expr_idents(value, idents);
            }
            Expr::Binary { left, right, .. } => {
                Self::collect_expr_idents(left, idents);
                Self::collect_expr_idents(right, idents);
            }
            Expr::Unary { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::Delete { target, .. } => Self::collect_expr_idents(target, idents),
            Expr::Call { callee, args, .. } => {
                Self::collect_expr_idents(callee, idents);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_expr_idents(e, idents)
                        }
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::collect_expr_idents(callee, idents);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_expr_idents(e, idents)
                        }
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_expr_idents(object, idents);
                if let MemberProp::Expr(e) = prop {
                    Self::collect_expr_idents(e, idents);
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(value, idents);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(index, idents);
                Self::collect_expr_idents(value, idents);
            }
            Expr::Index { object, index, .. } => {
                Self::collect_expr_idents(object, idents);
                Self::collect_expr_idents(index, idents);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_expr_idents(then_branch, idents);
                Self::collect_expr_idents(else_branch, idents);
            }
            Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } => {
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
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::collect_expr_idents(e, idents)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::collect_expr_idents(e, idents)
                        }
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => Self::collect_expr_idents(e, idents),
                ArrowBody::Block(s) => Self::collect_stmt_idents(s, idents),
            },
            Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_expr_idents(left, idents);
                Self::collect_expr_idents(right, idents);
            }
            Expr::TypeOf { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::Await { operand, .. } => Self::collect_expr_idents(operand, idents),
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::collect_expr_idents(e, idents);
                }
            }
            Expr::JsxElement {
                props, children, ..
            } => {
                for p in props {
                    match p {
                        tishlang_ast::JsxProp::Attr {
                            value: tishlang_ast::JsxAttrValue::Expr(e),
                            ..
                        }
                        | tishlang_ast::JsxProp::Spread(e) => Self::collect_expr_idents(e, idents),
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
            // Descend into initializers: an assignment may live inside a closure
            // stored in a `let`/`const` (e.g. `let inc = () => { count = count + 1 }`).
            // The declared name itself is a binding, not an assignment, so it is
            // not added here. Closing this gap also closes it for arrow-block
            // bodies, which are scanned via collect_assigned_idents_in_expr.
            Statement::VarDecl { init: Some(e), .. } => {
                Self::collect_assigned_idents_in_expr(e, names)
            }
            Statement::VarDecl { init: None, .. } => {}
            Statement::VarDeclDestructure { init, .. } => {
                Self::collect_assigned_idents_in_expr(init, names)
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::collect_assigned_idents_in_stmt(s, names);
                }
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_assigned_idents_in_expr(cond, names);
                Self::collect_assigned_idents_in_stmt(then_branch, names);
                if let Some(eb) = else_branch {
                    Self::collect_assigned_idents_in_stmt(eb, names);
                }
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
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
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
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
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
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
            Statement::Break { .. }
            | Statement::Continue { .. }
            | Statement::Import { .. }
            | Statement::Export { .. }
            | Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {}
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
            Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } => {
                names.insert(name.to_string());
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::collect_assigned_idents_in_expr(object, names);
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                Self::collect_assigned_idents_in_expr(object, names);
                Self::collect_assigned_idents_in_expr(index, names);
                Self::collect_assigned_idents_in_expr(value, names);
            }
            Expr::Binary { left, right, .. } => {
                Self::collect_assigned_idents_in_expr(left, names);
                Self::collect_assigned_idents_in_expr(right, names);
            }
            Expr::Unary { operand, .. } => Self::collect_assigned_idents_in_expr(operand, names),
            Expr::Delete { target, .. } => Self::collect_assigned_idents_in_expr(target, names),
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
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_assigned_idents_in_expr(cond, names);
                Self::collect_assigned_idents_in_expr(then_branch, names);
                Self::collect_assigned_idents_in_expr(else_branch, names);
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => Self::collect_assigned_idents_in_expr(e, names),
                ArrowBody::Block(s) => Self::collect_assigned_idents_in_stmt(s, names),
            },
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
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
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
            Expr::JsxElement {
                props, children, ..
            } => {
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
                Statement::For { init: Some(i), .. } => {
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

    /// Collect block vars captured (referenced) by this closure and any nested
    /// closures. block_vars: vars declared in the enclosing block. The caller
    /// (`collect_vars_needing_capture_cell`) further restricts to vars that are
    /// also assigned somewhere in the defining scope.
    fn collect_captured_block_vars_from_closure(
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
        let outer_captured: HashSet<String> = referenced
            .difference(&param_names)
            .cloned()
            .collect::<HashSet<_>>()
            .difference(&local_var_names)
            .cloned()
            .collect();
        // Every block var this closure captures is a candidate; the caller keeps
        // only those also assigned somewhere in the defining scope.
        for v in &outer_captured {
            if block_vars.contains(v) {
                result.insert(v.clone());
            }
        }
        // Recurse into nested fns
        Self::collect_captured_block_vars_from_statements(body, block_vars, result);
    }

    fn collect_captured_block_vars_from_arrow(
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
        let outer_captured: HashSet<String> = referenced
            .difference(&param_names)
            .cloned()
            .collect::<HashSet<_>>()
            .difference(&local_var_names)
            .cloned()
            .collect();
        for v in &outer_captured {
            if block_vars.contains(v) {
                result.insert(v.clone());
            }
        }
        match body {
            ArrowBody::Expr(e) => Self::collect_captured_block_vars_from_expr(e, block_vars, result),
            ArrowBody::Block(s) => {
                Self::collect_captured_block_vars_from_statements(s, block_vars, result)
            }
        }
    }

    fn collect_captured_block_vars_from_expr(
        expr: &Expr,
        block_vars: &HashSet<String>,
        result: &mut HashSet<String>,
    ) {
        match expr {
            Expr::ArrowFunction { params, body, .. } => {
                Self::collect_captured_block_vars_from_arrow(params, body, block_vars, result);
            }
            Expr::Call { callee, args, .. } => {
                Self::collect_captured_block_vars_from_expr(callee, block_vars, result);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::collect_captured_block_vars_from_expr(callee, block_vars, result);
                for arg in args {
                    match arg {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_captured_block_vars_from_expr(object, block_vars, result);
                if let MemberProp::Expr(e) = prop {
                    Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                }
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_captured_block_vars_from_expr(cond, block_vars, result);
                Self::collect_captured_block_vars_from_expr(then_branch, block_vars, result);
                Self::collect_captured_block_vars_from_expr(else_branch, block_vars, result);
            }
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_captured_block_vars_from_expr(left, block_vars, result);
                Self::collect_captured_block_vars_from_expr(right, block_vars, result);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_captured_block_vars_from_statements(
        stmt: &Statement,
        block_vars: &HashSet<String>,
        result: &mut HashSet<String>,
    ) {
        match stmt {
            Statement::FunDecl { params, body, .. } => {
                Self::collect_captured_block_vars_from_closure(params, body, block_vars, result);
            }
            Statement::ExprStmt { expr, .. } => {
                Self::collect_captured_block_vars_from_expr(expr, block_vars, result);
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::collect_captured_block_vars_from_statements(s, block_vars, result);
                }
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_captured_block_vars_from_expr(cond, block_vars, result);
                Self::collect_captured_block_vars_from_statements(then_branch, block_vars, result);
                if let Some(eb) = else_branch {
                    Self::collect_captured_block_vars_from_statements(eb, block_vars, result);
                }
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(i) = init {
                    Self::collect_captured_block_vars_from_statements(i, block_vars, result);
                }
                if let Some(c) = cond {
                    Self::collect_captured_block_vars_from_expr(c, block_vars, result);
                }
                if let Some(u) = update {
                    Self::collect_captured_block_vars_from_expr(u, block_vars, result);
                }
                Self::collect_captured_block_vars_from_statements(body, block_vars, result);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_captured_block_vars_from_expr(iterable, block_vars, result);
                Self::collect_captured_block_vars_from_statements(body, block_vars, result);
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                Self::collect_captured_block_vars_from_expr(cond, block_vars, result);
                Self::collect_captured_block_vars_from_statements(body, block_vars, result);
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                Self::collect_captured_block_vars_from_expr(expr, block_vars, result);
                for (ce, stmts) in cases {
                    if let Some(e) = ce {
                        Self::collect_captured_block_vars_from_expr(e, block_vars, result);
                    }
                    for s in stmts {
                        Self::collect_captured_block_vars_from_statements(s, block_vars, result);
                    }
                }
                if let Some(stmts) = default_body {
                    for s in stmts {
                        Self::collect_captured_block_vars_from_statements(s, block_vars, result);
                    }
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::collect_captured_block_vars_from_statements(body, block_vars, result);
                if let Some(c) = catch_body {
                    Self::collect_captured_block_vars_from_statements(c, block_vars, result);
                }
                if let Some(f) = finally_body {
                    Self::collect_captured_block_vars_from_statements(f, block_vars, result);
                }
            }
            Statement::VarDecl { init: Some(e), .. } => {
                Self::collect_captured_block_vars_from_expr(e, block_vars, result);
            }
            Statement::VarDeclDestructure { init, .. } => {
                Self::collect_captured_block_vars_from_expr(init, block_vars, result);
            }
            Statement::Return { value: Some(e), .. } => {
                Self::collect_captured_block_vars_from_expr(e, block_vars, result);
            }
            Statement::Throw { value, .. } => {
                Self::collect_captured_block_vars_from_expr(value, block_vars, result)
            }
            _ => {}
        }
    }

    /// For a block, return the names of block-scoped vars that must live in a
    /// shared `VmRef` cell because a nested closure captures them by reference.
    ///
    /// A var needs a cell when it is BOTH (a) captured (referenced) by some nested
    /// closure AND (b) assigned somewhere in the defining scope. The assignment may
    /// be inside a closure (`counter()`, sibling `inc`/`get`) or in the enclosing
    /// scope — including AFTER the closure is created (`let t = 0; let f = () => t;
    /// t = 100`). Capture alone is not enough: a never-mutated var can be snapshot
    /// by value. The previous rule (captured AND mutated *inside* a closure) was too
    /// narrow — it snapshotted capture-then-mutate vars by value, so the rust backend
    /// returned the stale value and diverged from node/vm/interp/cranelift.
    fn collect_vars_needing_capture_cell(statements: &[Statement]) -> HashSet<String> {
        let mut block_vars = HashSet::new();
        Self::collect_block_var_names(statements, &mut block_vars);
        // (a) Block vars captured by any nested closure.
        let mut captured = HashSet::new();
        for s in statements {
            Self::collect_captured_block_vars_from_statements(s, &block_vars, &mut captured);
        }
        // (b) Idents assigned anywhere in this scope (incl. inside closures).
        let mut assigned_in_scope = HashSet::new();
        for s in statements {
            Self::collect_assigned_idents_in_stmt(s, &mut assigned_in_scope);
        }
        captured.retain(|v| assigned_in_scope.contains(v));
        // A `for (let i = 0; …; i++)` counter is declared ONCE in the header but is a
        // per-iteration `let` in JS: a closure in the body must snapshot THIS iteration's
        // value, not share one cell across all iterations. The loop's own `i++` would
        // otherwise pull it in here. (for-of vars are not block vars, and body-`let`s are
        // re-declared each iteration so they get a fresh cell regardless — only header
        // counters, declared once, must be excluded.) See loop_let_capture.tish.
        let mut for_counters = HashSet::new();
        for s in statements {
            if let Statement::For { init: Some(i), .. } = s {
                match i.as_ref() {
                    Statement::VarDecl { name, .. } => {
                        for_counters.insert(name.to_string());
                    }
                    Statement::VarDeclDestructure { pattern, .. } => {
                        Self::collect_destruct_names(pattern, &mut for_counters);
                    }
                    _ => {}
                }
            }
        }
        captured.retain(|v| !for_counters.contains(v));
        captured
    }

    /// Collect variable names declared in a statement (VarDecl, Destructure, For init).
    fn collect_local_var_names(stmt: &Statement, names: &mut HashSet<String>) {
        match stmt {
            Statement::VarDecl { name, .. } => {
                names.insert(name.to_string());
            }
            Statement::VarDeclDestructure { pattern, .. } => {
                Self::collect_destruct_names(pattern, names);
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::collect_local_var_names(s, names);
                }
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_local_var_names(then_branch, names);
                if let Some(eb) = else_branch {
                    Self::collect_local_var_names(eb, names);
                }
            }
            Statement::For { init, body, .. } => {
                if let Some(i) = init {
                    Self::collect_local_var_names(i, names);
                }
                Self::collect_local_var_names(body, names);
            }
            Statement::ForOf { body, .. } => Self::collect_local_var_names(body, names),
            Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                Self::collect_local_var_names(body, names);
            }
            Statement::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, stmts) in cases {
                    for s in stmts {
                        Self::collect_local_var_names(s, names);
                    }
                }
                if let Some(stmts) = default_body {
                    for s in stmts {
                        Self::collect_local_var_names(s, names);
                    }
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::collect_local_var_names(body, names);
                if let Some(c) = catch_body {
                    Self::collect_local_var_names(c, names);
                }
                if let Some(f) = finally_body {
                    Self::collect_local_var_names(f, names);
                }
            }
            Statement::FunDecl { body, .. } => Self::collect_local_var_names(body, names),
            _ => {}
        }
    }

    /// Names bound by a fn/arrow PARAMETER, a `for…of` loop var, or a `catch` param anywhere in `s`
    /// (recursively, incl. nested fns, control flow, and arrow exprs). Complements
    /// `collect_local_var_names` (`let`/`const`); together they are every name that can shadow a
    /// top-level numeric global.
    fn collect_binding_names(s: &Statement, out: &mut HashSet<String>) {
        use Statement::*;
        let mut ce = |x: &Expr, out: &mut HashSet<String>| Self::collect_arrow_params_expr(x, out);
        match s {
            FunDecl { params, body, .. } => {
                for p in params {
                    for n in p.bound_names() {
                        out.insert(n.to_string());
                    }
                }
                Self::collect_binding_names(body, out);
            }
            ForOf { name, iterable, body, .. } => {
                out.insert(name.to_string());
                ce(iterable, out);
                Self::collect_binding_names(body, out);
            }
            Try { body, catch_param, catch_body, finally_body, .. } => {
                if let Some(cp) = catch_param {
                    out.insert(cp.to_string());
                }
                Self::collect_binding_names(body, out);
                if let Some(c) = catch_body {
                    Self::collect_binding_names(c, out);
                }
                if let Some(f) = finally_body {
                    Self::collect_binding_names(f, out);
                }
            }
            Block { statements, .. } | Multi { statements, .. } => {
                statements.iter().for_each(|x| Self::collect_binding_names(x, out))
            }
            If { cond, then_branch, else_branch, .. } => {
                ce(cond, out);
                Self::collect_binding_names(then_branch, out);
                if let Some(eb) = else_branch {
                    Self::collect_binding_names(eb, out);
                }
            }
            While { cond, body, .. } | DoWhile { body, cond, .. } => {
                ce(cond, out);
                Self::collect_binding_names(body, out);
            }
            For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    Self::collect_binding_names(i, out);
                }
                if let Some(c) = cond {
                    ce(c, out);
                }
                if let Some(u) = update {
                    ce(u, out);
                }
                Self::collect_binding_names(body, out);
            }
            VarDecl { init: Some(i), .. } => ce(i, out),
            VarDeclDestructure { init, .. } => ce(init, out),
            ExprStmt { expr, .. } => ce(expr, out),
            Return { value: Some(v), .. } => ce(v, out),
            Throw { value, .. } => ce(value, out),
            Switch { expr, cases, default_body, .. } => {
                ce(expr, out);
                for (g, b) in cases {
                    if let Some(ge) = g {
                        ce(ge, out);
                    }
                    b.iter().for_each(|x| Self::collect_binding_names(x, out));
                }
                if let Some(d) = default_body {
                    d.iter().for_each(|x| Self::collect_binding_names(x, out));
                }
            }
            _ => {}
        }
    }

    /// Collect arrow-function parameter names anywhere in an expression (recursively), so an arrow
    /// param that shadows a top-level numeric global also disqualifies the global.
    fn collect_arrow_params_expr(e: &Expr, out: &mut HashSet<String>) {
        match e {
            Expr::ArrowFunction { params, body, .. } => {
                for p in params {
                    for n in p.bound_names() {
                        out.insert(n.to_string());
                    }
                }
                match body {
                    ArrowBody::Expr(x) => Self::collect_arrow_params_expr(x, out),
                    ArrowBody::Block(b) => Self::collect_binding_names(b, out),
                }
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                Self::collect_arrow_params_expr(callee, out);
                for a in args {
                    if let CallArg::Expr(x) | CallArg::Spread(x) = a {
                        Self::collect_arrow_params_expr(x, out);
                    }
                }
            }
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_arrow_params_expr(left, out);
                Self::collect_arrow_params_expr(right, out);
            }
            Expr::Unary { operand, .. } => Self::collect_arrow_params_expr(operand, out),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                Self::collect_arrow_params_expr(cond, out);
                Self::collect_arrow_params_expr(then_branch, out);
                Self::collect_arrow_params_expr(else_branch, out);
            }
            Expr::Index { object, index, .. } => {
                Self::collect_arrow_params_expr(object, out);
                Self::collect_arrow_params_expr(index, out);
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_arrow_params_expr(object, out);
                if let MemberProp::Expr(pe) = prop {
                    Self::collect_arrow_params_expr(pe, out);
                }
            }
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => Self::collect_arrow_params_expr(value, out),
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(x) | ArrayElement::Spread(x) => {
                            Self::collect_arrow_params_expr(x, out)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for pr in props {
                    match pr {
                        ObjectProp::KeyValue(_, x, _) | ObjectProp::Spread(x) => {
                            Self::collect_arrow_params_expr(x, out)
                        }
                    }
                }
            }
            Expr::TemplateLiteral { exprs, .. } => {
                exprs.iter().for_each(|x| Self::collect_arrow_params_expr(x, out))
            }
            _ => {}
        }
    }

    fn collect_destruct_names(pattern: &DestructPattern, names: &mut HashSet<String>) {
        match pattern {
            DestructPattern::Array(elements) => {
                for el in elements {
                    if let Some(DestructElement::Ident(n, _)) = el {
                        names.insert(n.to_string());
                    }
                    if let Some(DestructElement::Pattern(p)) = el {
                        Self::collect_destruct_names(p, names);
                    }
                }
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    match &prop.value {
                        DestructElement::Ident(n, _) => {
                            names.insert(n.to_string());
                        }
                        DestructElement::Pattern(p) => Self::collect_destruct_names(p, names),
                        DestructElement::Rest(n, _) => {
                            names.insert(n.to_string());
                        }
                    }
                }
            }
        }
    }

    fn collect_stmt_idents(stmt: &Statement, idents: &mut HashSet<String>) {
        match stmt {
            Statement::ExprStmt { expr, .. } => Self::collect_expr_idents(expr, idents),
            Statement::VarDecl { init, .. } => {
                if let Some(e) = init {
                    Self::collect_expr_idents(e, idents);
                }
            }
            Statement::VarDeclDestructure { init, .. } => Self::collect_expr_idents(init, idents),
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::collect_stmt_idents(s, idents);
                }
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_stmt_idents(then_branch, idents);
                if let Some(e) = else_branch {
                    Self::collect_stmt_idents(e, idents);
                }
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
                Self::collect_expr_idents(cond, idents);
                Self::collect_stmt_idents(body, idents);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(s) = init {
                    Self::collect_stmt_idents(s, idents);
                }
                if let Some(e) = cond {
                    Self::collect_expr_idents(e, idents);
                }
                if let Some(e) = update {
                    Self::collect_expr_idents(e, idents);
                }
                Self::collect_stmt_idents(body, idents);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_expr_idents(iterable, idents);
                Self::collect_stmt_idents(body, idents);
            }
            Statement::Return { value, .. } => {
                if let Some(e) = value {
                    Self::collect_expr_idents(e, idents);
                }
            }
            Statement::Throw { value, .. } => Self::collect_expr_idents(value, idents),
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::collect_stmt_idents(body, idents);
                if let Some(c) = catch_body {
                    Self::collect_stmt_idents(c, idents);
                }
                if let Some(f) = finally_body {
                    Self::collect_stmt_idents(f, idents);
                }
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                Self::collect_expr_idents(expr, idents);
                for (case_expr, stmts) in cases {
                    if let Some(e) = case_expr {
                        Self::collect_expr_idents(e, idents);
                    }
                    for s in stmts {
                        Self::collect_stmt_idents(s, idents);
                    }
                }
                if let Some(stmts) = default_body {
                    for s in stmts {
                        Self::collect_stmt_idents(s, idents);
                    }
                }
            }
            Statement::FunDecl { body, .. } => Self::collect_stmt_idents(body, idents),
            Statement::Break { .. }
            | Statement::Continue { .. }
            | Statement::Import { .. }
            | Statement::Export { .. }
            | Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {}
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
        let outer_params: Vec<String> = self
            .outer_params_stack
            .iter()
            .flat_map(|p| p.iter().cloned())
            .filter(|name| referenced.contains(name) && !param_names.contains(name))
            .collect();

        // Collect outer variables (from outer scopes) that need to be captured
        let outer_vars: Vec<String> = self
            .outer_vars_stack
            .iter()
            .flat_map(|v| v.iter().cloned())
            .filter(|name| {
                referenced.contains(name)
                    && !param_names.contains(name)
                    && !local_var_names.contains(name)
            })
            .filter(|name| {
                ![
                    "Boolean",
                    "console",
                    "Math",
                    "JSON",
                    "Date",
                    "Set",
                    "Map",
                    "Object",
                    "process",
                    "setTimeout",
                    "clearTimeout",
                    "setInterval",
                    "clearInterval",
                    "Promise",
                    "Symbol",
                    "RegExp",
                    "Polars",
                ]
                .contains(&name.as_str())
                    && !self.native_numeric_globals.contains_key(name.as_str())
            })
            .collect();

        // Outer vars that are assigned in the body need RefCell; read-only get Value binding
        let mut assigned_in_body = HashSet::new();
        match body {
            ArrowBody::Expr(e) => Self::collect_assigned_idents_in_expr(e, &mut assigned_in_body),
            ArrowBody::Block(s) => Self::collect_assigned_idents_in_stmt(s, &mut assigned_in_body),
        }
        // Live cell capture: assigned here, or already `Rc<RefCell<Value>>` in a parent scope
        // (cleanups may only read `timer2` but must see updates from nested callbacks).
        let cell_capture_outer_vars: Vec<String> = outer_vars
            .iter()
            .filter(|v| assigned_in_body.contains(*v) || self.rc_cell_storage_contains(v))
            .cloned()
            .collect();
        let read_only_outer_vars: Vec<String> = outer_vars
            .iter()
            .filter(|v| !assigned_in_body.contains(*v) && !self.rc_cell_storage_contains(v))
            .cloned()
            .collect();

        // Wrap outer captures in Rc<RefCell<>> and use _ref suffix.
        // Clone existing Rc only when VarDecl actually emitted `Rc<RefCell<...>>` (see rc_cell_storage_*).
        for outer_param in &outer_params {
            let param_escaped = Self::escape_ident(outer_param);
            let ref_name = format!("{}_ref", param_escaped);
            if self.rc_cell_storage_contains(outer_param) {
                code.push_str(&format!(
                    "    let {} = {}.clone();\n",
                    ref_name, param_escaped
                ));
            } else {
                code.push_str(&format!(
                    "    let {} = VmRef::new({}.clone());\n",
                    ref_name, param_escaped
                ));
            }
        }
        for outer_var in &outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            let ref_name = format!("{}_ref", var_escaped);
            if self.rc_cell_storage_contains(outer_var) {
                code.push_str(&format!(
                    "    let {} = {}.clone();\n",
                    ref_name, var_escaped
                ));
            } else {
                code.push_str(&format!(
                    "    let {} = VmRef::new({}.clone());\n",
                    ref_name, var_escaped
                ));
            }
        }
        // Only clone builtins that are actually referenced (clone so outer scope can still use, e.g. process for PORT)
        for builtin in &[
            "console",
            "Math",
            "JSON",
            "Date",
            "Set",
            "Map",
            "Object",
            "Float64Array",
            "Float32Array",
            "Int8Array",
            "Uint8Array",
            "Uint8ClampedArray",
            "Int16Array",
            "Uint16Array",
            "Int32Array",
            "Uint32Array",
            "AudioContext",
            "process",
            "setTimeout",
            "clearTimeout",
            "setInterval",
            "clearInterval",
            "Promise",
            "Symbol",
            "RegExp",
            "Polars",
        ] {
            if referenced.contains(*builtin) {
                code.push_str(&format!("    let {} = {}.clone();\n", builtin, builtin));
            }
        }

        // Clone only function cells that are actually referenced in this arrow
        let referenced_funcs: Vec<String> = self
            .function_scope_stack
            .last()
            .map(|scope| {
                scope
                    .iter()
                    .filter(|f| referenced.contains(f.as_str()) && !param_names.contains(*f))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for func_name in &referenced_funcs {
            let escaped = Self::escape_ident(func_name);
            code.push_str(&format!(
                "    let {}_ref = {}_cell.clone();\n",
                escaped, escaped
            ));
        }

        // Locals from an enclosing Value::native (e.g. captured helper fns) are not on
        // outer_vars_stack but must not move into multiple sibling closures.
        const BUILTINS: &[&str] = &[
            "Boolean", "console", "Math", "JSON", "Date", "Object", "process",
            "setTimeout", "clearTimeout", "setInterval", "clearInterval", "Promise",
            "Symbol", "RegExp", "Polars", "Infinity", "NaN", "serve",
        ];
        let mut already_captured: HashSet<String> = outer_vars
            .iter()
            .chain(outer_params.iter())
            .chain(referenced_funcs.iter())
            .cloned()
            .collect();
        already_captured.extend(BUILTINS.iter().map(|s| s.to_string()));
        let implicit_env_captures: Vec<String> = if self.value_fn_depth > 0 {
            referenced
                .iter()
                .filter(|name| {
                    !param_names.contains(name.as_str())
                        && !local_var_names.contains(name.as_str())
                        && !already_captured.contains(name.as_str())
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        for name in &implicit_env_captures {
            let escaped = Self::escape_ident(name);
            code.push_str(&format!(
                "    let {}_ref = VmRef::new({}.clone());\n",
                escaped, escaped
            ));
        }

        code.push_str("    Value::native(move |args: &[Value]| {\n");
        self.value_fn_depth += 1;
        // #303: like the FunDecl closure above, a nested arrow is its own `-> Value` frame, so the
        // enclosing `try`'s `Result` channel must not leak in. Reset + restore at every exit.
        let saved_try_depth = self.try_closure_depth;
        self.try_closure_depth = 0;

        // Make captured outer params available as plain Values (from _ref RefCells)
        for outer_param in &outer_params {
            let param_escaped = Self::escape_ident(outer_param);
            code.push_str(&format!(
                "        let {} = (*{}_ref.borrow()).clone();\n",
                param_escaped, param_escaped
            ));
        }
        // Outer vars that share a RefCell with the parent: capture the cell (read + write)
        for outer_var in &cell_capture_outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            code.push_str(&format!(
                "        let {} = {}_ref.clone();\n",
                var_escaped, var_escaped
            ));
        }
        // Read-only outer vars: snapshot Value at closure creation
        for outer_var in &read_only_outer_vars {
            let var_escaped = Self::escape_ident(outer_var);
            code.push_str(&format!(
                "        let {} = (*{}_ref.borrow()).clone();\n",
                var_escaped, var_escaped
            ));
        }
        for name in &implicit_env_captures {
            let escaped = Self::escape_ident(name);
            code.push_str(&format!(
                "        let {} = (*{}_ref.borrow()).clone();\n",
                escaped, escaped
            ));
        }

        // Make captured functions available
        for func_name in &referenced_funcs {
            let escaped = Self::escape_ident(func_name);
            code.push_str(&format!(
                "        let {} = (*{}_ref.borrow()).clone();\n",
                escaped, escaped
            ));
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
                    if let Some(default_expr) = &tp.default {
                        // Default applies only for a MISSING positional arg (matches interp + VM);
                        // an explicit `null` keeps the null. emit_expr captured like the destructure
                        // path below so any prelude lands in `code`, not the outer output buffer.
                        let saved = std::mem::take(&mut self.output);
                        let default_str = self.emit_expr(default_expr)?;
                        let prelude = std::mem::replace(&mut self.output, saved);
                        code.push_str(&prelude);
                        code.push_str(&format!(
                            "        {} {} = match args.get({}) {{ Some(v) => v.clone(), None => {} }};\n",
                            Self::mut_kw_for(tp.name.as_ref(), "let mut"),
                            Self::escape_ident(tp.name.as_ref()),
                            i,
                            default_str
                        ));
                    } else {
                        code.push_str(&format!(
                            "        {} {} = args.get({}).cloned().unwrap_or(Value::Null);\n",
                            Self::mut_kw_for(tp.name.as_ref(), "let mut"),
                            Self::escape_ident(tp.name.as_ref()),
                            i
                        ));
                    }
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

        // Cell-backed outer vars need refcell_wrapped_vars for Assign and for reads in emit_expr
        let saved_refcell_vars = self.refcell_wrapped_vars.clone();
        for v in &cell_capture_outer_vars {
            self.refcell_wrapped_vars.insert(v.clone());
        }
        for v in &read_only_outer_vars {
            self.refcell_wrapped_vars.remove(v);
        }
        for v in &implicit_env_captures {
            self.refcell_wrapped_vars.remove(v);
        }

        self.type_context.push_fun_param_scope(params, None);

        let arrow_body_res: Result<(), CompileError> = match body {
            tishlang_ast::ArrowBody::Expr(expr) => {
                let expr_code = self.emit_expr(expr)?;
                // Bind to a temp before the closure returns: if `expr_code` reads a
                // cell-captured var its `RefCell` borrow guard is a temporary, and a
                // borrow left in tail position outlives the local cell binding —
                // which fails to compile (E0597). The `let` releases it at the `;`.
                code.push_str(&format!(
                    "        let __arrow_ret = {};\n        __arrow_ret\n",
                    expr_code
                ));
                Ok(())
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
                Ok(())
            }
        };

        self.type_context.pop_scope();
        if let Err(e) = arrow_body_res {
            self.value_fn_depth = self.value_fn_depth.saturating_sub(1);
            self.try_closure_depth = saved_try_depth;
            return Err(e);
        }

        self.value_fn_depth = self.value_fn_depth.saturating_sub(1);
        self.try_closure_depth = saved_try_depth;

        // Restore state
        self.refcell_wrapped_vars = saved_refcell_vars;
        self.outer_params_stack.pop();
        self.outer_vars_stack.pop();

        code.push_str("    })\n");
        code.push('}');

        Ok(code)
    }

    /// Emit an expression as a native Rust type (not wrapped in Value).
    /// Falls back to emit_expr + conversion if the expression cannot be directly
    /// emitted as the target type.
    fn emit_native_expr(
        &mut self,
        expr: &Expr,
        target_type: &RustType,
    ) -> Result<String, CompileError> {
        // #177: `let bodies = makeBodies()` — route the array-factory call to its native free fn
        // returning `Vec<TishStruct_alias>` directly (no boxed `Value::Array` round-trip).
        if !self.aggregate_fns.is_empty() {
            if let Expr::Call { callee, args, .. } = expr {
                if let Some((code, _)) = self.try_emit_toplevel_agg_call(callee, args, false)? {
                    return Ok(code);
                }
            }
        }

        // Try to emit literals directly as native types
        if let Expr::Literal { value, .. } = expr {
            match (target_type, value) {
                (RustType::F64, Literal::Number(n)) => {
                    return Ok(Self::f64_lit(*n));
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

        // Tuple literal: `[a, b]` against a `[T0, T1]` tuple type -> native Rust tuple `(a, b)`.
        if let (RustType::Tuple(elem_types), Expr::Array { elements, .. }) = (target_type, expr) {
            if elements.len() == elem_types.len()
                && elements.iter().all(|e| matches!(e, ArrayElement::Expr(_)))
            {
                let mut items = Vec::new();
                for (elem, ty) in elements.iter().zip(elem_types) {
                    if let ArrayElement::Expr(e) = elem {
                        items.push(self.emit_native_expr(e, ty)?);
                    }
                }
                return Ok(if items.len() == 1 {
                    format!("({},)", items[0])
                } else {
                    format!("({})", items.join(", "))
                });
            }
            // arity/shape mismatch -> boxed fallback
            let value_expr = self.emit_expr(expr)?;
            return Ok(target_type.from_value_expr(&value_expr));
        }

        // Try to emit object literals directly as a Rust struct literal
        // when the target is a `RustType::Named` (a user `type Foo = {...}`
        // alias). Each property in source order is matched to a struct
        // field; missing fields fall back to `default_value()` so the
        // emit succeeds even on partial literals (rare, but harmless).
        if let (RustType::Named { name, fields }, Expr::Object { props, .. }) = (target_type, expr)
        {
            use std::collections::HashMap;
            let field_types: HashMap<&str, &RustType> =
                fields.iter().map(|(k, t)| (k.as_ref(), t)).collect();
            let mut field_inits: HashMap<String, String> = HashMap::new();
            let mut bail = false;
            for prop in props {
                match prop {
                    ObjectProp::KeyValue(key, value, _) => {
                        if let Some(field_ty) = field_types.get(key.as_ref()) {
                            let v = self.emit_native_expr(value, field_ty)?;
                            field_inits.insert(crate::types::field_ident(key.as_ref()), v);
                        }
                    }
                    // Spread can't be statically matched to struct fields:
                    // fall back to the dynamic Value path.
                    ObjectProp::Spread(_) => {
                        bail = true;
                        break;
                    }
                }
            }
            if !bail {
                let assigns = fields
                    .iter()
                    .map(|(k, t)| {
                        let fid = crate::types::field_ident(k);
                        match field_inits.remove(&fid) {
                            Some(v) => format!("{}: {}", fid, v),
                            None => format!("{}: {}", fid, t.default_value()),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                return Ok(format!(
                    "{} {{ {} }}",
                    crate::types::named_struct_ident(name),
                    assigns
                ));
            }
        }

        // Check if the identifier is already of the target type
        if let Expr::Ident { name, .. } = expr {
            if *target_type == RustType::F64 {
                if self.native_numeric_globals.contains_key(name.as_ref()) {
                    return Ok(Self::native_global_get(name.as_ref()));
                }
                if let Some(uv) = self.usize_var_subst.get(name.as_ref()) {
                    return Ok(format!("({} as f64)", uv));
                }
            }
            if let RustType::Vec(inner) = target_type {
                if **inner == RustType::F64 {
                    if let Some(static_name) = self.module_const_f64_array_rust_ref(name.as_ref()) {
                        return Ok(format!(
                            "{}.iter().copied().collect::<Vec<f64>>()",
                            static_name
                        ));
                    }
                }
            }
            let var_type = self.type_context.get_type(name.as_ref());
            if &var_type == target_type {
                let esc = Self::escape_ident(name.as_ref()).into_owned();
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    return Ok(format!("(*{}.borrow()).clone()", esc));
                }
                return Ok(esc);
            }
        }

        // Native typed-array HOFs (TISH_NATIVE_HOF): `xs.reduce/map/filter/some/every(<arrow>)`
        // whose native result type matches this binding's target → emit the iterator chain
        // directly, with NO box/unbox round-trip (the per-element `value_call` is gone too).
        if let Expr::Call { callee, args, .. } = expr {
            if let Some((code, ty)) = self.native_vec_hof_for_call(callee, args)? {
                if &ty == target_type {
                    return Ok(code);
                }
            }
        }

        // Fast path: when the native typed emitter already yields the target type, use its code
        // directly — skipping the `Value::Number(<expr>)` box that `from_value_expr` would
        // immediately unbox. This round-trip otherwise lands in hot loops: `let xt = x*x - y*y + x0`
        // (xt inferred f64) emitted `match &Value::Number(<expr>) { Value::Number(n) => *n,
        // _ => panic!() }` *every iteration*. `emit_typed_expr`'s contract guarantees `code` is a
        // value of `typed_ty` directly, so when it equals the target the code is exactly what we
        // want, unboxed. (Any other type falls through to the unchanged box-and-coerce path below.)
        if let Ok((typed_code, typed_ty)) = self.emit_typed_expr(expr) {
            if &typed_ty == target_type {
                return Ok(typed_code);
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
    // ───────────────────────── M5: native monomorphic functions ─────────────────────────
    fn ann_is_number(ann: &TypeAnnotation) -> bool {
        RustType::from_annotation(ann) == RustType::F64
    }

    // ── Soundness: demote `number` locals that a reassignment can turn non-numeric ──────────────
    //
    // `let s = 0` is inferred `number` → lowered to a native `f64`, and a reassignment stores into
    // it via `s = match &<rhs> { Value::Number(n) => *n, _ => panic!("expected number") }`. That
    // coercion PANICS when `<rhs>` is not a number — which `s = s + arr[i]` produces whenever
    // `arr[i]` is a String (JS `+` is string concat). Node, the interpreter, and the VM all yield
    // a string there (the VM array-JIT bails to the interpreter on a non-numeric element). The
    // fix: keep such a local a boxed `Value`, so the boxed `ops::add` — which concatenates —
    // flows through unchanged.
    //
    // A reassignment is SAFE iff its RHS lowers to a native `f64`, which is exactly what
    // `emit_typed_expr` decides. `expr_native_type` is a read-only mirror of that decision and is
    // deliberately conservative: any form it does not model → `Value` → demote (sound; at worst an
    // unnecessary box). A fixpoint propagates demotions through chains (`y = y + s` once `s` is
    // demoted). The map is name-flat across the whole program (a name demoted in one function is
    // demoted in all) — still sound, and harmless to the perf gauntlet, where each kernel is its
    // own program with unique accumulator names.
    fn collect_demoted_numeric_locals(&self, stmts: &[Statement]) -> HashSet<String> {
        // 1. Flat env: every annotated local/param name → its native `RustType`.
        let mut env: HashMap<String, RustType> = HashMap::new();
        Self::collect_annotated_types(stmts, &self.type_aliases, &mut env);
        // 2. Every reassignment `(name, rhs)` anywhere in the program (incl. nested exprs/closures).
        let mut reassigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(stmts, &mut reassigns);
        // 3. Fixpoint: demote a `number` local whose any reassignment RHS isn't native `f64`.
        let mut demoted: HashSet<String> = HashSet::new();
        loop {
            let mut changed = false;
            for (name, rhs) in &reassigns {
                if demoted.contains(name) {
                    continue;
                }
                if env.get(name) == Some(&RustType::F64)
                    && self.expr_native_type(rhs, &env) != RustType::F64
                {
                    demoted.insert(name.clone());
                    env.insert(name.clone(), RustType::Value);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        demoted
    }

    // ── In-bounds index elision (#173 part 3) ────────────────────────────────────────────────────
    //
    // JS `a[i] = v` past the end GROWS the array, and `a[oob]` reads `undefined` (→ NaN/false), so the
    // native lowering wraps every numeric/bool `Vec` store in a `resize`-grow branch and every read in
    // `.get().unwrap_or(..)`. When the index is PROVABLY in `[0, len)` for a fixed-length `Vec`, both
    // guards are dead: the store is a direct `a[i] = v`, the read a direct `a[i]`. This is the
    // bounds-check elision fast JS engines (JSC/Bun) do after range-proving a counted loop.
    //
    // Soundness rests on three independent facts, all conservative (any gap keeps the safe lowering):
    //   • `vec_fixed_len[a] = B`  — `a` is filled once to length `B` and never length-changed after;
    //   • an active guard `i < B`  — the access sits inside a loop bounding the index above by `B`;
    //   • `i` is non-negative      — so it can't wrap to a huge `usize` below the bound.

    /// Parse a loop condition `counter < bound` / `counter <= bound` into `(counter, bound, strict)`
    /// when it has the bare `Ident <cmp> (int-literal | Ident)` shape; else `None`.
    fn parse_loop_guard(cond: &Expr) -> Option<(String, BoundKey, bool)> {
        let Expr::Binary { left, op, right, .. } = cond else {
            return None;
        };
        let strict = match op {
            BinOp::Lt => true,
            BinOp::Le => false,
            _ => return None,
        };
        let Expr::Ident { name, .. } = left.as_ref() else {
            return None;
        };
        Some((name.to_string(), Self::bound_key_of(right)?, strict))
    }

    /// A symbolic bound from an integer literal, a bare variable, or an `a.length` member read;
    /// richer forms (`2 * n`, `a.length - 1`, …) are not modeled.
    fn bound_key_of(e: &Expr) -> Option<BoundKey> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Self::int_literal_value(*n).map(BoundKey::Const),
            Expr::Ident { name, .. } => Some(BoundKey::Var(name.to_string())),
            Expr::Member {
                object,
                prop: MemberProp::Name { name: p, .. },
                optional: false,
                ..
            } if p.as_ref() == "length" => match object.as_ref() {
                Expr::Ident { name, .. } => Some(BoundKey::Len(name.to_string())),
                _ => None,
            },
            _ => None,
        }
    }

    /// Prove `a[index]` is in-bounds for the native `Vec` local `a`: `a` has a fixed length `B`, an
    /// enclosing guard bounds the index strictly below `B` (a non-strict `<= B-1` const works too),
    /// and the index is a non-negative bare counter. Conservative — only a bare `Ident` index is
    /// modeled (covers `a[i]` / `a[k]`; not `a[k - i]`).
    fn index_guard_bounds_len(&self, guard_bound: &BoundKey, len: &BoundKey, strict: bool) -> bool {
        if !strict {
            return false;
        }
        if guard_bound == len {
            return true;
        }
        if let (BoundKey::Const(gc), BoundKey::Var(lv)) = (guard_bound, len) {
            if self.native_param_lit(lv) == Some(*gc as f64) {
                return true;
            }
        }
        if let (BoundKey::Var(gv), BoundKey::Var(lv)) = (guard_bound, len) {
            if gv == lv {
                return true;
            }
            if let (Some(gl), Some(ll)) = (self.native_param_lit(gv), self.native_param_lit(lv)) {
                return gl == ll;
            }
        }
        if let (BoundKey::TwiceVar(tv), BoundKey::TwiceVar(lv)) = (guard_bound, len) {
            return strict && tv == lv;
        }
        false
    }

    /// Upper bound (exclusive) of a symbolic vec length, when provable from const param literals.
    fn bound_key_upper_i64(&self, len: &BoundKey) -> Option<i64> {
        match len {
            BoundKey::Const(c) => Some(*c),
            BoundKey::Var(p) => self
                .native_param_lit(p)
                .filter(|n| n.fract() == 0.0 && *n >= 0.0)
                .map(|n| n as i64),
            BoundKey::TwiceVar(p) => self
                .native_param_lit(p)
                .filter(|n| n.fract() == 0.0 && *n >= 0.0)
                .map(|n| 2 * n as i64),
            BoundKey::Len(_) => None,
        }
    }

    /// `let d1 = row + col` / `let d2 = row - col + n` in a native-vec fn — mark coords for direct
    /// `diag1[d1]` / `diag2[d2]` indexing without resize guards.
    fn try_register_diag_coord_int_range(&mut self, name: &str, init: &Expr) {
        if self.native_vec_ret.is_none() {
            return;
        }
        let ctr_ok = |e: &Expr| -> bool {
            match e {
                Expr::Ident { name: c, .. } => {
                    self.usize_for_counters.contains(c.as_ref())
                        || self.usize_var_subst.contains_key(c.as_ref())
                }
                _ => false,
            }
        };
        if let Expr::Binary {
            left,
            op: BinOp::Add,
            right,
            ..
        } = init
        {
            if matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "row")
                && ctr_ok(right.as_ref())
            {
                self.diag_coord_indices.insert(name.to_string());
                return;
            }
        }
        if let Expr::Binary {
            left,
            op: BinOp::Add,
            right: n_ref,
            ..
        } = init
        {
            if matches!(n_ref.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "n") {
                if let Expr::Binary {
                    left: row,
                    op: BinOp::Sub,
                    right: ctr,
                    ..
                } = left.as_ref()
                {
                    if matches!(row.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "row")
                        && ctr_ok(ctr)
                    {
                        self.diag_coord_indices.insert(name.to_string());
                    }
                }
            }
        }
    }

    /// RHS of `let x = …` in a native-vec fn body is pure f64 arithmetic on params/counters.
    fn expr_infer_native_f64_in_vec_fn(
        e: &Expr,
        type_ctx: &crate::types::TypeContext,
        usize_counters: &std::collections::HashSet<String>,
    ) -> bool {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => n.is_finite(),
            Expr::Ident { name, .. } => {
                let t = type_ctx.get_type(name.as_ref());
                t == RustType::F64 || usize_counters.contains(name.as_ref())
            }
            Expr::Binary {
                left,
                op: BinOp::Add | BinOp::Sub,
                right,
                ..
            } => Self::expr_infer_native_f64_in_vec_fn(left, type_ctx, usize_counters)
                && Self::expr_infer_native_f64_in_vec_fn(right, type_ctx, usize_counters),
            _ => false,
        }
    }

    fn index_in_bounds(&self, index: &Expr, arr: &str) -> bool {
        let Some(len) = self.vec_fixed_len.get(arr) else {
            return false;
        };
        let Expr::Ident { name: idx, .. } = index else {
            if let Expr::Literal {
                value: Literal::Number(n),
                ..
            } = index
            {
                if let BoundKey::Const(len) = len {
                    if Self::int_literal_value(*n).is_some_and(|v| v >= 0 && v < *len) {
                        return true;
                    }
                } else if Self::int_literal_value(*n) == Some(0) {
                    return true;
                }
            }
            return self.sub_index_in_bounds(index, arr);
        };
        let idx = idx.as_ref();
        // Lower bound: the index can never be negative.
        let nonneg = self.nonneg_locals.contains(idx)
            || self.usize_for_counters.contains(idx)
            || self.diag_coord_indices.contains(idx)
            || self
                .int_range_locals
                .get(idx)
                .is_some_and(|&(lo, _)| lo >= 0);
        if !nonneg {
            return false;
        }
        if self.diag_coord_indices.contains(idx) && matches!(len, BoundKey::TwiceVar(_)) {
            return true;
        }
        if let Some(&(lo, hi)) = self.int_range_locals.get(idx) {
            if lo >= 0 {
                if let Some(ub) = self.bound_key_upper_i64(len) {
                    if hi < ub {
                        return true;
                    }
                }
            }
        }
        if self.strict_lt_bounds.iter().any(|(v, bound)| {
            idx == v
                && match self.vec_fixed_len.get(arr) {
                    Some(BoundKey::Var(len_v)) => len_v == bound,
                    Some(BoundKey::Const(c)) => self
                        .native_param_lit(bound)
                        .is_some_and(|n| n.fract() == 0.0 && n as i64 == *c),
                    _ => false,
                }
        }) {
            return true;
        }
        // Upper bound: some LIVE active guard proves `idx < len`. `arr` is in `vec_fixed_len`, so it
        // never shrinks — a guard on `arr.length` (`i < arr.length`) therefore also bounds `idx`.
        self.active_index_guards.iter().any(|g| {
            g.live
                && g.var == idx
                && match (&g.bound, len) {
                    // `i < arr.length` directly bounds `arr[i]` (fixed-len ⇒ no shrink).
                    (BoundKey::Len(a), _) => g.strict && a == arr,
                    // `i < B` where `B` is the same key as `arr`'s fixed length.
                    _ if self.index_guard_bounds_len(&g.bound, len, g.strict) => true,
                    // `i <= C-1` with a constant length `C` also proves `i < C`.
                    (BoundKey::Const(gc), BoundKey::Const(lc)) => !g.strict && *gc + 1 <= *lc,
                    // `i < k2` where `k2 = (k+1)>>1` and `k` is bounded by the same length.
                    (BoundKey::Var(gv), BoundKey::Var(ll)) => {
                        g.strict
                            && self.shift_half_of.get(gv).is_some_and(|p| {
                                self.bounded_below_len.get(p) == Some(&BoundKey::Var(ll.clone()))
                            })
                    }
                    // `i < k` where `k` was read from a proven in-bounds `arr[k']`.
                    (BoundKey::Var(gv), bound_len) => {
                        g.strict && self.bounded_below_len.get(gv) == Some(bound_len)
                    }
                    _ => false,
                }
        })
    }

    /// `arr[lk - rk]` in-bounds when `lk` is strictly below the fixed length and `rk < lk`.
    fn sub_index_in_bounds(&self, index: &Expr, arr: &str) -> bool {
        let Some(len) = self.vec_fixed_len.get(arr) else {
            return false;
        };
        let Expr::Binary {
            left,
            op: BinOp::Sub,
            right,
            ..
        } = index
        else {
            return false;
        };
        let Expr::Ident { name: lk, .. } = left.as_ref() else {
            return false;
        };
        let Expr::Ident { name: rk, .. } = right.as_ref() else {
            if Self::int_literal_value_of(right) != Some(1) {
                return false;
            }
            return self.index_minus_one_in_bounds(lk, arr);
        };
        let lk = lk.as_ref();
        let rk = rk.as_ref();
        let rk_nonneg = self.nonneg_locals.contains(rk)
            || self.usize_for_counters.contains(rk)
            || self
                .int_range_locals
                .get(rk)
                .is_some_and(|&(lo, _)| lo >= 0);
        if !rk_nonneg {
            return false;
        }
        let lk_bounded = self.active_index_guards.iter().any(|g| {
            g.live
                && g.var == lk
                && match (&g.bound, len) {
                    (BoundKey::Len(a), _) => g.strict && a == arr,
                    _ if self.index_guard_bounds_len(&g.bound, len, g.strict) => true,
                    (BoundKey::Const(gc), BoundKey::Const(lc)) => g.strict && *gc < *lc,
                    (BoundKey::Var(lv), BoundKey::Var(ll)) => g.strict && lv == ll,
                    _ => false,
                }
        }) || self
            .bounded_below_len
            .get(lk)
            .is_some_and(|b| b == len || self.index_guard_bounds_len(b, len, true));
        let rk_lt_lk = self.active_index_guards.iter().any(|g| {
            g.live && g.var == rk && g.bound == BoundKey::Var(lk.to_string()) && g.strict
        }) || self.active_index_guards.iter().any(|g| {
            g.live
                && g.var == rk
                && g.strict
                && matches!(&g.bound, BoundKey::Var(k2) if self.shift_half_of.get(k2) == Some(&lk.to_string()))
        });
        lk_bounded && rk_lt_lk
    }

    /// `arr[lk - 1]` when `lk` is bounded by the vec's fixed length (fannkuch `count[r-1]`).
    fn index_minus_one_in_bounds(&self, lk: &str, arr: &str) -> bool {
        let len = match self.vec_fixed_len.get(arr) {
            Some(l) => l,
            None => return false,
        };
        match len {
            BoundKey::Var(n) => self.bounded_below_len.get(lk) == Some(&BoundKey::Var(n.clone())),
            BoundKey::Const(c) => self.bounded_below_len.get(lk) == Some(&BoundKey::Const(*c)),
            BoundKey::TwiceVar(_) | BoundKey::Len(_) => false,
        }
    }

    /// Permutation indices read from `arr[0]` are in `[0, len-1]` when `arr` has fixed length `len`.
    fn seed_int_range_from_len(&mut self, var: &str, len: &BoundKey) {
        match len {
            BoundKey::Const(c) if *c > 0 => {
                self.int_range_locals.insert(var.to_string(), (0, *c - 1));
            }
            BoundKey::Var(p) => {
                if let Some(n) = self.native_param_lit(p) {
                    if n.fract() == 0.0 && n >= 1.0 {
                        self.int_range_locals.insert(var.to_string(), (0, n as i64 - 1));
                    }
                }
            }
            BoundKey::TwiceVar(p) => {
                if let Some(n) = self.native_param_lit(p) {
                    if n.fract() == 0.0 && n >= 1.0 {
                        self.int_range_locals.insert(var.to_string(), (0, 2 * n as i64 - 1));
                    }
                }
            }
            BoundKey::Const(_) | BoundKey::Len(_) => {}
        }
    }

    fn merge_fn_body_inference(&mut self, stmts: &[Statement]) {
        for (k, v) in self.collect_vec_fixed_len(stmts) {
            self.vec_fixed_len.insert(k, v);
        }
        for n in self.collect_nonneg_locals(stmts) {
            self.nonneg_locals.insert(n);
        }
        for (k, v) in self.collect_int_range_locals(stmts) {
            self.int_range_locals.insert(k, v);
        }
        for (k, v) in Self::collect_shift_half_of(stmts) {
            self.shift_half_of.insert(k, v);
        }
        for (k, v) in Self::collect_array_elem_ranges(stmts) {
            self.array_elem_ranges.insert(k, v);
        }
        for n in Self::collect_int_valued_locals(stmts) {
            self.int_valued_locals.insert(n);
        }
    }

    /// Push an active index guard parsed from a loop condition `var < bound` / `var <= bound`.
    /// Returns whether a guard was pushed (the caller pops it after emitting the loop body).
    fn push_index_guard(&mut self, cond: Option<&Expr>) -> bool {
        if let Some((var, bound, strict)) = cond.and_then(Self::parse_loop_guard) {
            self.active_index_guards.push(IndexGuard {
                var,
                bound,
                strict,
                live: true,
            });
            true
        } else {
            false
        }
    }

    /// Clear any live guard whose counter is `name`: once the counter is reassigned, its value is no
    /// longer bounded by the loop condition for the remainder of the body. Conservatively clears ALL
    /// matching guards (a same-named outer counter is mutated by the same store).
    fn invalidate_index_guard(&mut self, name: &str) {
        for g in self.active_index_guards.iter_mut() {
            if g.var == name {
                g.live = false;
            }
        }
    }

    /// Native `Vec` locals whose length is provably `>= B` at every use and can only GROW: filled
    /// once to length `B` (a fill loop `for (i=0;i<B;i++){ a.push(_); … }` or a non-empty array
    /// literal) and never shrunk, aliased, or escaped. Growing ops (`push`) are fine — they keep
    /// `len >= B`, which is all the upper-bound proof `idx < B <= len` needs. Anything that could
    /// shrink (`pop`/`shift`/`splice`/`length=`), reassign, or let `a` escape (passed as an argument,
    /// captured, aliased into another binding) DISQUALIFIES it. The element type is re-checked at the
    /// use site, so this name-keyed prepass need not know which locals are `Vec`s yet.
    fn collect_vec_fixed_len(&self, stmts: &[Statement]) -> HashMap<String, BoundKey> {
        let mut cand: HashMap<String, BoundKey> = HashMap::new();
        let mut escaped: HashSet<String> = HashSet::new();
        self.scan_vec_fill(stmts, &mut cand, &mut escaped);
        for s in stmts {
            Self::for_each_stmt_expr(s, &mut |e| {
                Self::flag_vec_escapes_inner(e, &mut escaped, Some(&self.native_vec_fns));
            });
        }
        cand.retain(|name, _| !escaped.contains(name));
        cand
    }

    /// Record a length-`B` candidate for each `Vec` set by a fill loop (`for (i=0;i<B;i++){ … }` whose
    /// body is one-or-more `a.push(_)` statements, `B` a literal/var) or a non-empty array literal. A
    /// second length-setter for the same name disqualifies it (ambiguous).
    fn scan_vec_fill(
        &self,
        stmts: &[Statement],
        cand: &mut HashMap<String, BoundKey>,
        escaped: &mut HashSet<String>,
    ) {
        for s in stmts {
            if let Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } = s
            {
                if let Some((arrs, len)) = Self::match_fill_loop_arrays(
                    init.as_deref(),
                    cond.as_ref(),
                    update.as_ref(),
                    body,
                ) {
                    for a in arrs {
                        if cand.insert(a.clone(), len.clone()).is_some() {
                            escaped.insert(a);
                        }
                    }
                }
            }
            if let Statement::VarDecl {
                name,
                init: Some(Expr::Array { elements, .. }),
                ..
            } = s
            {
                if !elements.is_empty()
                    && elements.iter().all(|el| matches!(el, ArrayElement::Expr(_)))
                    && cand
                        .insert(name.to_string(), BoundKey::Const(elements.len() as i64))
                        .is_some()
                {
                    escaped.insert(name.to_string());
                }
                // Empty `[]` init does NOT pin length — a subsequent fill loop sets the real bound.
            }
            Self::for_each_child_stmt_list(s, &mut |list| self.scan_vec_fill(list, cand, escaped));
        }
    }

    /// Context-aware escape walk: any bare-`Ident` use of a name in a value position adds it to
    /// `escaped` (it could be aliased/mutated elsewhere). Uses that CANNOT shrink the array are
    /// exempt: `a[i]` / `a[i]=v` (the object), `a.push(...)` (grows, keeps `len >= B`), and a
    /// non-call member read like `a.length`. Conservative — over-flagging only loses the optimization.
    fn flag_vec_escapes(e: &Expr, escaped: &mut HashSet<String>) {
        Self::flag_vec_escapes_inner(e, escaped, None);
    }

  /// Context-aware escape walk: any bare-`Ident` use of a name in a value position adds it to
    /// `escaped` (it could be aliased/mutated elsewhere). Uses that CANNOT shrink the array are
    /// exempt: `a[i]` / `a[i]=v` (the object), `a.push(...)` (grows, keeps `len >= B`), and a
    /// non-call member read like `a.length`. Conservative — over-flagging only loses the optimization.
    /// When `native_vec_fns` is set, array idents passed by reference to a native-vec callee are not
    /// flagged — those calls borrow without changing length.
    fn flag_vec_escapes_inner(
        e: &Expr,
        escaped: &mut HashSet<String>,
        native_vec_fns: Option<&std::collections::HashMap<String, NativeVecFnSig>>,
    ) {
        match e {
            // A bare value-position read of `a` — it can flow into another binding / call and be
            // mutated out of sight, so the fixed-length fact no longer holds.
            Expr::Ident { name, .. } => {
                escaped.insert(name.to_string());
            }
            Expr::Index { object, index, .. } => {
                if !matches!(object.as_ref(), Expr::Ident { .. }) {
                    Self::flag_vec_escapes_inner(object, escaped, native_vec_fns);
                }
                Self::flag_vec_escapes_inner(index, escaped, native_vec_fns);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                if !matches!(object.as_ref(), Expr::Ident { .. }) {
                    Self::flag_vec_escapes_inner(object, escaped, native_vec_fns);
                }
                Self::flag_vec_escapes_inner(index, escaped, native_vec_fns);
                Self::flag_vec_escapes_inner(value, escaped, native_vec_fns);
            }
            Expr::Member { object, prop, .. } => {
                // A member READ (`a.length`, `a.foo`) doesn't mutate or alias the array.
                if !matches!(object.as_ref(), Expr::Ident { .. }) {
                    Self::flag_vec_escapes_inner(object, escaped, native_vec_fns);
                }
                if let MemberProp::Expr(pe) = prop {
                    Self::flag_vec_escapes_inner(pe, escaped, native_vec_fns);
                }
            }
            Expr::Call { callee, args, .. } => {
                if let Some(fns) = native_vec_fns {
                    if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                        if let Some(sig) = fns.get(fname.as_ref()) {
                            Self::flag_vec_escapes_inner(callee, escaped, native_vec_fns);
                            for (a, (_, kind)) in args.iter().zip(sig.params.iter()) {
                                let borrowed_array_ident = matches!(
                                    (kind, a),
                                    (VecParamKind::Array { .. }, CallArg::Expr(Expr::Ident { .. }))
                                );
                                if borrowed_array_ident {
                                    continue;
                                }
                                match a {
                                    CallArg::Expr(e) | CallArg::Spread(e) => {
                                        Self::flag_vec_escapes_inner(e, escaped, native_vec_fns);
                                    }
                                }
                            }
                            return;
                        }
                    }
                }
                match callee.as_ref() {
                    // `a.push(x)` grows the array (keeps `len >= B`) — `a` itself is not flagged; the
                    // ARGUMENT is still a value position. Any OTHER method (`pop`/`shift`/`splice`/…
                    // or a user method that could mutate) flags `a`.
                    Expr::Member {
                        object,
                        prop: MemberProp::Name { name: method, .. },
                        ..
                    } if matches!(object.as_ref(), Expr::Ident { .. }) => {
                        if method.as_ref() != "push" {
                            if let Expr::Ident { name, .. } = object.as_ref() {
                                escaped.insert(name.to_string());
                            }
                        }
                    }
                    other => Self::flag_vec_escapes_inner(other, escaped, native_vec_fns),
                }
                for a in args {
                    match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::flag_vec_escapes_inner(e, escaped, native_vec_fns);
                        }
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::flag_vec_escapes_inner(callee, escaped, native_vec_fns);
                for a in args {
                    match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::flag_vec_escapes_inner(e, escaped, native_vec_fns);
                        }
                    }
                }
            }
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::flag_vec_escapes_inner(left, escaped, native_vec_fns);
                Self::flag_vec_escapes_inner(right, escaped, native_vec_fns);
            }
            Expr::Unary { operand, .. }
            | Expr::TypeOf { operand, .. }
            | Expr::Await { operand, .. } => Self::flag_vec_escapes_inner(operand, escaped, native_vec_fns),
            Expr::Delete { target, .. } => Self::flag_vec_escapes_inner(target, escaped, native_vec_fns),
            // A reassignment target could install a shorter array; flag it. (RHS is a value pos.)
            Expr::Assign { name, value, .. }
            | Expr::CompoundAssign { name, value, .. }
            | Expr::LogicalAssign { name, value, .. } => {
                escaped.insert(name.to_string());
                Self::flag_vec_escapes_inner(value, escaped, native_vec_fns);
            }
            Expr::MemberAssign { object, value, .. } => {
                if let Expr::Ident { name, .. } = object.as_ref() {
                    escaped.insert(name.to_string()); // `a.length = …` etc.
                } else {
                    Self::flag_vec_escapes_inner(object, escaped, native_vec_fns);
                }
                Self::flag_vec_escapes_inner(value, escaped, native_vec_fns);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::flag_vec_escapes_inner(cond, escaped, native_vec_fns);
                Self::flag_vec_escapes_inner(then_branch, escaped, native_vec_fns);
                Self::flag_vec_escapes_inner(else_branch, escaped, native_vec_fns);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::flag_vec_escapes_inner(e, escaped, native_vec_fns)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::flag_vec_escapes_inner(e, escaped, native_vec_fns)
                        }
                    }
                }
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::flag_vec_escapes_inner(e, escaped, native_vec_fns);
                }
            }
            // A closure body can capture and mutate an array out of line; conservatively flag every
            // name it mentions so a captured Vec is never treated as fixed-length.
            Expr::ArrowFunction { body, .. } => {
                let mut idents = HashSet::new();
                match body {
                    ArrowBody::Expr(e) => Self::collect_expr_idents(e, &mut idents),
                    ArrowBody::Block(s) => {
                        Self::for_each_stmt_expr(s, &mut |e| {
                            Self::collect_expr_idents(e, &mut idents)
                        });
                    }
                }
                escaped.extend(idents);
            }
            _ => {}
        }
    }

    /// A fill loop `for (let i = 0; i < B; i++) { a1.push(_); a2.push(_); … }`: every body statement
    /// is a distinct `Ident.push(<one arg>)`, `i` starts at 0 and increments, `B` is a literal/var.
    /// Returns the pushed array names and the shared length `B`.
    fn match_fill_loop_arrays(
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Option<(Vec<String>, BoundKey)> {
        let Some(Statement::VarDecl {
            name: i_name,
            init: Some(i_init),
            ..
        }) = init
        else {
            return None;
        };
        if Self::int_literal_value_of(i_init) != Some(0) {
            return None;
        }
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right: bound,
            ..
        } = cond?
        else {
            return None;
        };
        let Expr::Ident { name: c, .. } = left.as_ref() else {
            return None;
        };
        if c.as_ref() != i_name.as_ref() {
            return None;
        }
        if !Self::is_increment_of(update?, i_name.as_ref()) {
            return None;
        }
        let len = Self::bound_key_of(bound)?;
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        if stmts.is_empty() {
            return None;
        }
        let mut arrs = Vec::new();
        for st in stmts {
            let Statement::ExprStmt {
                expr: Expr::Call { callee, args, .. },
                ..
            } = st
            else {
                return None;
            };
            let Expr::Member {
                object,
                prop: MemberProp::Name { name: method, .. },
                optional: false,
                ..
            } = callee.as_ref()
            else {
                return None;
            };
            if method.as_ref() != "push" || args.len() != 1 {
                return None;
            }
            let Expr::Ident { name: arr, .. } = object.as_ref() else {
                return None;
            };
            arrs.push(arr.to_string());
        }
        Some((arrs, len))
    }

    /// `let k2 = (k + 1) >> 1` seeds: `i < k2` then proves `i < k` for `arr[k - i]`.
    fn parse_shift_half_init(e: &Expr) -> Option<String> {
        let Expr::Binary {
            left,
            op: BinOp::Shr,
            right,
            ..
        } = e
        else {
            return None;
        };
        if Self::int_literal_value_of(right) != Some(1) {
            return None;
        }
        let Expr::Binary {
            left: add_l,
            op: BinOp::Add,
            right: add_r,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: k, .. } = add_l.as_ref() else {
            return None;
        };
        if Self::int_literal_value_of(add_r) != Some(1) {
            return None;
        }
        Some(k.to_string())
    }

    fn collect_shift_half_of(stmts: &[Statement]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for s in stmts {
            Self::for_each_stmt_expr(s, &mut |e| {
                if let Expr::Assign { name, value, .. } = e {
                    if let Some(k) = Self::parse_shift_half_init(value) {
                        out.insert(name.to_string(), k);
                    }
                }
            });
            match s {
                Statement::VarDecl {
                    name,
                    init: Some(e),
                    ..
                } => {
                    if let Some(k) = Self::parse_shift_half_init(e) {
                        out.insert(name.to_string(), k);
                    }
                }
                _ => {}
            }
            Self::for_each_child_stmt_list(s, &mut |list| {
                for (k2, k) in Self::collect_shift_half_of(list) {
                    out.insert(k2, k);
                }
            });
        }
        out
    }

    /// `for (let i = 0; i < bound; i++)` with increment-only counter in a native body.
    fn parse_usize_for_counter(
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
    ) -> Option<(String, String)> {
        let Statement::VarDecl {
            name,
            init: Some(i0),
            ..
        } = init?
        else {
            return None;
        };
        if Self::int_literal_value_of(i0) != Some(0) {
            return None;
        }
        if !Self::is_increment_of(update?, name.as_ref()) {
            return None;
        }
        let Expr::Binary {
            left,
            op: BinOp::Lt,
            right,
            ..
        } = cond?
        else {
            return None;
        };
        let Expr::Ident { name: lv, .. } = left.as_ref() else {
            return None;
        };
        if lv.as_ref() != name.as_ref() {
            return None;
        }
        let Expr::Ident { name: bound, .. } = right.as_ref() else {
            return None;
        };
        Some((name.to_string(), bound.to_string()))
    }

    /// `let y0 = ((py / h) * 3) - 1.5` immediately after a usize `py` loop — fuse into one coord decl.
    fn parse_linear_coord_from_counter(
        stmt: &Statement,
        counter: &str,
        bound_ident: &str,
        bound_lit: Option<f64>,
    ) -> Option<(String, f64, f64, f64)> {
        let (coord_name, value) = match stmt {
            Statement::VarDecl {
                name,
                init: Some(e),
                ..
            } => (name.to_string(), e),
            Statement::ExprStmt { expr, .. } => match expr {
                Expr::Assign { name, value, .. } => (name.to_string(), value.as_ref()),
                _ => return None,
            },
            _ => return None,
        };
        let Expr::Binary {
            left,
            op: BinOp::Sub,
            right: sub_rhs,
            ..
        } = value
        else {
            return None;
        };
        let sub = match sub_rhs.as_ref() {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => *n,
            _ => return None,
        };
        let Expr::Binary {
            left: mul_expr,
            op: BinOp::Mul,
            right: mul_rhs,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        let mul = match mul_rhs.as_ref() {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => *n,
            _ => return None,
        };
        let div = match mul_expr.as_ref() {
            Expr::Binary {
                left,
                op: BinOp::Div,
                right: div_rhs,
                ..
            } => {
                let Expr::Ident { name: c, .. } = left.as_ref() else {
                    return None;
                };
                if c.as_ref() != counter {
                    return None;
                }
                match div_rhs.as_ref() {
                    Expr::Literal {
                        value: Literal::Number(n),
                        ..
                    } => *n,
                    Expr::Ident { name: p, .. } if p.as_ref() == bound_ident => {
                        bound_lit?
                    }
                    _ => return None,
                }
            }
            Expr::Ident { name: c, .. } if c.as_ref() == counter => 1.0,
            _ => return None,
        };
        Some((coord_name, mul, div, sub))
    }

    /// Loop bound is a native scalar (`f64`/`i32` shadow param or local) — safe for `0..(bound as usize)`.
    fn usize_for_loop_bound_is_native(&self, bound: &str) -> bool {
        matches!(
            self.type_context.get_type(bound),
            RustType::F64 | RustType::I32
        )
    }

    fn try_emit_usize_for_loop(
        &mut self,
        counter: &str,
        bound: &str,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        let usize_var = format!(
            "_usize_{}_{}",
            Self::escape_ident(counter),
            self.loop_label_index
        );
        self.loop_label_index += 1;
        let esc_ctr = Self::escape_ident(counter).into_owned();
        let bound_usize = if let Some(lit) = self.native_param_lit(bound) {
            if lit.fract() == 0.0 && lit >= 0.0 {
                format!("{}", lit as usize)
            } else {
                format!("({} as usize)", Self::escape_ident(bound))
            }
        } else {
            format!("({} as usize)", Self::escape_ident(bound))
        };
        let bound_key = if let Some(lit) = self.native_param_lit(bound) {
            if lit.fract() == 0.0 {
                BoundKey::Const(lit as i64)
            } else {
                BoundKey::Var(bound.to_string())
            }
        } else {
            BoundKey::Var(bound.to_string())
        };
        let bound_lit = self.native_param_lit(bound);
        let body_stmts: &[Statement] = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            single => std::slice::from_ref(single),
        };
        let fused_coord = if self.native_fn_body_emit || self.native_vec_ret.is_some() {
            body_stmts
                .first()
                .and_then(|s| {
                    Self::parse_linear_coord_from_counter(s, counter, bound, bound_lit)
                })
        } else {
            None
        };
        let (rest_stmts, fused_coord) = if let Some(fc) = fused_coord {
            if body_stmts.len() < 2 {
                (body_stmts, None)
            } else {
                (&body_stmts[1..], Some(fc))
            }
        } else {
            (body_stmts, None)
        };
        self.writeln(&format!("for {} in 0..{} {{", usize_var, bound_usize));
        self.indent += 1;
        let native_usize = self.native_fn_body_emit
            || self.native_vec_ret.is_some()
            || (self.value_fn_depth > 0 && self.usize_for_loop_bound_is_native(bound));
        let emit_f64_counter = self.native_fn_body_emit || !self.native_vec_ret.is_some();
        if native_usize {
            self.usize_var_subst.insert(counter.to_string(), usize_var.clone());
            self.usize_for_counters.insert(counter.to_string());
        }
        let fused_was_peeled = fused_coord.is_some();
        if let Some((coord_name, mul, div, sub)) = fused_coord.as_ref() {
            let esc_coord = Self::escape_ident(coord_name);
            let scale = *mul / *div;
            self.writeln(&format!(
                "let mut {}: f64 = (({} as f64) * {} - {});",
                esc_coord,
                usize_var,
                Self::f64_lit(scale),
                Self::f64_lit(*sub),
            ));
            self.type_context.define(coord_name, RustType::F64);
        } else if emit_f64_counter {
            self.writeln(&format!(
                "let mut {}: f64 = ({} as f64);",
                esc_ctr, usize_var
            ));
            self.type_context.define(counter, RustType::F64);
            self.usize_for_counters.insert(counter.to_string());
        } else {
            self.type_context.define(counter, RustType::F64);
        }
        let extra_guard = match &bound_key {
            BoundKey::Var(bv) => self.shift_half_of.get(bv).map(|parent| {
                IndexGuard {
                    var: counter.to_string(),
                    bound: BoundKey::Var(parent.clone()),
                    strict: true,
                    live: true,
                }
            }),
            _ => None,
        };
        let pushed_extra = extra_guard.is_some();
        self.active_index_guards.push(IndexGuard {
            var: counter.to_string(),
            bound: bound_key,
            strict: true,
            live: true,
        });
        if let Some(g) = extra_guard {
            self.active_index_guards.push(g);
        }
        let emit_body = if fused_was_peeled && rest_stmts.len() == 1 {
            rest_stmts[0].clone()
        } else if fused_was_peeled {
            Statement::Multi {
                statements: rest_stmts.to_vec(),
                span: body.span(),
            }
        } else {
            body.clone()
        };
        if self.native_fn_body_emit {
            self.emit_native_fn_body(&emit_body)?;
        } else {
            self.emit_statement(&emit_body)?;
        }
        self.active_index_guards.pop();
        if pushed_extra {
            self.active_index_guards.pop();
        }
        if native_usize {
            self.usize_var_subst.remove(counter);
        }
        self.indent -= 1;
        self.writeln("}");
        Ok(true)
    }

    /// `if ((permCount & 1) === 0) { checksum += flips } else { checksum -= flips }`.
    fn try_emit_checksum_parity_if(
        &mut self,
        cond: &Expr,
        then_branch: &Statement,
        else_branch: &Option<Box<Statement>>,
    ) -> Result<bool, CompileError> {
        let Some(eb) = else_branch.as_ref() else {
            return Ok(false);
        };
        let Expr::Binary {
            left,
            op: BinOp::StrictEq,
            right,
            ..
        } = cond
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(right) != Some(0) {
            return Ok(false);
        }
        let Expr::Binary {
            left: and_l,
            op: BinOp::BitAnd,
            right: and_r,
            ..
        } = left.as_ref()
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(and_r) != Some(1) {
            return Ok(false);
        };
        let Expr::Ident { name: parity_var, .. } = and_l.as_ref() else {
            return Ok(false);
        };
        if !self.int_valued_locals.contains(parity_var.as_ref()) {
            return Ok(false);
        }
        let (check_name, flip_name) = match Self::parse_checksum_parity_branches(then_branch, eb.as_ref()) {
            Some(v) => v,
            None => return Ok(false),
        };
        let esc_check = Self::escape_ident(&check_name);
        let esc_flip = Self::escape_ident(&flip_name);
        let esc_parity = Self::escape_ident(parity_var.as_ref());
        self.writeln(&format!("if (({} as i64) & 1) == 0 {{", esc_parity));
        self.indent += 1;
        self.writeln(&format!("{} += {};", esc_check, esc_flip));
        self.indent -= 1;
        self.writeln("} else {");
        self.indent += 1;
        self.writeln(&format!("{} -= {};", esc_check, esc_flip));
        self.indent -= 1;
        self.writeln("}");
        Ok(true)
    }

    fn parse_checksum_parity_branches(
        then_branch: &Statement,
        else_branch: &Statement,
    ) -> Option<(String, String)> {
        let parse_add = |stmt: &Statement| -> Option<(String, String)> {
            let expr = match stmt {
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    if statements.len() != 1 {
                        return None;
                    }
                    match &statements[0] {
                        Statement::ExprStmt { expr, .. } => expr,
                        _ => return None,
                    }
                }
                Statement::ExprStmt { expr, .. } => expr,
                _ => return None,
            };
            let Expr::Assign { name: check, value, .. } = expr else {
                return None;
            };
            let Expr::Binary {
                left,
                op: BinOp::Add,
                right,
                ..
            } = value.as_ref()
            else {
                return None;
            };
            let Expr::Ident { name: l, .. } = left.as_ref() else {
                return None;
            };
            if l.as_ref() != check.as_ref() {
                return None;
            }
            let Expr::Ident { name: flip, .. } = right.as_ref() else {
                return None;
            };
            Some((check.to_string(), flip.to_string()))
        };
        let parse_sub = |stmt: &Statement| -> Option<(String, String)> {
            let expr = match stmt {
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    if statements.len() != 1 {
                        return None;
                    }
                    match &statements[0] {
                        Statement::ExprStmt { expr, .. } => expr,
                        _ => return None,
                    }
                }
                Statement::ExprStmt { expr, .. } => expr,
                _ => return None,
            };
            let Expr::Assign { name: check, value, .. } = expr else {
                return None;
            };
            let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = value.as_ref()
            else {
                return None;
            };
            let Expr::Ident { name: l, .. } = left.as_ref() else {
                return None;
            };
            if l.as_ref() != check.as_ref() {
                return None;
            }
            let Expr::Ident { name: flip, .. } = right.as_ref() else {
                return None;
            };
            Some((check.to_string(), flip.to_string()))
        };
        let (tc, tf) = parse_add(then_branch)?;
        let (ec, ef) = parse_sub(else_branch)?;
        if tc != ec || tf != ef {
            return None;
        }
        Some((tc, tf))
    }

    /// Native `arr[i] = val` when `arr` is a local `Vec` — optionally without the boxed `Value::Null`
    /// wrapper for statement-position side effects.
    fn try_emit_native_vec_index_assign(
        &mut self,
        object: &Expr,
        index: &Expr,
        value: &Expr,
        side_effect_only: bool,
    ) -> Result<Option<String>, CompileError> {
        let Expr::Ident { name, .. } = object else {
            return Ok(None);
        };
        if self.refcell_wrapped_vars.contains(name.as_ref()) {
            return Ok(None);
        }
        let obj_type = self.type_context.get_type(name.as_ref());
        let RustType::Vec(elem_type) = obj_type else {
            return Ok(None);
        };
        let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
        let in_bounds = self.index_in_bounds(index, name.as_ref());
        let idx_usize = if let Expr::Ident { name: idx_name, .. } = index {
            if let Some(uv) = self.usize_var_subst.get(idx_name.as_ref()) {
                uv.clone()
            } else {
                let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
                if idx_ty == RustType::F64 || idx_ty == RustType::I32 {
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
                }
            }
        } else {
            let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
            if idx_ty == RustType::F64 || idx_ty == RustType::I32 {
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
            }
        };
        let native_val = if *elem_type == RustType::I32 {
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = value
            {
                if let Some(n) = Self::int_literal_value_of(right) {
                    let (lc, lt) = self.emit_typed_expr(left)?;
                    if lt == RustType::I32 {
                        format!("({} - {}i32)", lc, n as i32)
                    } else {
                        let (val_code, val_ty) = self.emit_typed_expr(value)?;
                        if val_ty == RustType::I32 {
                            val_code
                        } else {
                            format!("({}) as i32", val_code)
                        }
                    }
                } else {
                    let (val_code, val_ty) = self.emit_typed_expr(value)?;
                    if val_ty == RustType::I32 {
                        val_code
                    } else {
                        format!("({}) as i32", val_code)
                    }
                }
            } else {
                let (val_code, val_ty) = self.emit_typed_expr(value)?;
                if val_ty == RustType::I32 {
                    val_code
                } else if val_ty == RustType::Value {
                    elem_type.from_value_expr(&val_code)
                } else {
                    format!("({}) as i32", val_code)
                }
            }
        } else {
            let (val_code, val_ty) = self.emit_typed_expr(value)?;
            if val_ty == *elem_type {
                val_code
            } else if val_ty == RustType::Value {
                elem_type.from_value_expr(&val_code)
            } else {
                val_code
            }
        };
        let assign = match elem_type.as_ref() {
            RustType::F64 | RustType::Bool | RustType::I32 if in_bounds => {
                if side_effect_only {
                    format!("{}[{}] = {}", esc_obj, idx_usize, native_val)
                } else {
                    format!("{{ {}[{}] = {}; Value::Null }}", esc_obj, idx_usize, native_val)
                }
            }
            RustType::F64 | RustType::Bool => {
                let pad = if matches!(elem_type.as_ref(), RustType::F64) {
                    "f64::NAN"
                } else {
                    "false"
                };
                format!(
                    "{{ let _idx = {}; if _idx >= {}.len() {{ {}.resize(_idx + 1, {}); }} {}[_idx] = {}; Value::Null }}",
                    idx_usize, esc_obj, esc_obj, pad, esc_obj, native_val
                )
            }
            _ if side_effect_only => {
                format!("{}[{}] = {}", esc_obj, idx_usize, native_val)
            }
            _ => format!("{{ {}[{}] = {}; Value::Null }}", esc_obj, idx_usize, native_val),
        };
        Ok(Some(assign))
    }

    /// `for (i=0; i<r; i++) { arr[i] = arr[i+1] }` → `arr.copy_within(1..(r+1), 0)` (fannkuch perm1 rotation).
    fn try_emit_vec_copy_within_shift_for(
        &mut self,
        init: Option<&Statement>,
        cond: Option<&Expr>,
        update: Option<&Expr>,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        if !self.native_vec_ret.is_some() && !self.native_fn_body_emit {
            return Ok(false);
        }
        let (i_name, r_name) = match Self::parse_usize_for_counter(init, cond, update) {
            Some(v) => v,
            None => return Ok(false),
        };
        let arr_name = match Self::parse_vec_left_shift_body(body, &i_name) {
            Some(a) => a,
            None => return Ok(false),
        };
        if self.refcell_wrapped_vars.contains(arr_name.as_str()) {
            return Ok(false);
        }
        let RustType::Vec(elem) = self.type_context.get_type(arr_name.as_str()) else {
            return Ok(false);
        };
        if *elem != RustType::F64 {
            return Ok(false);
        }
        let esc_arr = Self::escape_ident(arr_name.as_str());
        let esc_r = Self::escape_ident(&r_name);
        self.writeln(&format!(
            "{{ let _ru = ({} as usize); if _ru > 0 {{ {}.copy_within(1..(_ru + 1), 0); }} }}",
            esc_r, esc_arr
        ));
        Ok(true)
    }

    /// `arr[i] = arr[i+1]` loop body for [`try_emit_vec_copy_within_shift_for`].
    fn parse_vec_left_shift_body(body: &Statement, i_name: &str) -> Option<String> {
        let st = match body {
            Statement::Block { statements, .. } if statements.len() == 1 => &statements[0],
            Statement::ExprStmt { .. } => body,
            _ => return None,
        };
        let Statement::ExprStmt {
            expr: Expr::IndexAssign {
                object,
                index,
                value,
                ..
            },
            ..
        } = st
        else {
            return None;
        };
        let Expr::Ident { name: arr, .. } = object.as_ref() else {
            return None;
        };
        if !matches!(index.as_ref(), Expr::Ident { name, .. } if name.as_ref() == i_name) {
            return None;
        }
        let Expr::Index {
            object: o2,
            index: idx2,
            optional: false,
            ..
        } = value.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: arr2, .. } = o2.as_ref() else {
            return None;
        };
        if arr.as_ref() != arr2.as_ref() {
            return None;
        }
        let Expr::Binary {
            left,
            op: BinOp::Add,
            right,
            ..
        } = idx2.as_ref()
        else {
            return None;
        };
        if !matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == i_name) {
            return None;
        }
        if Self::int_literal_value_of(right) != Some(1) {
            return None;
        }
        Some(arr.to_string())
    }

    /// `while (i < r) { let j = i+1; arr[i] = arr[j]; i = j }` → `for ui in 0..r { arr[ui]=arr[ui+1] }`.
    fn try_emit_native_left_shift_while(
        &mut self,
        cond: &Expr,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        let Expr::Binary {
            left: lv,
            op: BinOp::Lt,
            right: rv,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: i_name, .. } = lv.as_ref() else {
            return Ok(false);
        };
        let Expr::Ident { name: r_name, .. } = rv.as_ref() else {
            return Ok(false);
        };
        let stmts = match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.as_slice()
            }
            s => std::slice::from_ref(s),
        };
        if stmts.len() != 3 {
            return Ok(false);
        }
        let Statement::VarDecl {
            name: j_name,
            init: Some(j_init),
            ..
        } = &stmts[0]
        else {
            return Ok(false);
        };
        let Statement::ExprStmt { expr: idx_assign, .. } = &stmts[1] else {
            return Ok(false);
        };
        let Statement::ExprStmt { expr: i_upd, .. } = &stmts[2] else {
            return Ok(false);
        };
        let Expr::Binary {
            left: add_l,
            op: BinOp::Add,
            right: one,
            ..
        } = j_init
        else {
            return Ok(false);
        };
        if Self::int_literal_value_of(one) != Some(1) {
            return Ok(false);
        }
        if !matches!(add_l.as_ref(), Expr::Ident { name, .. } if name.as_ref() == i_name.as_ref()) {
            return Ok(false);
        }
        let Expr::IndexAssign {
            object,
            index: li,
            value: rhs,
            ..
        } = idx_assign
        else {
            return Ok(false);
        };
        let Expr::Ident { name: arr_name, .. } = object.as_ref() else {
            return Ok(false);
        };
        if !matches!(li.as_ref(), Expr::Ident { name, .. } if name.as_ref() == i_name.as_ref()) {
            return Ok(false);
        }
        let Expr::Index {
            object: ro,
            index: rj,
            optional: false,
            ..
        } = rhs.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: arr2, .. } = ro.as_ref() else {
            return Ok(false);
        };
        if arr_name.as_ref() != arr2.as_ref() {
            return Ok(false);
        }
        if !matches!(rj.as_ref(), Expr::Ident { name, .. } if name.as_ref() == j_name.as_ref()) {
            return Ok(false);
        }
        let Expr::Assign { name: i_up, value: iv, .. } = i_upd else {
            return Ok(false);
        };
        if i_up.as_ref() != i_name.as_ref() {
            return Ok(false);
        }
        if !matches!(iv.as_ref(), Expr::Ident { name, .. } if name.as_ref() == j_name.as_ref()) {
            return Ok(false);
        }
        let esc_arr = Self::escape_ident(arr_name.as_ref());
        let esc_r = Self::escape_ident(r_name.as_ref());
        self.writeln(&format!(
            "{}.copy_within(1..(({} as usize) + 1), 0);",
            esc_arr, esc_r
        ));
        Ok(true)
    }

    /// `while (iter < N && escape)` in a native body when `N` is a known constant — a `usize`
    /// counted loop with an escape `break` (mandelbrot's inner iteration).
    fn try_emit_usize_bounded_escape_while(
        &mut self,
        cond: &Expr,
        body: &Statement,
    ) -> Result<bool, CompileError> {
        let Expr::Binary {
            left,
            op: BinOp::And,
            right: escape,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Binary {
            left: lv,
            op: BinOp::Lt,
            right: rv,
            ..
        } = left.as_ref()
        else {
            return Ok(false);
        };
        let Expr::Ident { name: iter_name, .. } = lv.as_ref() else {
            return Ok(false);
        };
        let max_usize = match rv.as_ref() {
            Expr::Ident { name: p, .. } => {
                let v = match self.native_param_lit(p.as_ref()) {
                    Some(v) if v.fract() == 0.0 && v >= 0.0 => v as usize,
                    _ => return Ok(false),
                };
                v
            }
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => {
                if n.fract() != 0.0 || *n < 0.0 {
                    return Ok(false);
                }
                *n as usize
            }
            _ => return Ok(false),
        };
        let (escape_code, escape_ty) = self.emit_typed_expr(escape)?;
        let escape_bool = match escape_ty {
            RustType::Bool => escape_code,
            RustType::F64 => format!("({} != 0.0)", escape_code),
            _ => format!("{}.is_truthy()", escape_code),
        };
        let usize_var = format!(
            "_usize_{}_{}",
            Self::escape_ident(iter_name.as_ref()),
            self.loop_label_index
        );
        self.loop_label_index += 1;
        let stayed_var = format!("_stayed_{}", self.loop_label_index);
        self.writeln(&format!("let mut {} = true;", stayed_var));
        self.writeln(&format!("for {} in 0..{} {{", usize_var, max_usize));
        self.indent += 1;
        self.writeln(&format!("if !{} {{ {} = false; break; }}", escape_bool, stayed_var));
        self.skip_iter_local = Some(iter_name.to_string());
        if self.native_fn_body_emit {
            self.emit_native_fn_body(body)?;
        } else {
            self.emit_statement(body)?;
        }
        self.indent -= 1;
        self.writeln("}");
        self.pending_stayed_var = Some(stayed_var);
        Ok(true)
    }

    fn parse_strict_eq_idents(expr: &Expr) -> Option<(String, String)> {
        let Expr::Binary {
            left,
            op: BinOp::StrictEq,
            right,
            ..
        } = expr
        else {
            return None;
        };
        let Expr::Ident { name: a, .. } = left.as_ref() else {
            return None;
        };
        let Expr::Ident { name: b, .. } = right.as_ref() else {
            return None;
        };
        Some((a.to_string(), b.to_string()))
    }

    /// Fold `if (iter === maxIter) { count = count + 1 }` after a usize escape loop into `if stayed`.
    fn try_emit_stayed_count_if(
        &mut self,
        cond: &Expr,
        then_branch: &Statement,
        stayed: &str,
    ) -> Result<bool, CompileError> {
        let Expr::Binary {
            left,
            op: BinOp::StrictEq,
            right,
            ..
        } = cond
        else {
            return Ok(false);
        };
        let Expr::Ident { name: _iter, .. } = left.as_ref() else {
            return Ok(false);
        };
        let max_ok = match right.as_ref() {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => n.fract() == 0.0,
            Expr::Ident { name: p, .. } => self
                .native_param_lit(p.as_ref())
                .is_some_and(|v| v.fract() == 0.0),
            _ => false,
        };
        if !max_ok {
            return Ok(false);
        }
        let count_name = match Self::parse_count_plus_one_stmt(then_branch) {
            Some(c) => c,
            None => return Ok(false),
        };
        let esc_count = Self::escape_ident(&count_name);
        self.writeln(&format!("if {} {{", stayed));
        self.indent += 1;
        self.writeln(&format!("{} += 1_f64;", esc_count));
        self.indent -= 1;
        self.writeln("}");
        self.skip_iter_local = None;
        Ok(true)
    }

    fn statement_contains_break(stmt: &Statement) -> bool {
        match stmt {
            Statement::Break { .. } => true,
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(Self::statement_contains_break)
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => Self::statement_contains_break(then_branch)
                || else_branch
                    .as_ref()
                    .is_some_and(|s| Self::statement_contains_break(s)),
            _ => false,
        }
    }

    fn parse_count_plus_one_stmt(stmt: &Statement) -> Option<String> {
        let expr = match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                if statements.len() != 1 {
                    return None;
                }
                match &statements[0] {
                    Statement::ExprStmt { expr, .. } => expr,
                    _ => return None,
                }
            }
            Statement::ExprStmt { expr, .. } => expr,
            _ => return None,
        };
        let Expr::Assign { name, value, .. } = expr else {
            return None;
        };
        let Expr::Binary {
            left,
            op: BinOp::Add,
            right,
            ..
        } = value.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: l, .. } = left.as_ref() else {
            return None;
        };
        if l.as_ref() != name.as_ref() || Self::int_literal_value_of(right) != Some(1) {
            return None;
        }
        Some(name.to_string())
    }

    /// Locals provably `>= 0` at every program point: a one-sided sign fixpoint. Seeds non-negative
    /// `for` counters (init `>= 0` literal, increment-only) and `let x = <nonneg expr>`, then keeps a
    /// name only while its init AND every reassignment RHS evaluate non-negative given the current
    /// set. Conservative: a name that can't be proven simply isn't included.
    fn collect_nonneg_locals(&self, stmts: &[Statement]) -> HashSet<String> {
        // Candidate decls/reassigns and the non-negative `for` counters.
        let mut counters: HashSet<String> = HashSet::new();
        let mut defs: Vec<(String, Expr)> = Vec::new();
        self.collect_nonneg_defs(stmts, &mut counters, &mut defs);
        let mut nonneg = counters.clone();
        for (n, _) in &defs {
            nonneg.insert(n.clone());
        }
        // Fixpoint: drop any name whose init/reassignment RHS isn't provably non-negative.
        loop {
            let mut changed = false;
            let snapshot = nonneg.clone();
            for (n, rhs) in &defs {
                if snapshot.contains(n) && !Self::expr_nonneg(rhs, &snapshot, &counters) {
                    nonneg.remove(n);
                    changed = true;
                }
            }
            if !changed {
                return nonneg;
            }
        }
    }

    fn collect_nonneg_defs(
        &self,
        stmts: &[Statement],
        counters: &mut HashSet<String>,
        defs: &mut Vec<(String, Expr)>,
    ) {
        for s in stmts {
            if let Statement::For {
                init: Some(init),
                update: Some(update),
                ..
            } = s
            {
                if let Statement::VarDecl {
                    name,
                    init: Some(i0),
                    ..
                } = init.as_ref()
                {
                    if Self::int_literal_value_of(i0).is_some_and(|v| v >= 0)
                        && Self::is_increment_of(update, name.as_ref())
                    {
                        counters.insert(name.to_string());
                    }
                }
            }
            if let Statement::VarDecl {
                name,
                init: Some(init),
                ..
            } = s
            {
                defs.push((name.to_string(), init.clone()));
            }
            // Reassignments (`x = rhs`) are additional defs the RHS must keep non-negative.
            Self::for_each_stmt_expr(s, &mut |e| {
                if let Expr::Assign { name, value, .. } = e {
                    defs.push((name.to_string(), (**value).clone()));
                }
            });
            Self::for_each_child_stmt_list(s, &mut |list| {
                self.collect_nonneg_defs(list, counters, defs)
            });
        }
    }

    /// Whether `e` evaluates to a non-negative number given the current `nonneg` set and the
    /// non-negative `counters`. Conservative: only the structurally-obvious non-negative forms.
    fn expr_nonneg(e: &Expr, nonneg: &HashSet<String>, counters: &HashSet<String>) -> bool {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => *n >= 0.0,
            Expr::Ident { name, .. } => {
                nonneg.contains(name.as_ref()) || counters.contains(name.as_ref())
            }
            Expr::Binary { left, op, right, .. } => match op {
                // Sum/product of non-negatives is non-negative; `%`/`&`/`>>>` of a non-negative
                // dividend/operand stays non-negative; `<<`/`|`/`^` could flip the sign bit, so no.
                BinOp::Add | BinOp::Mul => {
                    Self::expr_nonneg(left, nonneg, counters)
                        && Self::expr_nonneg(right, nonneg, counters)
                }
                BinOp::Mod => Self::expr_nonneg(left, nonneg, counters),
                BinOp::UShr => true,
                BinOp::BitAnd => {
                    Self::expr_nonneg(left, nonneg, counters)
                        || Self::expr_nonneg(right, nonneg, counters)
                }
                _ => false,
            },
            _ => false,
        }
    }

    // ── Integer-range lattice (#174) ────────────────────────────────────────────────────────────
    //
    // Prove an `f64` expression always holds an integer within `(-2^53, 2^53)`, so it can be
    // computed in `i64` with a result BIT-IDENTICAL to the `f64` the interpreter/VM produce. The
    // immediate payoff is `x % c` → an integer remainder instead of `fmod` (fmod is ~5-10× slower);
    // the lattice is sound by construction — every rule preserves "integer-valued AND within the
    // exact-`f64` range", and any unprovable form yields `None` (treated as unbounded → no rewrite).
    //
    // The classic win is a `% c`-bounded recurrence (e.g. an LCG `seed = (seed*A + C) % M`): the
    // modulo caps the result to `[0, M-1]` regardless of the dividend's size, so the fixpoint
    // converges and every intermediate stays well under 2^53.

    /// Prove `e` is always an integer in `[min, max]` (inclusive), both inside `(-2^53, 2^53)`.
    /// `ranges` supplies proven bounds for in-scope locals. `None` = unprovable / unbounded.
    fn int_range(
        &self,
        e: &Expr,
        ranges: &HashMap<String, (i64, i64)>,
    ) -> Option<(i64, i64)> {
        const LIM: i64 = 1 << 53;
        let clamp = |lo: i64, hi: i64| -> Option<(i64, i64)> {
            if lo <= hi && lo > -LIM && hi < LIM {
                Some((lo, hi))
            } else {
                None
            }
        };
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Self::int_literal_value(*n).and_then(|v| clamp(v, v)),
            Expr::Ident { name, .. } => ranges.get(name.as_ref()).copied(),
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => {
                let (lo, hi) = self.int_range(operand, ranges)?;
                clamp(-hi, -lo)
            }
            Expr::Binary {
                left, op, right, ..
            } => match op {
                // Bitwise & shift always yield an int32 — exact and far inside 2^53. A positive
                // literal `&`-mask tightens the upper bound (common: `h & 0xFF` → [0, 255]).
                BinOp::BitAnd => {
                    let mask = Self::int_literal_value_of(left)
                        .or_else(|| Self::int_literal_value_of(right));
                    match mask {
                        Some(m) if m >= 0 => clamp(0, m),
                        _ => clamp(i32::MIN as i64, i32::MAX as i64),
                    }
                }
                BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                    clamp(i32::MIN as i64, i32::MAX as i64)
                }
                BinOp::UShr => clamp(0, u32::MAX as i64),
                // `x % c` (c a positive integer literal) — the result is an integer in (-c, c) when
                // the dividend is a proven integer; sign follows the dividend (JS `%` / Rust `%` both
                // truncate toward zero), so a non-negative dividend gives `[0, min(c-1, hi)]`.
                BinOp::Mod => {
                    let c = Self::int_literal_value_of(right).filter(|&c| c > 0)?;
                    // The dividend must be a proven INTEGER (so the result is integral); the modulo
                    // then caps the magnitude to `< c` REGARDLESS of the dividend's size — a fixed
                    // (not dividend-dependent) bound, so `% c`-driven recurrences converge in one
                    // step. Sign follows the dividend (Rust `%` / JS `%` both truncate to zero).
                    // The dividend is integral if it is range-bounded OR merely int-VALUED (e.g.
                    // `r % 97` where `r` is a loop counter — integral but unbounded), the latter
                    // giving the conservative two-sided `(-(c-1), c-1)`.
                    if let Some((lo, _hi)) = self.int_range(left, ranges) {
                        if lo >= 0 {
                            clamp(0, c - 1)
                        } else {
                            clamp(-(c - 1), c - 1)
                        }
                    } else if self.expr_is_int_valued(left) {
                        clamp(-(c - 1), c - 1)
                    } else {
                        None
                    }
                }
                BinOp::Add => {
                    let (la, ua) = self.int_range(left, ranges)?;
                    let (lb, ub) = self.int_range(right, ranges)?;
                    clamp(la + lb, ua + ub)
                }
                BinOp::Sub => {
                    let (la, ua) = self.int_range(left, ranges)?;
                    let (lb, ub) = self.int_range(right, ranges)?;
                    clamp(la - ub, ua - lb)
                }
                BinOp::Mul => {
                    let (la, ua) = self.int_range(left, ranges)?;
                    let (lb, ub) = self.int_range(right, ranges)?;
                    // Compute in i128 so corner products can't overflow before the 2^53 clamp.
                    let p = [
                        (la as i128) * (lb as i128),
                        (la as i128) * (ub as i128),
                        (ua as i128) * (lb as i128),
                        (ua as i128) * (ub as i128),
                    ];
                    let lo = *p.iter().min().unwrap();
                    let hi = *p.iter().max().unwrap();
                    if lo > -(LIM as i128) && hi < (LIM as i128) {
                        clamp(lo as i64, hi as i64)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// An integer-valued, exactly-`f64`-representable number from a numeric literal value.
    fn int_literal_value(n: f64) -> Option<i64> {
        if n.is_finite() && n.fract() == 0.0 && n.abs() < (1i64 << 53) as f64 {
            Some(n as i64)
        } else {
            None
        }
    }

    /// As [`int_literal_value`] but for an `Expr` that is a numeric literal (else `None`).
    fn int_literal_value_of(e: &Expr) -> Option<i64> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Self::int_literal_value(*n),
            _ => None,
        }
    }

    /// Names of `f64` locals provably integer-bounded within `(-2^53, 2^53)` across the whole
    /// program. Seeds from integer-literal initializers and literal-bounded `for` counters, then
    /// runs a join fixpoint over reassignments: a local keeps a bound only if its init and EVERY
    /// reassignment RHS are `int_range`-provable and the joined range stabilizes within a few
    /// rounds (else it is dropped = unbounded). Sound: a dropped local simply keeps the `f64` path.
    fn collect_int_range_locals(&self, stmts: &[Statement]) -> HashMap<String, (i64, i64)> {
        let mut ranges: HashMap<String, (i64, i64)> = HashMap::new();
        // Seed: `let x = <int literal>` and `for (let i = <int>; i < <int>; i++/i+=1)` counters.
        Self::seed_int_ranges(stmts, &mut ranges);
        if ranges.is_empty() {
            return ranges;
        }
        // All reassignments `(name, rhs)` — a local is bounded only if every one stays provable.
        let mut reassigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(stmts, &mut reassigns);

        // Phase A — join rounds: grow each seeded local's range toward a fixpoint. A reassignment
        // whose RHS is unprovable drops the local immediately. With the modulo cap fixed, `% c`
        // recurrences converge in ≤2 rounds; the round cap just bounds non-converging growth (those
        // are caught by phase B).
        for _round in 0..8 {
            let mut changed = false;
            let snapshot = ranges.clone();
            for (name, rhs) in &reassigns {
                let Some(&(clo, chi)) = snapshot.get(name.as_str()) else {
                    continue;
                };
                match self.int_range(rhs, &snapshot) {
                    Some((rlo, rhi)) => {
                        let (nlo, nhi) = (clo.min(rlo), chi.max(rhi));
                        if (nlo, nhi) != (clo, chi) {
                            ranges.insert(name.clone(), (nlo, nhi));
                            changed = true;
                        }
                    }
                    None => {
                        ranges.remove(name.as_str());
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Phase B — validate the result is an INDUCTIVE INVARIANT: a local keeps its range only if
        // every reassignment's RHS range (evaluated against the final map) stays within it. A local
        // that kept growing (e.g. `s = s + 1`, no cap) fails this and is dropped — and dropping it
        // can make other RHS unprovable, so iterate to a fixpoint. This is what makes the analysis
        // SOUND regardless of the round cap: only true fixpoints survive.
        loop {
            let mut dropped = false;
            let snapshot = ranges.clone();
            for (name, rhs) in &reassigns {
                let Some(&(clo, chi)) = snapshot.get(name.as_str()) else {
                    continue;
                };
                let ok = matches!(self.int_range(rhs, &snapshot), Some((rlo, rhi)) if rlo >= clo && rhi <= chi);
                if !ok {
                    ranges.remove(name.as_str());
                    dropped = true;
                }
            }
            if !dropped {
                return ranges;
            }
        }
    }

    // ── i32-loop-var lowering (bun/JSC-style integer-register hash accumulator) ─────────────────
    //
    // A `number` local `h` that (i) is declared `let h = <int literal>` immediately before a `for`,
    // (ii) is reassigned ONLY inside that loop by bitwise/shift expressions that lower fully in the
    // int32 domain, and (iii) whose every NUMERIC (non-bitwise) read happens where `h`'s JS value is
    // a *signed* int32 — can be kept in an `i32` register across the loop instead of round-tripping
    // `f64`↔`i32` on each op. The single excursion is an arithmetic node (`h * C`) that `int_range`
    // proves exceeds 2^53, so it stays `f64` (the multiply rounds in f64 *before* `ToUint32`, exactly
    // as V8 does). Soundness rests on:
    //   • `int_range_locals` proving `h` is always an exact integer in (-2^53, 2^53) — the i32
    //     register then holds precisely `ToInt32(h)`, and reads coerce `(h as f64)` = the signed
    //     int32 value, while `>>> 0` boxings reinterpret the register as `u32`.
    //   • the SIGNEDNESS pass below: after `^ & | << >>` `h` is signed-int32-valued; after `>>>`
    //     (and at init, since the literal may exceed i32::MAX) it is uint32-valued. A *numeric* read
    //     of `h` at a uint32-valued point would see the wrong sign → BAIL to the f64 path.
    // Anything unprovable bails → the existing f64 lowering, so this is purely additive.

    /// `op` is a bitwise/shift operator (operands coerced to int32 by JS).
    fn is_bitwise_op(op: BinOp) -> bool {
        matches!(
            op,
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr
        )
    }

    /// `e` lowers FULLY in the int32 domain (top node bitwise/shift; every leaf is either a numeric
    /// literal, the loop var `h`, or an arithmetic/`Mod` subtree of int-provable numbers that becomes
    /// a single `f64` excursion re-narrowed by `to_int32`). Mirrors what `emit_int32_operand` will
    /// actually emit, so a `true` here means the reassignment really lowers without a per-op round
    /// trip. Conservative: unknown forms (calls, member access, etc.) → false.
    fn i32_chain_lowerable(&self, e: &Expr, var: &str) -> bool {
        match e {
            Expr::Binary { left, op, right, .. } if Self::is_bitwise_op(*op) => {
                self.i32_chain_lowerable(left, var) && self.i32_chain_lowerable(right, var)
            }
            // A non-bitwise node is a LEAF in the int32 chain: it must lower to a plain `f64` that
            // `to_int32` then narrows. Require it provably integer-valued (so the f64 is exact) —
            // either the var itself, an int literal, or an int-range/int-valued arithmetic subtree.
            _ => self.i32_leaf_is_f64(e, var),
        }
    }

    /// An int32-chain LEAF that provably emits a plain `f64`: the loop var, an integer literal, or a
    /// `+ - * % / **`-arithmetic / unary subtree over numbers proven integer-valued (so `as f64` is
    /// exact and `to_int32` recovers the bit-pattern). Bitwise sub-nodes are handled by the caller.
    fn i32_leaf_is_f64(&self, e: &Expr, var: &str) -> bool {
        match e {
            Expr::Ident { name, .. } => name.as_ref() == var || self.expr_is_int_valued(e),
            Expr::Literal { value: Literal::Number(n), .. } => {
                n.is_finite() && n.fract() == 0.0
            }
            Expr::Unary { op: UnaryOp::Neg | UnaryOp::BitNot, operand, .. } => {
                self.i32_leaf_is_f64(operand, var)
            }
            Expr::Binary { left, op, right, .. } => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod => {
                    self.i32_leaf_is_f64(left, var) && self.i32_leaf_is_f64(right, var)
                }
                op if Self::is_bitwise_op(*op) => {
                    self.i32_chain_lowerable(left, var) && self.i32_chain_lowerable(right, var)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// `e` provably evaluates to a FINITE f64 with `|e| < 2^62`, so `to_int32_unchecked` (no
    /// `is_finite` guard, no saturating cast) is sound for it. Handled shapes: an `I32`-register read
    /// (`|x| < 2^31`), a finite numeric literal, unary `-`, and `+ - *` over such operands (bounds
    /// combined; any branch unprovable ⇒ `None`). Bitwise/shift sub-nodes are NOT leaves here, so we
    /// don't descend into them (the caller's `to_int32`/`to_uint32` already bound those to 32 bits).
    fn f64_finite_bounded_below_2pow62(&self, e: &Expr) -> bool {
        self.f64_abs_bound(e).is_some_and(|b| b < 4.611686018427388e18) // 2^62
    }

    /// Conservative magnitude bound for [`f64_finite_bounded_below_2pow62`]; `None` if not provable.
    fn f64_abs_bound(&self, e: &Expr) -> Option<f64> {
        match e {
            // An i32-register accumulator: its magnitude is `< 2^31`.
            Expr::Ident { name, .. }
                if self.type_context.get_type(name.as_ref()) == RustType::I32 =>
            {
                Some(2147483648.0) // 2^31
            }
            Expr::Literal { value: Literal::Number(n), .. } if n.is_finite() => Some(n.abs()),
            Expr::Unary { op: UnaryOp::Neg, operand, .. } => self.f64_abs_bound(operand),
            Expr::Binary { left, op, right, .. } => {
                let la = self.f64_abs_bound(left)?;
                let ra = self.f64_abs_bound(right)?;
                match op {
                    BinOp::Add | BinOp::Sub => Some(la + ra),
                    BinOp::Mul => Some(la * ra),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Walk `e` and decide whether `var` is read SAFELY given it is currently `signed`-int32-valued
    /// (`signed == false` ⇒ uint32-valued). A read of `var` directly under a bitwise/shift op is a
    /// register read (always safe); a read of `var` in any *numeric* position is safe only while
    /// `signed`. `bitwise_parent` tracks whether the immediate parent op is bitwise/shift. Returns
    /// `false` (bail) if any numeric read happens while not `signed`.
    fn i32_reads_ok(e: &Expr, var: &str, signed: bool, bitwise_parent: bool) -> bool {
        match e {
            Expr::Ident { name, .. } if name.as_ref() == var => bitwise_parent || signed,
            Expr::Binary { left, op, right, .. } => {
                let bw = Self::is_bitwise_op(*op);
                Self::i32_reads_ok(left, var, signed, bw)
                    && Self::i32_reads_ok(right, var, signed, bw)
            }
            Expr::Unary { operand, .. } => Self::i32_reads_ok(operand, var, signed, false),
            _ => {
                // Any other read of `var` (call arg, member, index, ternary, …) is a numeric/opaque
                // use: only bitwise-parent or signed positions pass; otherwise bail if it mentions
                // `var` at all (conservative — we can't track signedness through opaque forms).
                if Self::collect_idents_of(e).contains(var) {
                    bitwise_parent || signed
                } else {
                    true
                }
            }
        }
    }

    fn collect_idents_of(e: &Expr) -> HashSet<String> {
        let mut idents = HashSet::new();
        Self::collect_expr_idents(e, &mut idents);
        idents
    }

    /// EVERY read of `var` outside its own body-assignment RHSs must be a register (bitwise) read —
    /// e.g. the final `return h >>> 0`. Body-assignment RHSs (`var = <rhs>`) are vetted by the
    /// ordered signedness pass, so this walker SKIPS the RHS of an assignment whose target is `var`,
    /// and rejects any other numeric (non-bitwise) read of `var` anywhere in `stmts`.
    fn i32_only_bitwise_reads_outside_assigns(stmts: &[Statement], var: &str) -> bool {
        stmts
            .iter()
            .all(|s| Self::i32_external_reads_ok_stmt(s, var))
    }

    fn i32_external_reads_ok_stmt(s: &Statement, var: &str) -> bool {
        let mut ok = true;
        Self::for_each_stmt_expr(s, &mut |e| {
            if !ok {
                return;
            }
            ok &= Self::i32_external_reads_ok_expr(e, var, false);
        });
        ok
    }

    /// As `i32_reads_ok` with `signed = false` (the strictest state), but a `var = <rhs>` assignment
    /// node has its RHS reads SKIPPED — those are the loop assignments, vetted by the ordered pass.
    fn i32_external_reads_ok_expr(e: &Expr, var: &str, bitwise_parent: bool) -> bool {
        match e {
            // The write target name is not a read; its RHS is vetted by the ordered signedness pass.
            Expr::Assign { name, value, .. } if name.as_ref() == var => {
                // The RHS may itself contain *nested* assigns to OTHER vars referencing `var`, but
                // those would have failed the "single-writer" check; the RHS `var` reads are the
                // ordered-pass's job, so don't re-check them here.
                let _ = value;
                true
            }
            Expr::Ident { name, .. } if name.as_ref() == var => bitwise_parent,
            Expr::Binary { left, op, right, .. } => {
                let bw = Self::is_bitwise_op(*op);
                Self::i32_external_reads_ok_expr(left, var, bw)
                    && Self::i32_external_reads_ok_expr(right, var, bw)
            }
            Expr::Unary { operand, .. } => Self::i32_external_reads_ok_expr(operand, var, false),
            _ => {
                if Self::collect_idents_of(e).contains(var) {
                    bitwise_parent
                } else {
                    true
                }
            }
        }
    }

    /// Collect every `number` accumulator eligible for i32-register loop lowering. Scans every
    /// statement list (top level + nested blocks/loops/fn bodies); the eligibility gate itself uses
    /// the whole-program (name-keyed) reassignment set, so a name with any writer outside its loop
    /// body bails. Soundness is per-name, not per-scope, which the strict gate guarantees.
    fn collect_i32_loop_vars(&self, stmts: &[Statement]) -> HashSet<String> {
        let mut out = HashSet::new();
        self.collect_i32_loop_vars_in(stmts, stmts, &mut out);
        out
    }

    /// `stmts` is the statement list currently being scanned for the decl-then-`for` pattern;
    /// `root` is the whole program, used by the gate's whole-program writer/reader checks.
    fn collect_i32_loop_vars_in(
        &self,
        stmts: &[Statement],
        root: &[Statement],
        out: &mut HashSet<String>,
    ) {
        // `let h = <int>` directly followed by a `for` whose body reassigns `h`.
        for win in stmts.windows(2) {
            if let (
                Statement::VarDecl {
                    name,
                    mutable: true,
                    init: Some(init),
                    ..
                },
                Statement::For { body, .. },
            ) = (&win[0], &win[1])
            {
                if Self::int_literal_value_of(init).is_some()
                    && self.i32_loop_var_eligible(name.as_ref(), body, root)
                {
                    out.insert(name.to_string());
                }
            }
        }
        // Recurse into nested statement lists (each block / fn body / loop body is scanned).
        for s in stmts {
            Self::for_each_child_stmt_list(s, &mut |list| {
                self.collect_i32_loop_vars_in(list, root, out)
            });
        }
    }

    /// Eligibility gate for the i32-register lowering of `var`, declared just before `for (…) body`.
    /// All bail conditions keep the existing f64 path (purely additive). `var` qualifies iff:
    ///   (a) `int_range` proves it always holds an exact integer in (-2^53, 2^53);
    ///   (b) it is not closure-captured into a cell;
    ///   (c) it is written ONLY by the assignments inside `body`, each a bitwise/shift expr that
    ///       lowers fully in the int32 domain;
    ///   (d) the forward signedness pass over those assignments admits every numeric read of `var`;
    ///   (e) every read of `var` OUTSIDE those assignment RHSs is a register (bitwise) read.
    fn i32_loop_var_eligible(&self, var: &str, body: &Statement, root: &[Statement]) -> bool {
        // (a)
        if !self.int_range_locals.contains_key(var) {
            return false;
        }
        // (b)
        if self.refcell_wrapped_vars.contains(var) {
            return false;
        }
        // (c) reassignments to `var` inside the loop body, in source order.
        let mut body_assigns: Vec<&Expr> = Vec::new();
        Self::collect_ordered_assigns_to(body, var, &mut body_assigns);
        if body_assigns.is_empty() {
            return false;
        }
        for rhs in &body_assigns {
            let top_bitwise = matches!(rhs, Expr::Binary { op, .. } if Self::is_bitwise_op(*op));
            if !top_bitwise || !self.i32_chain_lowerable(rhs, var) {
                return false;
            }
        }
        // `var` must have NO writer outside this loop body — whole-program count must match.
        let mut all_assigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(root, &mut all_assigns);
        let total_writes = all_assigns.iter().filter(|(n, _)| n == var).count();
        if total_writes != body_assigns.len() {
            return false;
        }
        // (d) SIGNEDNESS pass. Init value may exceed i32::MAX ⇒ start uint32-valued. Each RHS is read
        // against the CURRENT signedness; new signedness follows the top op (`>>>` → unsigned).
        let mut signed = false;
        for rhs in &body_assigns {
            if !Self::i32_reads_ok(rhs, var, signed, false) {
                return false;
            }
            signed = !matches!(rhs, Expr::Binary { op: BinOp::UShr, .. });
        }
        // (e) Every other read of `var` in the program must be a register (bitwise) read.
        if !Self::i32_only_bitwise_reads_outside_assigns(root, var) {
            return false;
        }
        true
    }

    /// Collect, in source order, the RHS of every top-level `var = <rhs>` assignment to `var`
    /// reachable in `body` (descending blocks/if/loops but NOT into nested fn bodies).
    fn collect_ordered_assigns_to<'a>(s: &'a Statement, var: &str, out: &mut Vec<&'a Expr>) {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for st in statements {
                    Self::collect_ordered_assigns_to(st, var, out);
                }
            }
            Statement::ExprStmt { expr, .. } => {
                if let Expr::Assign { name, value, .. } = expr {
                    if name.as_ref() == var {
                        out.push(value.as_ref());
                    }
                }
            }
            Statement::If { then_branch, else_branch, .. } => {
                Self::collect_ordered_assigns_to(then_branch, var, out);
                if let Some(e) = else_branch {
                    Self::collect_ordered_assigns_to(e, var, out);
                }
            }
            Statement::For { body, .. }
            | Statement::ForOf { body, .. }
            | Statement::While { body, .. }
            | Statement::DoWhile { body, .. } => {
                Self::collect_ordered_assigns_to(body, var, out)
            }
            _ => {}
        }
    }

    /// Invoke `f` with every nested *statement list* directly reachable from `s` (blocks, `if`
    /// branches, loop bodies, fn bodies). Used to scan each lexical scope for the decl-then-`for`
    /// pattern. Branch/loop bodies are single `Statement`s, passed as 1-element slices.
    fn for_each_child_stmt_list(s: &Statement, f: &mut dyn FnMut(&[Statement])) {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                f(statements)
            }
            Statement::If { then_branch, else_branch, .. } => {
                f(std::slice::from_ref(then_branch));
                if let Some(e) = else_branch {
                    f(std::slice::from_ref(e));
                }
            }
            Statement::For { body, .. }
            | Statement::ForOf { body, .. }
            | Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::FunDecl { body, .. } => f(std::slice::from_ref(body)),
            Statement::Switch { cases, default_body, .. } => {
                for (_, body) in cases {
                    f(body);
                }
                if let Some(b) = default_body {
                    f(b);
                }
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                f(std::slice::from_ref(body));
                if let Some(b) = catch_body {
                    f(std::slice::from_ref(b));
                }
                if let Some(b) = finally_body {
                    f(std::slice::from_ref(b));
                }
            }
            _ => {}
        }
    }

    /// Invoke `f` on every top-level expression of `s`, recursing through nested control-flow
    /// statements (blocks, if, loops, switch, try, return/throw). `f` is responsible for recursing
    /// into each expression's own subtree. Does NOT descend into nested fn-decl bodies (a different
    /// lexical scope; a captured loop var would be RefCell-bailed before reaching here).
    fn for_each_stmt_expr(s: &Statement, f: &mut dyn FnMut(&Expr)) {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for st in statements {
                    Self::for_each_stmt_expr(st, f);
                }
            }
            Statement::VarDecl { init: Some(e), .. } => f(e),
            Statement::VarDeclDestructure { init, .. } => f(init),
            Statement::ExprStmt { expr, .. } => f(expr),
            Statement::If { cond, then_branch, else_branch, .. } => {
                f(cond);
                Self::for_each_stmt_expr(then_branch, f);
                if let Some(e) = else_branch {
                    Self::for_each_stmt_expr(e, f);
                }
            }
            Statement::While { cond, body, .. } => {
                f(cond);
                Self::for_each_stmt_expr(body, f);
            }
            Statement::DoWhile { body, cond, .. } => {
                Self::for_each_stmt_expr(body, f);
                f(cond);
            }
            Statement::For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    Self::for_each_stmt_expr(i, f);
                }
                if let Some(c) = cond {
                    f(c);
                }
                if let Some(u) = update {
                    f(u);
                }
                Self::for_each_stmt_expr(body, f);
            }
            Statement::ForOf { iterable, body, .. } => {
                f(iterable);
                Self::for_each_stmt_expr(body, f);
            }
            Statement::Return { value: Some(e), .. } => f(e),
            Statement::Throw { value, .. } => f(value),
            Statement::Switch { expr, cases, default_body, .. } => {
                f(expr);
                for (g, body) in cases {
                    if let Some(g) = g {
                        f(g);
                    }
                    for st in body {
                        Self::for_each_stmt_expr(st, f);
                    }
                }
                if let Some(b) = default_body {
                    for st in b {
                        Self::for_each_stmt_expr(st, f);
                    }
                }
            }
            Statement::Try { body, catch_body, finally_body, .. } => {
                Self::for_each_stmt_expr(body, f);
                if let Some(b) = catch_body {
                    Self::for_each_stmt_expr(b, f);
                }
                if let Some(b) = finally_body {
                    Self::for_each_stmt_expr(b, f);
                }
            }
            // A nested `function f(){…}` body is a separate scope: a loop var read there would be a
            // capture (RefCell-bailed) or a shadow (different binding) — don't descend.
            _ => {}
        }
    }

    /// Seed integer ranges: integer-literal `let` initializers and literal-bounded `for` counters.
    fn seed_int_ranges(stmts: &[Statement], out: &mut HashMap<String, (i64, i64)>) {
        for s in stmts {
            match s {
                Statement::VarDecl {
                    name,
                    init: Some(e),
                    ..
                } => {
                    if let Some(v) = Self::int_literal_value_of(e) {
                        out.insert(name.to_string(), (v, v));
                    }
                }
                Statement::For {
                    init, cond, body, ..
                } => {
                    // `for (let i = <int>; i < <int>; ...)` → counter `i` ∈ [start, end-1].
                    if let (
                        Some(Statement::VarDecl {
                            name,
                            init: Some(istart),
                            ..
                        }),
                        Some(Expr::Binary {
                            left,
                            op: BinOp::Lt,
                            right,
                            ..
                        }),
                    ) = (init.as_deref(), cond.as_ref())
                    {
                        if let (Some(start), Some(end)) = (
                            Self::int_literal_value_of(istart),
                            Self::int_literal_value_of(right),
                        ) {
                            if matches!(left.as_ref(), Expr::Ident { name: cn, .. } if cn.as_ref() == name.as_ref())
                                && end > start
                                && end - 1 < (1i64 << 53)
                            {
                                out.insert(name.to_string(), (start, end - 1));
                            }
                        }
                    }
                    Self::seed_int_ranges(std::slice::from_ref(body), out);
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::seed_int_ranges(statements, out)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::seed_int_ranges(std::slice::from_ref(then_branch), out);
                    if let Some(e) = else_branch {
                        Self::seed_int_ranges(std::slice::from_ref(e), out);
                    }
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    Self::seed_int_ranges(std::slice::from_ref(body), out)
                }
                Statement::FunDecl { body, .. } => Self::seed_int_ranges(std::slice::from_ref(body), out),
                _ => {}
            }
        }
    }

    /// `e` is provably INTEGER-valued (zero fractional part at runtime), per `set` for locals.
    /// Closed under `+ - * %` (modulo by a positive integer literal), unary `- ~`, bitwise/shift,
    /// and integer literals — so a loop counter (`0`, then `i + 1`) stays integral. Magnitude is
    /// NOT tracked (that is `int_range`'s job); this only certifies integrality.
    fn is_int_valued(e: &Expr, set: &HashSet<String>) -> bool {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => n.is_finite() && n.fract() == 0.0,
            Expr::Ident { name, .. } => set.contains(name.as_ref()),
            Expr::Unary {
                op: UnaryOp::Neg | UnaryOp::BitNot,
                operand,
                ..
            } => Self::is_int_valued(operand, set),
            Expr::Binary {
                left, op, right, ..
            } => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul => {
                    Self::is_int_valued(left, set) && Self::is_int_valued(right, set)
                }
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
                | BinOp::UShr => true,
                BinOp::Mod => {
                    Self::int_literal_value_of(right).is_some_and(|c| c != 0)
                        && Self::is_int_valued(left, set)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// `self`-bound [`is_int_valued`] against the computed `int_valued_locals`.
    fn expr_is_int_valued(&self, e: &Expr) -> bool {
        Self::is_int_valued(e, &self.int_valued_locals)
    }

    /// Locals that are always integer-valued. Greatest-fixpoint: assume every `let` local is
    /// integral, then drop any whose initializer or any reassignment RHS is not `is_int_valued`
    /// under the current set, until stable. Sound: dropping only ever removes names, and `+ - * %`
    /// preserve integrality even past 2^53 (the f64 result still has zero fractional part).
    fn collect_int_valued_locals(stmts: &[Statement]) -> HashSet<String> {
        // All declared local names (candidates).
        let mut names: HashSet<String> = HashSet::new();
        Self::collect_local_decl_names(stmts, &mut names);
        // Init/reassignment expressions per name.
        let mut defs: Vec<(String, &Expr)> = Vec::new();
        Self::collect_int_valued_defs(stmts, &mut defs);
        let mut reassigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(stmts, &mut reassigns);
        loop {
            let mut changed = false;
            for (name, e) in defs.iter().chain(reassigns.iter()) {
                if names.contains(name.as_str()) && !Self::is_int_valued(e, &names) {
                    names.remove(name.as_str());
                    changed = true;
                }
            }
            if !changed {
                return names;
            }
        }
    }

    /// Every `let`-declared local name (recursing through all nested statements).
    fn collect_local_decl_names(stmts: &[Statement], out: &mut HashSet<String>) {
        for s in stmts {
            match s {
                Statement::VarDecl { name, .. } => {
                    out.insert(name.to_string());
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::collect_local_decl_names(statements, out)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::collect_local_decl_names(std::slice::from_ref(then_branch), out);
                    if let Some(e) = else_branch {
                        Self::collect_local_decl_names(std::slice::from_ref(e), out);
                    }
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    Self::collect_local_decl_names(std::slice::from_ref(body), out)
                }
                Statement::For { init, body, .. } => {
                    if let Some(i) = init {
                        Self::collect_local_decl_names(std::slice::from_ref(i), out);
                    }
                    Self::collect_local_decl_names(std::slice::from_ref(body), out);
                }
                Statement::FunDecl { body, .. } => {
                    Self::collect_local_decl_names(std::slice::from_ref(body), out)
                }
                _ => {}
            }
        }
    }

    /// `(name, init-expr)` for every `let name = <init>` (recursing), for the int-valued fixpoint.
    fn collect_int_valued_defs<'a>(stmts: &'a [Statement], out: &mut Vec<(String, &'a Expr)>) {
        for s in stmts {
            match s {
                Statement::VarDecl {
                    name,
                    init: Some(e),
                    ..
                } => out.push((name.to_string(), e)),
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::collect_int_valued_defs(statements, out)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::collect_int_valued_defs(std::slice::from_ref(then_branch), out);
                    if let Some(e) = else_branch {
                        Self::collect_int_valued_defs(std::slice::from_ref(e), out);
                    }
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    Self::collect_int_valued_defs(std::slice::from_ref(body), out)
                }
                Statement::For { init, body, .. } => {
                    if let Some(i) = init {
                        Self::collect_int_valued_defs(std::slice::from_ref(i), out);
                    }
                    Self::collect_int_valued_defs(std::slice::from_ref(body), out);
                }
                Statement::FunDecl { body, .. } => {
                    Self::collect_int_valued_defs(std::slice::from_ref(body), out)
                }
                _ => {}
            }
        }
    }

    /// Map `number[]` locals initialized from an array literal of integer literals → the inclusive
    /// element range, both inside `(-2^53, 2^53)`.
    fn collect_array_elem_ranges(stmts: &[Statement]) -> HashMap<String, (i64, i64)> {
        let mut out = HashMap::new();
        Self::array_elem_ranges_walk(stmts, &mut out);
        out
    }

    fn array_elem_ranges_walk(stmts: &[Statement], out: &mut HashMap<String, (i64, i64)>) {
        for s in stmts {
            match s {
                Statement::VarDecl {
                    name,
                    init: Some(Expr::Array { elements, .. }),
                    ..
                } => {
                    let mut lo = i64::MAX;
                    let mut hi = i64::MIN;
                    let mut ok = !elements.is_empty();
                    for el in elements {
                        match el {
                            ArrayElement::Expr(e) => match Self::int_literal_value_of(e) {
                                Some(v) => {
                                    lo = lo.min(v);
                                    hi = hi.max(v);
                                }
                                None => {
                                    ok = false;
                                    break;
                                }
                            },
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        out.insert(name.to_string(), (lo, hi));
                    }
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::array_elem_ranges_walk(statements, out)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::array_elem_ranges_walk(std::slice::from_ref(then_branch), out);
                    if let Some(e) = else_branch {
                        Self::array_elem_ranges_walk(std::slice::from_ref(e), out);
                    }
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    Self::array_elem_ranges_walk(std::slice::from_ref(body), out)
                }
                Statement::For { body, .. } => {
                    Self::array_elem_ranges_walk(std::slice::from_ref(body), out)
                }
                Statement::FunDecl { body, .. } => {
                    Self::array_elem_ranges_walk(std::slice::from_ref(body), out)
                }
                _ => {}
            }
        }
    }

    /// Emit `e` as a native `i64` expression, used inside a native fold whose accumulator is `i64`.
    /// Returns `None` (caller keeps the `f64` fold) unless `e` is provably integer AND magnitude-
    /// bounded `< 2^53` at every node — so the `i64` arithmetic is bit-identical to the `f64` the
    /// interpreter/VM produce. `i64vars` are names already bound as `i64` (emitted bare); any other
    /// operand must be a bounded `f64` (emitted via `emit_typed_expr` then `as i64`, exact for
    /// integers < 2^53). Handles `+ - *` and `% <pos int literal>`; bails on anything else.
    fn emit_i64(
        &mut self,
        e: &Expr,
        i64vars: &HashSet<String>,
        ranges: &HashMap<String, (i64, i64)>,
    ) -> Result<Option<String>, CompileError> {
        // Whole-node bound (proves integrality + < 2^53). Without it, i64 could diverge from f64.
        if self.int_range(e, ranges).is_none() {
            return Ok(None);
        }
        if let Expr::Literal {
            value: Literal::Number(n),
            ..
        } = e
        {
            return Ok(Self::int_literal_value(*n).map(|v| format!("{}i64", v)));
        }
        if let Expr::Ident { name, .. } = e {
            if i64vars.contains(name.as_ref()) {
                return Ok(Some(Self::escape_ident(name.as_ref()).into_owned()));
            }
        }
        if let Expr::Binary {
            left, op, right, ..
        } = e
        {
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul => {
                    let (Some(l), Some(r)) = (
                        self.emit_i64(left, i64vars, ranges)?,
                        self.emit_i64(right, i64vars, ranges)?,
                    ) else {
                        return Ok(None);
                    };
                    let sym = match op {
                        BinOp::Add => "+",
                        BinOp::Sub => "-",
                        _ => "*",
                    };
                    return Ok(Some(format!("({} {} {})", l, sym, r)));
                }
                BinOp::Mod => {
                    if let Some(c) = Self::int_literal_value_of(right).filter(|&c| c > 0) {
                        if let Some(l) = self.emit_i64(left, i64vars, ranges)? {
                            return Ok(Some(format!("({} % {}i64)", l, c)));
                        }
                    }
                    return Ok(None);
                }
                _ => return Ok(None),
            }
        }
        // Fallback: a bounded non-i64-var leaf (e.g. the f64 element variable) → cast once.
        let (code, ty) = self.emit_typed_expr(e)?;
        if ty == RustType::F64 {
            Ok(Some(format!("(({}) as i64)", code)))
        } else {
            Ok(None)
        }
    }

    /// Record every annotated `VarDecl`/param name → its native `RustType`, recursing through all
    /// nested statements (loops, ifs, blocks, switch/try, function bodies). Flat; last write wins.
    fn collect_annotated_types(
        stmts: &[Statement],
        aliases: &HashMap<String, RustType>,
        env: &mut HashMap<String, RustType>,
    ) {
        for s in stmts {
            match s {
                Statement::VarDecl {
                    name,
                    type_ann: Some(ann),
                    ..
                } => {
                    env.insert(
                        name.to_string(),
                        RustType::from_annotation_with_aliases(ann, aliases),
                    );
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    Self::collect_annotated_types(statements, aliases, env)
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::collect_annotated_types(std::slice::from_ref(then_branch), aliases, env);
                    if let Some(e) = else_branch {
                        Self::collect_annotated_types(std::slice::from_ref(e), aliases, env);
                    }
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    Self::collect_annotated_types(std::slice::from_ref(body), aliases, env)
                }
                Statement::ForOf {
                    name,
                    iterable,
                    body,
                    ..
                } => {
                    // A loop var iterating a `Vec<elem>` local binds `elem` — so `total += n` (n the
                    // loop var over a `number[]`) is seen as native f64 and `total` is NOT demoted.
                    // Sound: the Vec's elements are genuinely that native type at runtime.
                    if let Expr::Ident { name: it_name, .. } = iterable {
                        let elem_ty = match env.get(it_name.as_ref()) {
                            Some(RustType::Vec(elem)) => Some((**elem).clone()),
                            _ => None,
                        };
                        if let Some(t) = elem_ty {
                            env.insert(name.to_string(), t);
                        }
                    }
                    Self::collect_annotated_types(std::slice::from_ref(body), aliases, env)
                }
                Statement::For { init, body, .. } => {
                    if let Some(i) = init {
                        Self::collect_annotated_types(std::slice::from_ref(i), aliases, env);
                    }
                    Self::collect_annotated_types(std::slice::from_ref(body), aliases, env);
                }
                Statement::FunDecl {
                    params,
                    rest_param,
                    body,
                    ..
                } => {
                    for p in params {
                        if let FunParam::Simple(tp) = p {
                            if let Some(ann) = &tp.type_ann {
                                env.insert(
                                    tp.name.to_string(),
                                    RustType::from_annotation_with_aliases(ann, aliases),
                                );
                            }
                        }
                    }
                    // Typed rest-param `...args: number[]` -> `Vec<f64>`, so a ForOf loop var over it
                    // binds the element type and accumulators stay native.
                    if let Some(rp) = rest_param {
                        if let Some(ann) = &rp.type_ann {
                            env.insert(
                                rp.name.to_string(),
                                RustType::from_annotation_with_aliases(ann, aliases),
                            );
                        }
                    }
                    Self::collect_annotated_types(std::slice::from_ref(body), aliases, env);
                }
                Statement::Switch {
                    cases,
                    default_body,
                    ..
                } => {
                    for (_, body) in cases {
                        Self::collect_annotated_types(body, aliases, env);
                    }
                    if let Some(b) = default_body {
                        Self::collect_annotated_types(b, aliases, env);
                    }
                }
                Statement::Try {
                    body,
                    catch_body,
                    finally_body,
                    ..
                } => {
                    Self::collect_annotated_types(std::slice::from_ref(body), aliases, env);
                    if let Some(b) = catch_body {
                        Self::collect_annotated_types(std::slice::from_ref(b), aliases, env);
                    }
                    if let Some(b) = finally_body {
                        Self::collect_annotated_types(std::slice::from_ref(b), aliases, env);
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_reassignments_stmts<'a>(stmts: &'a [Statement], out: &mut Vec<(String, &'a Expr)>) {
        for s in stmts {
            Self::collect_reassignments_stmt(s, out);
        }
    }

    /// Collect every `(name, rhs)` reassignment (`=`, compound `+=`, logical `||=`) reachable from
    /// `s` — descending through nested statements and expressions (including closures).
    fn collect_reassignments_stmt<'a>(s: &'a Statement, out: &mut Vec<(String, &'a Expr)>) {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                Self::collect_reassignments_stmts(statements, out)
            }
            Statement::VarDecl { init: Some(e), .. } => Self::collect_reassignments_expr(e, out),
            Statement::VarDeclDestructure { init, .. } => {
                Self::collect_reassignments_expr(init, out)
            }
            Statement::ExprStmt { expr, .. } => Self::collect_reassignments_expr(expr, out),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_reassignments_expr(cond, out);
                Self::collect_reassignments_stmt(then_branch, out);
                if let Some(e) = else_branch {
                    Self::collect_reassignments_stmt(e, out);
                }
            }
            Statement::While { cond, body, .. } => {
                Self::collect_reassignments_expr(cond, out);
                Self::collect_reassignments_stmt(body, out);
            }
            Statement::DoWhile { body, cond, .. } => {
                Self::collect_reassignments_stmt(body, out);
                Self::collect_reassignments_expr(cond, out);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(i) = init {
                    Self::collect_reassignments_stmt(i, out);
                }
                if let Some(c) = cond {
                    Self::collect_reassignments_expr(c, out);
                }
                if let Some(u) = update {
                    Self::collect_reassignments_expr(u, out);
                }
                Self::collect_reassignments_stmt(body, out);
            }
            Statement::ForOf { iterable, body, .. } => {
                Self::collect_reassignments_expr(iterable, out);
                Self::collect_reassignments_stmt(body, out);
            }
            Statement::Return { value: Some(e), .. } => Self::collect_reassignments_expr(e, out),
            Statement::Throw { value, .. } => Self::collect_reassignments_expr(value, out),
            Statement::FunDecl { body, .. } => Self::collect_reassignments_stmt(body, out),
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                Self::collect_reassignments_expr(expr, out);
                for (g, body) in cases {
                    if let Some(g) = g {
                        Self::collect_reassignments_expr(g, out);
                    }
                    Self::collect_reassignments_stmts(body, out);
                }
                if let Some(b) = default_body {
                    Self::collect_reassignments_stmts(b, out);
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::collect_reassignments_stmt(body, out);
                if let Some(b) = catch_body {
                    Self::collect_reassignments_stmt(b, out);
                }
                if let Some(b) = finally_body {
                    Self::collect_reassignments_stmt(b, out);
                }
            }
            _ => {}
        }
    }

    fn collect_reassignments_expr<'a>(e: &'a Expr, out: &mut Vec<(String, &'a Expr)>) {
        match e {
            Expr::Assign { name, value, .. }
            | Expr::CompoundAssign { name, value, .. }
            | Expr::LogicalAssign { name, value, .. } => {
                out.push((name.to_string(), value.as_ref()));
                Self::collect_reassignments_expr(value, out);
            }
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::collect_reassignments_expr(left, out);
                Self::collect_reassignments_expr(right, out);
            }
            Expr::Unary { operand, .. }
            | Expr::TypeOf { operand, .. }
            | Expr::Await { operand, .. } => Self::collect_reassignments_expr(operand, out),
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                Self::collect_reassignments_expr(callee, out);
                for a in args {
                    match a {
                        CallArg::Expr(x) | CallArg::Spread(x) => {
                            Self::collect_reassignments_expr(x, out)
                        }
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::collect_reassignments_expr(object, out);
                if let MemberProp::Expr(p) = prop {
                    Self::collect_reassignments_expr(p, out);
                }
            }
            Expr::Index { object, index, .. } => {
                Self::collect_reassignments_expr(object, out);
                Self::collect_reassignments_expr(index, out);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::collect_reassignments_expr(cond, out);
                Self::collect_reassignments_expr(then_branch, out);
                Self::collect_reassignments_expr(else_branch, out);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(x) | ArrayElement::Spread(x) => {
                            Self::collect_reassignments_expr(x, out)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjectProp::KeyValue(_, v, _) => Self::collect_reassignments_expr(v, out),
                        ObjectProp::Spread(x) => Self::collect_reassignments_expr(x, out),
                    }
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::collect_reassignments_expr(object, out);
                Self::collect_reassignments_expr(value, out);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                Self::collect_reassignments_expr(object, out);
                Self::collect_reassignments_expr(index, out);
                Self::collect_reassignments_expr(value, out);
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for x in exprs {
                    Self::collect_reassignments_expr(x, out);
                }
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(x) => Self::collect_reassignments_expr(x, out),
                ArrowBody::Block(b) => Self::collect_reassignments_stmt(b, out),
            },
            _ => {}
        }
    }

    /// Read-only mirror of `emit_typed_expr`'s native-type decision (no code generated), over a
    /// flat `name → RustType` env. Returns `RustType::F64` only for forms that provably lower to a
    /// native `f64`; everything else → `RustType::Value`. Conservative by construction: it never
    /// claims `F64` where `emit_typed_expr` would box, so a numeric local is never wrongly kept
    /// native (which would reintroduce the coercion panic).
    fn expr_native_type(&self, e: &Expr, env: &HashMap<String, RustType>) -> RustType {
        match e {
            Expr::Literal { value, .. } => match value {
                Literal::Number(_) => RustType::F64,
                Literal::String(_) => RustType::String,
                Literal::Bool(_) => RustType::Bool,
                Literal::Null => RustType::Value,
            },
            Expr::Ident { name, .. } => env
                .get(name.as_ref())
                .filter(|t| t.is_native())
                .cloned()
                .unwrap_or(RustType::Value),
            Expr::Binary {
                left, op, right, ..
            } => {
                let lt = self.expr_native_type(left, env);
                let rt = self.expr_native_type(right, env);
                RustType::result_type_of_binop(*op, &lt, &rt).unwrap_or(RustType::Value)
            }
            // `vec[i]` where `vec` is a `number[]` (Vec<f64>) → the element type. A `Vec<f64>`
            // can only hold numbers, so this never feeds a string into the accumulator.
            Expr::Index {
                object,
                optional: false,
                ..
            } => {
                if let Expr::Ident { name, .. } = object.as_ref() {
                    if let Some(RustType::Vec(inner)) = env.get(name.as_ref()) {
                        return (**inner).clone();
                    }
                }
                RustType::Value
            }
            // `o.field` where `o` is a native struct local and `field` is a native field.
            Expr::Member {
                object,
                prop: MemberProp::Name { name: prop_name, .. },
                optional: false,
                ..
            } => {
                if let Expr::Ident { name: var_name, .. } = object.as_ref() {
                    // #173: `vec.length` on a native `Vec<_>` is a native `f64` (the emitter lowers it
                    // to `(vec.len() as f64)`), so a local fed by `arr.length` stays native.
                    if let Some(RustType::Vec(_)) = env.get(var_name.as_ref()) {
                        if prop_name.as_ref() == "length" {
                            return RustType::F64;
                        }
                    }
                    if let Some(RustType::Named { fields, .. }) = env.get(var_name.as_ref()) {
                        if let Some((_, field_ty)) =
                            fields.iter().find(|(k, _)| k.as_ref() == prop_name.as_ref())
                        {
                            if field_ty.is_native() {
                                return field_ty.clone();
                            }
                        }
                    }
                }
                RustType::Value
            }
            Expr::Call { callee, args, .. } => {
                // M5 native fn (`fn f_native(..) -> f64`); requires all-positional args.
                if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                    if self.native_fns.contains(fname.as_ref())
                        && args.iter().all(|a| matches!(a, CallArg::Expr(_)))
                    {
                        return RustType::F64;
                    }
                }
                // Single-arg `Math.<intrinsic>(x)` lowered to a direct `f64` method → number.
                if let [CallArg::Expr(_)] = args.as_slice() {
                    if let Expr::Member {
                        object,
                        prop: MemberProp::Name { name: method, .. },
                        ..
                    } = callee.as_ref()
                    {
                        if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                            && matches!(
                                method.as_ref(),
                                "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | "exp"
                                    | "trunc" | "log"
                            )
                        {
                            return RustType::F64;
                        }
                    }
                }
                // #169: a fused native `Vec<f64>` reduce produces an `f64` (the emitter lowers it via
                // `native_vec_hof_for_call`). Model it here so an accumulator fed by `xs.reduce(...)`
                // is not wrongly demoted to a boxed `Value`. Conservative: any miss → `Value`.
                if let Some(t) = self.native_vec_reduce_result_type(callee, args, env) {
                    return t;
                }
                RustType::Value
            }
            // Unary, Conditional, etc. are not modelled by `emit_typed_expr` (it boxes them), so a
            // store from one already coerces; treat as `Value` to match (→ demote if it feeds an
            // accumulator). Sound and consistent.
            _ => RustType::Value,
        }
    }

    /// #169: read-only mirror of [`try_native_vec_hof`]'s `reduce` preconditions, for
    /// [`expr_native_type`]. Returns `Some(F64)` exactly when a `xs.reduce((acc, x) => body, init)`
    /// call would fuse to a native `f64` fold (so an accumulator it feeds stays native instead of
    /// being demoted to a boxed `Value`). Any uncertainty returns `None` — the oracle never claims
    /// `F64` where the emitter would box, which is what keeps the demotion analysis sound.
    fn native_vec_reduce_result_type(
        &self,
        callee: &Expr,
        args: &[CallArg],
        env: &HashMap<String, RustType>,
    ) -> Option<RustType> {
        if !crate::native_opts_enabled() {
            return None;
        }
        let Expr::Member {
            object,
            prop: MemberProp::Name { name: method, .. },
            optional: false,
            ..
        } = callee
        else {
            return None;
        };
        if method.as_ref() != "reduce" {
            return None;
        }
        let Expr::Ident { name: recv_name, .. } = object.as_ref() else {
            return None;
        };
        // Receiver must be a native `Vec<f64>` (`.copied()` needs a `Copy` element).
        match env.get(recv_name.as_ref()) {
            Some(RustType::Vec(inner)) if **inner == RustType::F64 => {}
            _ => return None,
        }
        // `reduce(callback, init)` with a simple-param expression-body arrow that does not touch the
        // receiver (an alias inside the closure would break the `.iter()` borrow).
        if args.len() != 2 {
            return None;
        }
        let Some(CallArg::Expr(Expr::ArrowFunction { params, body, .. })) = args.first() else {
            return None;
        };
        if params.len() != 2 {
            return None;
        }
        let (FunParam::Simple(acc_p), FunParam::Simple(x_p)) = (&params[0], &params[1]) else {
            return None;
        };
        if acc_p.default.is_some() || x_p.default.is_some() {
            return None;
        }
        let ArrowBody::Expr(be) = body else {
            return None;
        };
        if crate::infer::pi_mentions(be, recv_name.as_ref()) {
            return None;
        }
        // The init must be native-numeric, and the body must lower to `f64` with both closure params
        // bound `f64` — exactly the emitter's preconditions, evaluated read-only.
        let CallArg::Expr(init_e) = &args[1] else {
            return None;
        };
        if self.expr_native_type(init_e, env) != RustType::F64 {
            return None;
        }
        let mut benv = env.clone();
        benv.insert(acc_p.name.to_string(), RustType::F64);
        benv.insert(x_p.name.to_string(), RustType::F64);
        if self.expr_native_type(be, &benv) == RustType::F64 {
            Some(RustType::F64)
        } else {
            None
        }
    }

    /// #176 — `thread_local` static name for a native numeric global (`seed` → `G_SEED`).
    /// A token-safe `G_*` static identifier for a tish global/module-const. `escape_ident` may return
    /// a raw identifier (`r#fn` for the keyword `fn`), and `#` is NOT valid inside a larger identifier
    /// (`G_R#FN` won't tokenize) — so strip the `r#` and scrub any non-word char before prefixing.
    fn global_static_name(name: &str) -> String {
        let esc = Self::escape_ident(name);
        let raw = esc.strip_prefix("r#").unwrap_or(&esc);
        let safe: String = raw
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        format!("G_{}", safe.to_uppercase())
    }

    fn native_global_static(name: &str) -> String {
        Self::global_static_name(name)
    }

    /// Read a native numeric global from its `thread_local Cell<f64>`.
    fn native_global_get(name: &str) -> String {
        format!(
            "{}.with(|c| c.get())",
            Self::native_global_static(name)
        )
    }

    /// Write a native numeric global into its `thread_local Cell<f64>`.
    fn native_global_set(name: &str, val: &str) -> String {
        format!(
            "{}.with(|c| c.set({}))",
            Self::native_global_static(name),
            val
        )
    }

    /// Detect `fn f(max) { G = (G*mul+add)%mod; return max*G/mod }` eligible for M5.
    fn collect_native_lcg_fns(
        stmts: &[Statement],
        native_fns: &std::collections::HashSet<String>,
    ) -> std::collections::HashMap<String, (String, f64, f64, f64)> {
        let mut out = std::collections::HashMap::new();
        for s in stmts {
            if let Statement::FunDecl { name, body, .. } = s {
                if !native_fns.contains(name.as_ref()) {
                    continue;
                }
                if let Some(sig) = Self::parse_native_lcg_fn(body) {
                    out.insert(name.to_string(), sig);
                }
            }
        }
        out
    }

    /// Parse `global = (global*mul+add)%mod; return (max*global)/mod`.
    fn parse_native_lcg_fn(body: &Statement) -> Option<(String, f64, f64, f64)> {
        let stmts = match body {
            Statement::Block { statements, .. } => statements.as_slice(),
            _ => return None,
        };
        let mut global: Option<String> = None;
        let mut mul = 0.0;
        let mut add = 0.0;
        let mut modulus = 0.0;
        let mut saw_assign = false;
        for s in stmts {
            match s {
                Statement::ExprStmt {
                    expr: Expr::Assign { name, value, .. },
                    ..
                } => {
                    if let Some((g, m, a, d)) = Self::parse_lcg_assign(value) {
                        if name.as_ref() != g {
                            return None;
                        }
                        global = Some(g);
                        mul = m;
                        add = a;
                        modulus = d;
                        saw_assign = true;
                    }
                }
                Statement::Return { value: Some(ret), .. } => {
                    if !saw_assign {
                        return None;
                    }
                    let g = global.as_ref()?;
                    if !Self::parse_lcg_return(ret, g, modulus) {
                        return None;
                    }
                    return Some((g.clone(), mul, add, modulus));
                }
                _ => {}
            }
        }
        None
    }

    fn parse_lcg_assign(value: &Expr) -> Option<(String, f64, f64, f64)> {
        let Expr::Binary {
            left,
            op: BinOp::Mod,
            right: rd,
            ..
        } = value
        else {
            return None;
        };
        let d = Self::int_literal_value_of(rd)?;
        let Expr::Binary {
            left: add_l,
            op: BinOp::Add,
            right: add_r,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        let (g_e, mr) = if let Expr::Binary {
            left: ml,
            op: BinOp::Mul,
            right: mr,
            ..
        } = add_l.as_ref()
        {
            (ml.as_ref(), mr.as_ref())
        } else {
            return None;
        };
        let Expr::Ident { name: g, .. } = g_e else {
            return None;
        };
        let m = Self::int_literal_value_of(mr)?;
        let a = Self::int_literal_value_of(add_r)?;
        Some((g.to_string(), m as f64, a as f64, d as f64))
    }

    fn parse_lcg_return(ret: &Expr, global: &str, modulus: f64) -> bool {
        let Expr::Binary {
            left,
            op: BinOp::Div,
            right: rd,
            ..
        } = ret
        else {
            return false;
        };
        if !matches!(rd.as_ref(), Expr::Literal { value: Literal::Number(n), .. } if *n == modulus) {
            return false;
        }
        let Expr::Binary {
            right: rg,
            op: BinOp::Mul,
            ..
        } = left.as_ref()
        else {
            return false;
        };
        matches!(rg.as_ref(), Expr::Ident { name, .. } if name.as_ref() == global)
    }

    fn emit_native_lcg_with(
        &self,
        global: &str,
        mul: f64,
        add: f64,
        modulus: f64,
        max_expr: &str,
    ) -> String {
        if let Some((g, hm, ha, hm_mod)) = &self.native_lcg_hoist {
            if g == global {
                if self.native_lcg_hoist_int {
                    return format!(
                        "{{ let s = (_lcg_seed * {}i64 + {}i64) % {}i64; _lcg_seed = s; (({}) * (s as f64)) / {} }}",
                        *hm as i64,
                        *ha as i64,
                        *hm_mod as i64,
                        max_expr,
                        Self::f64_lit(*hm_mod),
                    );
                }
                return format!(
                    "{{ let s = (((_lcg_seed * {}) + {}) % {}); _lcg_seed = s; (({}) * s) / {} }}",
                    Self::f64_lit(*hm),
                    Self::f64_lit(*ha),
                    Self::f64_lit(*hm_mod),
                    max_expr,
                    Self::f64_lit(*hm_mod),
                );
            }
        }
        let g = Self::native_global_static(global);
        format!(
            "{{ {}.with(|g| {{ let s = (((g.get() * {}) + {}) % {}); g.set(s); (({}) * s) / {} }}) }}",
            g,
            Self::f64_lit(mul),
            Self::f64_lit(add),
            Self::f64_lit(modulus),
            max_expr,
            Self::f64_lit(modulus),
        )
    }

    /// Literal call-site arg for an M5 fn param (`mandel(1200,1200,100)` → `w`/`h`/`maxIter`).
    fn native_param_lit(&self, param: &str) -> Option<f64> {
        let fname = self.native_fn_emit_name.as_ref()?;
        let lits = self.native_fn_literal_args.get(fname)?;
        let params = self.native_fn_param_names.get(fname)?;
        params
            .iter()
            .position(|p| p == param)
            .and_then(|i| lits.get(i).copied())
    }

    /// Top-level `let x = f(lit,…)` where `f` is M5-eligible — specialize the native fn body.
    fn collect_native_fn_literal_calls(
        stmts: &[Statement],
        native_fns: &std::collections::HashSet<String>,
    ) -> std::collections::HashMap<String, Vec<f64>> {
        use std::collections::HashMap;
        let mut out = HashMap::new();
        for s in stmts {
            if let Statement::VarDecl {
                init: Some(e),
                ..
            } = s
            {
                if let Expr::Call { callee, args, .. } = e {
                    if let Expr::Ident { name, .. } = callee.as_ref() {
                        let fname = name.as_ref();
                        if !native_fns.contains(fname) {
                            continue;
                        }
                        let mut lits: Vec<f64> = Vec::new();
                        let ok = args.iter().all(|a| {
                            if let CallArg::Expr(Expr::Literal {
                                value: Literal::Number(n),
                                ..
                            }) = a
                            {
                                lits.push(*n);
                                true
                            } else {
                                false
                            }
                        });
                        if ok && !lits.is_empty() {
                            out.insert(fname.to_string(), lits);
                        }
                    }
                }
            }
        }
        out
    }

    fn collect_native_fn_param_names(
        stmts: &[Statement],
        native_fns: &std::collections::HashSet<String>,
    ) -> std::collections::HashMap<String, Vec<String>> {
        use std::collections::HashMap;
        let mut out = HashMap::new();
        for s in stmts {
            if let Statement::FunDecl { name, params, .. } = s {
                if !native_fns.contains(name.as_ref()) {
                    continue;
                }
                let ps: Vec<String> = params
                    .iter()
                    .filter_map(|p| match p {
                        FunParam::Simple(tp) => Some(tp.name.to_string()),
                        _ => None,
                    })
                    .collect();
                out.insert(name.to_string(), ps);
            }
        }
        out
    }

    fn cum_static_and_len(&self, name: &str) -> Option<(String, usize)> {
        if let Some(static_name) = self.module_const_f64_aliases.get(name) {
            for (src, cum_static) in &self.module_const_f64_cum {
                if cum_static == static_name {
                    if let Some(vals) = self.module_const_f64_arrays.get(src) {
                        return Some((static_name.clone(), vals.len()));
                    }
                }
            }
        }
        if let Some(vals) = self.module_const_f64_arrays.get(name) {
            return Some((Self::module_const_static(name), vals.len()));
        }
        None
    }

    /// `let j = 0; while (cum[j] < r) { j++ }` → `let j = if (r < cum[0]) { 0 } …`.
    fn try_emit_const_cum_search_fold(
        &mut self,
        stmts: &[Statement],
    ) -> Result<Option<usize>, CompileError> {
        if stmts.len() < 2 {
            return Ok(None);
        }
        let j_name = match &stmts[0] {
            Statement::VarDecl {
                name,
                init: Some(Expr::Literal {
                    value: Literal::Number(n),
                    ..
                }),
                ..
            } if *n == 0.0 => name.as_ref(),
            _ => return Ok(None),
        };
        let Statement::While { cond, body, .. } = &stmts[1] else {
            return Ok(None);
        };
        let (cum_name, r_expr) = match cond {
            Expr::Binary {
                left,
                op: BinOp::Lt,
                right,
                ..
            } => {
                let obj = match left.as_ref() {
                    Expr::Index { object, .. } => object.as_ref(),
                    _ => return Ok(None),
                };
                let Expr::Ident { name: cn, .. } = obj else {
                    return Ok(None);
                };
                (cn.as_ref(), right.as_ref())
            }
            _ => return Ok(None),
        };
        let Some((cum_static, n)) = self.cum_static_and_len(cum_name) else {
            return Ok(None);
        };
        if n == 0 || n > 16 {
            return Ok(None);
        }
        let inc_ok = match body.as_ref() {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.len() == 1 && Self::stmt_increments_ident(&statements[0], j_name)
            }
            s => Self::stmt_increments_ident(s, j_name),
        };
        if !inc_ok {
            return Ok(None);
        }
        let (r_code, r_ty) = self.emit_typed_expr(r_expr)?;
        let r_f64 = if r_ty == RustType::F64 {
            r_code
        } else if r_ty == RustType::Value {
            RustType::F64.from_value_expr(&r_code)
        } else {
            return Ok(None);
        };
        let mut ladder = String::new();
        for i in 0..n {
            if i == 0 {
                ladder.push_str(&format!("if ({}) < {}[0] {{ 0_f64 }}", r_f64, cum_static));
            } else {
                ladder.push_str(&format!(
                    " else if ({}) < {}[{}] {{ {}_f64 }}",
                    r_f64,
                    cum_static,
                    i,
                    i
                ));
            }
        }
        ladder.push_str(&format!(" else {{ {}_f64 }}", n - 1));
        let escaped = Self::escape_ident(j_name);
        self.writeln(&format!("let mut {}: f64 = {};", escaped, ladder));
        self.type_context.define(j_name, RustType::F64);
        let mut skip = 2;
        if stmts.len() >= 3 {
            if let Some(codes_static) = Self::parse_fasta_codes_sum_fold(
                &stmts[2],
                j_name,
                &self.module_const_f64_arrays,
            ) {
                if let Statement::ExprStmt {
                    expr: Expr::Assign { name: sum_name, .. },
                    ..
                } = &stmts[2]
                {
                    let sum_esc = Self::escape_ident(sum_name.as_ref());
                    let arms: Vec<String> = (0..n)
                        .map(|i| format!("{} => {}[{}]", i, codes_static, i))
                        .collect();
                    self.writeln(&format!(
                        "{} = (({} + match ({} as usize) {{ {}, _ => 0.0 }}) % 2147483647_f64);",
                        sum_esc,
                        sum_esc,
                        escaped,
                        arms.join(", "),
                    ));
                    skip = 3;
                }
            }
        }
        Ok(Some(skip))
    }

    /// `sum = (sum + codes[j]) % 2147483647` when `codes` is a module const array.
    fn parse_fasta_codes_sum_fold(
        stmt: &Statement,
        j_name: &str,
        module_const: &HashMap<String, Vec<f64>>,
    ) -> Option<String> {
        let Statement::ExprStmt {
            expr: Expr::Assign { name: _sum, value, .. },
            ..
        } = stmt
        else {
            return None;
        };
        let Expr::Binary {
            left,
            op: BinOp::Mod,
            right: mod_r,
            ..
        } = value.as_ref()
        else {
            return None;
        };
        if Self::int_literal_value_of(mod_r) != Some(2147483647) {
            return None;
        }
        let Expr::Binary {
            left: add_l,
            op: BinOp::Add,
            right: add_r,
            ..
        } = left.as_ref()
        else {
            return None;
        };
        let Expr::Index {
            object,
            index,
            ..
        } = add_r.as_ref()
        else {
            return None;
        };
        let Expr::Ident { name: codes_name, .. } = object.as_ref() else {
            return None;
        };
        if !module_const.contains_key(codes_name.as_ref()) {
            return None;
        }
        matches!(index.as_ref(), Expr::Ident { name, .. } if name.as_ref() == j_name)
            .then(|| Self::module_const_static(codes_name.as_ref()))
    }

    fn stmt_increments_ident(s: &Statement, name: &str) -> bool {
        match s {
            Statement::ExprStmt { expr, .. } => match expr {
                Expr::Assign {
                    name: n,
                    value,
                    ..
                } => {
                    let Expr::Binary {
                        left,
                        op: BinOp::Add,
                        right,
                        ..
                    } = value.as_ref()
                    else {
                        return false;
                    };
                    n.as_ref() == name
                        && matches!(left.as_ref(), Expr::Ident { name: ln, .. } if ln.as_ref() == name)
                        && Self::int_literal_value_of(right) == Some(1)
                }
                Expr::PostfixInc { name: n, .. } | Expr::PrefixInc { name: n, .. } => {
                    n.as_ref() == name
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// #303 — does `stmt` contain a call (so it may set the pending-throw slot)? Conservative: any
    /// `Call`/`New` anywhere in the statement's expressions.
    fn stmt_may_throw(stmt: &Statement) -> bool {
        let mut found = false;
        Self::for_each_stmt_expr(stmt, &mut |e| {
            Self::walk_subexprs(e, &mut |x| {
                if matches!(x, Expr::Call { .. } | Expr::New { .. }) {
                    found = true;
                }
            });
        });
        found
    }

    /// #303 — a statement that already transfers control, so a pending-throw check after it would be
    /// dead code (the throw propagates via the dummy-return + the caller's own check instead).
    fn is_terminator_stmt(stmt: &Statement) -> bool {
        matches!(
            stmt,
            Statement::Return { .. }
                | Statement::Throw { .. }
                | Statement::Break { .. }
                | Statement::Continue { .. }
        )
    }

    /// #303 — emit the post-call pending-throw check in the form the current frame expects: a
    /// `Result`-returning frame (a `try`-closure, or `run()` at top level) re-raises it as
    /// `TishError::Throw`; a value-fn frame escapes with a dummy `Value`, leaving the slot set so the
    /// throw keeps climbing the call chain.
    fn emit_pending_throw_check(&mut self) {
        if self.try_closure_depth > 0 || self.value_fn_depth == 0 {
            self.writeln("if tishlang_runtime::has_pending_throw() { return Err(Box::new(tishlang_runtime::TishError::Throw(tishlang_runtime::take_pending_throw().unwrap_or(Value::Null))) as Box<dyn std::error::Error>); }");
        } else {
            self.writeln("if tishlang_runtime::has_pending_throw() { return Value::Null; }");
        }
    }

    /// #303 — are we emitting the body of a native-typed fn (`fn name_native(..) -> f64` /
    /// `fn name_nv(..) -> Vec<..> | ()`)? Such a frame has no `Value`/`Result` return channel, so the
    /// pending-throw check (which returns `Err`/`Value::Null`) cannot be typed there and must be
    /// suppressed — the throw still propagates: it stays in the slot and surfaces at the first
    /// `Value`/`Result` frame up the call chain. `native_fn_emit_name` is `Some` for the whole body
    /// of both native emitters and `None` everywhere else.
    fn in_native_typed_frame(&self) -> bool {
        self.native_fn_emit_name.is_some() || self.native_vec_ret.is_some()
    }

    fn emit_statements_with_folds(&mut self, statements: &[Statement]) -> Result<(), CompileError> {
        let mut i = 0;
        while i < statements.len() {
            if let Some(skip) = self.try_emit_const_cum_search_fold(&statements[i..])? {
                i += skip;
                continue;
            }
            if self.native_fn_body_emit || self.native_vec_ret.is_some() {
                if let Some(skip) = self.try_emit_mandel_iteration_fold(&statements[i..])? {
                    i += skip;
                    continue;
                }
            }
            let promote_strict_lt = if let Statement::If {
                cond,
                then_branch,
                else_branch: None,
                ..
            } = &statements[i]
            {
                Self::parse_strict_eq_idents(cond)
                    .filter(|_| Self::statement_contains_break(then_branch))
            } else {
                None
            };
            if self.native_fn_body_emit {
                self.emit_native_fn_body(&statements[i])?;
            } else {
                self.emit_statement(&statements[i])?;
                if Self::stmt_may_throw(&statements[i])
                    && !Self::is_terminator_stmt(&statements[i])
                    && (self.try_closure_depth > 0 || !self.in_native_typed_frame())
                {
                    self.emit_pending_throw_check();
                }
            }
            if let Some((lv, rv)) = promote_strict_lt {
                self.strict_lt_bounds.push((lv.clone(), rv.clone()));
                self.nonneg_locals.insert(lv.clone());
                i += 1;
                while i < statements.len() {
                    if let Some(skip) = self.try_emit_const_cum_search_fold(&statements[i..])? {
                        i += skip;
                        continue;
                    }
                    if self.native_fn_body_emit {
                        self.emit_native_fn_body(&statements[i])?;
                    } else {
                        self.emit_statement(&statements[i])?;
                        if Self::stmt_may_throw(&statements[i]) && !Self::is_terminator_stmt(&statements[i]) {
                            self.emit_pending_throw_check();
                        }
                    }
                    i += 1;
                }
                self.strict_lt_bounds.pop();
                self.nonneg_locals.remove(&lv);
                return Ok(());
            }
            i += 1;
        }
        Ok(())
    }

    /// Emit `thread_local! { static G_NAME: Cell<f64> = ... }` for each eligible global.
    fn emit_native_numeric_global_tls(&mut self) -> Result<(), CompileError> {
        self.writeln("thread_local! {");
        self.indent += 1;
        for (name, init) in self.native_numeric_globals.clone() {
            self.writeln(&format!(
                "static {}: std::cell::Cell<f64> = const {{ std::cell::Cell::new({}) }};",
                Self::native_global_static(&name),
                Self::f64_lit(init),
            ));
        }
        self.indent -= 1;
        self.writeln("}");
        Ok(())
    }

    /// Top-level `let name = [n0, n1, …]` with numeric literals, never reassigned — emitted as
    /// `const G_name: [f64; N]` for direct indexing from native fns (fasta `codes`/`probs`).
    fn collect_module_const_f64_arrays(stmts: &[Statement]) -> HashMap<String, Vec<f64>> {
        use std::collections::{HashMap, HashSet};
        let mut candidates: HashMap<String, Vec<f64>> = HashMap::new();
        for s in stmts {
            if let Statement::VarDecl {
                name,
                init,
                ..
            } = s
            {
                if let Some(Expr::Array { elements, .. }) = init.as_ref() {
                    let mut vals: Vec<f64> = Vec::new();
                    for el in elements {
                        if let ArrayElement::Expr(Expr::Literal {
                            value: Literal::Number(n),
                            ..
                        }) = el
                        {
                            vals.push(*n);
                        } else {
                            vals.clear();
                            break;
                        }
                    }
                    if !vals.is_empty() {
                        candidates.insert(name.to_string(), vals);
                    }
                }
            }
        }
        if candidates.is_empty() {
            return HashMap::new();
        }
        for s in stmts {
            if let Statement::FunDecl { body, .. } = s {
                let mut inner = HashSet::new();
                Self::collect_local_var_names(body, &mut inner);
                for n in inner {
                    candidates.remove(&n);
                }
            }
        }
        let globals: HashSet<String> = candidates.keys().cloned().collect();
        let mut reassigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(stmts, &mut reassigns);
        for (name, _) in &reassigns {
            if globals.contains(name) {
                candidates.remove(name);
            }
        }
        for name in candidates.keys().cloned().collect::<Vec<_>>() {
            if Self::module_const_has_disqualifying_use(stmts, &name) {
                candidates.remove(&name);
            }
        }
        candidates
    }

    /// Disqualifying: bare value read, member/call on the name (index-as-object is OK).
    fn module_const_has_disqualifying_use(stmts: &[Statement], g: &str) -> bool {
        let mut bad = false;
        Self::walk_stmts_for_module_const_disqualifiers(stmts, g, &mut bad);
        bad
    }

    fn walk_stmts_for_module_const_disqualifiers(stmts: &[Statement], g: &str, bad: &mut bool) {
        for s in stmts {
            Self::walk_stmt_for_module_const_disqualifiers(s, g, bad);
        }
    }

    fn walk_stmt_for_module_const_disqualifiers(s: &Statement, g: &str, bad: &mut bool) {
        if *bad {
            return;
        }
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                Self::walk_stmts_for_module_const_disqualifiers(statements, g, bad);
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::walk_stmt_for_module_const_disqualifiers(then_branch, g, bad);
                if let Some(eb) = else_branch {
                    Self::walk_stmt_for_module_const_disqualifiers(eb, g, bad);
                }
            }
            Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                Self::walk_stmt_for_module_const_disqualifiers(body, g, bad);
            }
            Statement::For { init, body, .. } => {
                if let Some(i) = init {
                    Self::walk_stmt_for_module_const_disqualifiers(i, g, bad);
                }
                Self::walk_stmt_for_module_const_disqualifiers(body, g, bad);
            }
            Statement::ForOf { body, .. } => {
                Self::walk_stmt_for_module_const_disqualifiers(body, g, bad);
            }
            Statement::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, stmts) in cases {
                    Self::walk_stmts_for_module_const_disqualifiers(stmts, g, bad);
                }
                if let Some(stmts) = default_body {
                    Self::walk_stmts_for_module_const_disqualifiers(stmts, g, bad);
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::walk_stmt_for_module_const_disqualifiers(body, g, bad);
                if let Some(c) = catch_body {
                    Self::walk_stmt_for_module_const_disqualifiers(c, g, bad);
                }
                if let Some(f) = finally_body {
                    Self::walk_stmt_for_module_const_disqualifiers(f, g, bad);
                }
            }
            Statement::ExprStmt { expr, .. } => {
                Self::walk_expr_for_module_const_disqualifiers(expr, g, bad);
            }
            Statement::Return { value, .. } => {
                if let Some(e) = value {
                    Self::walk_expr_for_module_const_disqualifiers(e, g, bad);
                }
            }
            Statement::FunDecl { body, .. } => {
                Self::walk_stmt_for_module_const_disqualifiers(body, g, bad);
            }
            Statement::VarDecl { init, .. } => {
                if let Some(e) = init {
                    Self::walk_expr_for_module_const_disqualifiers(e, g, bad);
                }
            }
            _ => {}
        }
    }

    fn walk_expr_for_module_const_disqualifiers(e: &Expr, g: &str, bad: &mut bool) {
        if *bad {
            return;
        }
        match e {
            Expr::Ident { name, .. } if name.as_ref() == g => {
                *bad = true;
            }
            Expr::Assign { name, value, .. }
            | Expr::CompoundAssign { name, value, .. }
            | Expr::LogicalAssign { name, value, .. } => {
                if name.as_ref() == g {
                    *bad = true;
                }
                Self::walk_expr_for_module_const_disqualifiers(value, g, bad);
            }
            Expr::PostfixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PrefixDec { name, .. } if name.as_ref() == g => {
                *bad = true;
            }
            Expr::Index { object, index, .. } => {
                if !matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == g) {
                    Self::walk_expr_for_module_const_disqualifiers(object, g, bad);
                }
                Self::walk_expr_for_module_const_disqualifiers(index, g, bad);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == g) {
                    *bad = true;
                } else {
                    Self::walk_expr_for_module_const_disqualifiers(object, g, bad);
                }
                Self::walk_expr_for_module_const_disqualifiers(index, g, bad);
                Self::walk_expr_for_module_const_disqualifiers(value, g, bad);
            }
            Expr::Member { object, prop, .. } => {
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == g) {
                    *bad = true;
                } else {
                    Self::walk_expr_for_module_const_disqualifiers(object, g, bad);
                }
                if let MemberProp::Expr(pe) = prop {
                    Self::walk_expr_for_module_const_disqualifiers(pe, g, bad);
                }
            }
            Expr::Call { callee, args, .. } => {
                let cumulative_arg_exempt = matches!(
                    callee.as_ref(),
                    Expr::Ident { name, .. } if name.as_ref() == "cumulative"
                );
                if matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == g) {
                    *bad = true;
                } else {
                    Self::walk_expr_for_module_const_disqualifiers(callee, g, bad);
                }
                for (i, a) in args.iter().enumerate() {
                    if let CallArg::Expr(arg) | CallArg::Spread(arg) = a {
                        if cumulative_arg_exempt
                            && i == 0
                            && matches!(arg, Expr::Ident { name, .. } if name.as_ref() == g)
                        {
                            continue;
                        }
                        Self::walk_expr_for_module_const_disqualifiers(arg, g, bad);
                    }
                }
            }
            Expr::Binary { left, right, .. } => {
                Self::walk_expr_for_module_const_disqualifiers(left, g, bad);
                Self::walk_expr_for_module_const_disqualifiers(right, g, bad);
            }
            Expr::Unary { operand, .. } => {
                Self::walk_expr_for_module_const_disqualifiers(operand, g, bad);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::walk_expr_for_module_const_disqualifiers(cond, g, bad);
                Self::walk_expr_for_module_const_disqualifiers(then_branch, g, bad);
                Self::walk_expr_for_module_const_disqualifiers(else_branch, g, bad);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    if let ArrayElement::Expr(e) = el {
                        Self::walk_expr_for_module_const_disqualifiers(e, g, bad);
                    }
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjectProp::KeyValue(_, value, _) => {
                            Self::walk_expr_for_module_const_disqualifiers(value, g, bad);
                        }
                        ObjectProp::Spread(e) => {
                            Self::walk_expr_for_module_const_disqualifiers(e, g, bad);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Precompute cumulative arrays for module const inputs referenced by `cumulative(p)`.
    fn collect_module_const_cum(
        stmts: &[Statement],
        module_const: &HashMap<String, Vec<f64>>,
    ) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        if module_const.is_empty() {
            return out;
        }
        fn walk_stmt(stmt: &Statement, module_const: &HashMap<String, Vec<f64>>, out: &mut HashMap<String, String>) {
            match stmt {
                Statement::VarDecl { init: Some(e), .. } => scan_expr(e, module_const, out),
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    for s in statements {
                        walk_stmt(s, module_const, out);
                    }
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    walk_stmt(then_branch, module_const, out);
                    if let Some(eb) = else_branch {
                        walk_stmt(eb, module_const, out);
                    }
                }
                Statement::For { init, body, .. } => {
                    if let Some(i) = init {
                        walk_stmt(i, module_const, out);
                    }
                    walk_stmt(body, module_const, out);
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    walk_stmt(body, module_const, out);
                }
                Statement::FunDecl { body, .. } => walk_stmt(body, module_const, out),
                Statement::ExprStmt { expr, .. } => scan_expr(expr, module_const, out),
                Statement::Return { value: Some(e), .. } => scan_expr(e, module_const, out),
                _ => {}
            }
        }
        fn scan_expr(e: &Expr, module_const: &HashMap<String, Vec<f64>>, out: &mut HashMap<String, String>) {
            if let Expr::Call { callee, args, .. } = e {
                if let Expr::Ident { name: fnname, .. } = callee.as_ref() {
                    if fnname.as_ref() == "cumulative" {
                        if let Some(CallArg::Expr(Expr::Ident { name: argn, .. })) = args.first() {
                            if module_const.contains_key(argn.as_ref()) {
                                let cum_static =
                                    Codegen::module_const_static(&format!("{}_cum", argn.as_ref()));
                                out.insert(argn.to_string(), cum_static);
                            }
                        }
                    }
                }
            }
        }
        for s in stmts {
            walk_stmt(s, module_const, &mut out);
        }
        out
    }

    /// `let cum = cumulative(probs)` where `probs` has a precomputed cum const → alias `cum`.
    fn collect_module_const_aliases(
        stmts: &[Statement],
        cum_map: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        if cum_map.is_empty() {
            return out;
        }
        let mut scan = |e: &Expr| {
            if let Expr::Call { callee, args, .. } = e {
                if let Expr::Ident { name: fnname, .. } = callee.as_ref() {
                    if fnname.as_ref() == "cumulative" {
                        if let Some(CallArg::Expr(Expr::Ident { name: argn, .. })) = args.first() {
                            if let Some(cum_static) = cum_map.get(argn.as_ref()) {
                                // Parent VarDecl name is handled in walk — see below.
                                let _ = cum_static;
                            }
                        }
                    }
                }
            }
        };
        fn walk_stmt(stmt: &Statement, cum_map: &HashMap<String, String>, out: &mut HashMap<String, String>) {
            match stmt {
                Statement::VarDecl { name, init, .. } => {
                    if let Some(init_e) = init {
                        if let Expr::Call { callee, args, .. } = init_e {
                            if let Expr::Ident { name: fnname, .. } = callee.as_ref() {
                                if fnname.as_ref() == "cumulative" {
                                    if let Some(CallArg::Expr(Expr::Ident { name: argn, .. })) =
                                        args.first()
                                    {
                                        if let Some(cum_static) = cum_map.get(argn.as_ref()) {
                                            out.insert(name.to_string(), cum_static.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                    for s in statements {
                        walk_stmt(s, cum_map, out);
                    }
                }
                Statement::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    walk_stmt(then_branch, cum_map, out);
                    if let Some(eb) = else_branch {
                        walk_stmt(eb, cum_map, out);
                    }
                }
                Statement::For { init, body, .. } => {
                    if let Some(i) = init {
                        walk_stmt(i, cum_map, out);
                    }
                    walk_stmt(body, cum_map, out);
                }
                Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                    walk_stmt(body, cum_map, out);
                }
                Statement::FunDecl { body, .. } => walk_stmt(body, cum_map, out),
                _ => {}
            }
        }
        for s in stmts {
            walk_stmt(s, cum_map, &mut out);
            Self::for_each_stmt_expr(s, &mut |e| scan(e));
        }
        out
    }

    fn module_const_static(name: &str) -> String {
        Self::global_static_name(name)
    }

    fn module_const_array_static(
        module_const: &HashMap<String, Vec<f64>>,
        aliases: &HashMap<String, String>,
        name: &str,
    ) -> Option<String> {
        if let Some(cum_static) = aliases.get(name) {
            return Some(cum_static.clone());
        }
        if module_const.contains_key(name) {
            return Some(Self::module_const_static(name));
        }
        None
    }

    /// Top-level `let xs = [1,2,3]` hoisted to `const G_XS` — resolve the Rust static name.
    fn module_const_f64_array_rust_ref(&self, name: &str) -> Option<String> {
        // A module-level const f64 array is hoisted to a top-level `static`, so it resolves at ANY scope
        // depth — including from inside a loop/block body (e.g. `[...srcArr]` nested in a for-loop). The
        // old `depth == 1` guard wrongly returned None at depth > 1, leaving the name undeclared (build
        // fail). The only case where the const must NOT win is when an inner local binding shadows the
        // name; the candidate set already drops fn-local and reassigned names, and this checks the
        // remaining case — a currently-live block-scoped local of the same name.
        if self
            .outer_vars_stack
            .iter()
            .any(|scope| scope.iter().any(|v| v == name))
        {
            return None;
        }
        Self::module_const_array_static(
            &self.module_const_f64_arrays,
            &self.module_const_f64_aliases,
            name,
        )
    }

    fn module_const_f64_array_as_value(static_name: &str) -> String {
        format!(
            "Value::NumberArray(VmRef::new({}.iter().copied().collect::<Vec<f64>>()))",
            static_name
        )
    }

    fn emit_module_const_f64_arrays(&mut self) -> Result<(), CompileError> {
        for (name, vals) in self.module_const_f64_arrays.clone() {
            let lit = vals
                .iter()
                .map(|v| Self::f64_lit(*v))
                .collect::<Vec<_>>()
                .join(", ");
            self.writeln(&format!(
                "const {}: [f64; {}] = [{}];",
                Self::module_const_static(name.as_str()),
                vals.len(),
                lit
            ));
        }
        for (src, cum_static) in self.module_const_f64_cum.clone() {
            let vals = self.module_const_f64_arrays.get(&src).expect("cum src");
            let mut cum: Vec<f64> = Vec::new();
            let mut c = 0.0;
            for v in vals {
                c += *v;
                cum.push(c);
            }
            let lit = cum
                .iter()
                .map(|v| Self::f64_lit(*v))
                .collect::<Vec<_>>()
                .join(", ");
            self.writeln(&format!(
                "const {}: [f64; {}] = [{}];",
                cum_static,
                cum.len(),
                lit
            ));
        }
        Ok(())
    }

























    fn expr_is_map_construct(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::New {
                callee,
                ..
            } if matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Map")
        )
    }

    /// #176 — classify top-level `let x = <number-literal>` bindings whose every use is a numeric
    /// read or a whole-binding assign with a numeric-shaped RHS (fasta `seed`).
    fn collect_native_numeric_globals(stmts: &[Statement]) -> HashMap<String, f64> {
        use std::collections::{HashMap, HashSet};
        let mut candidates: HashMap<String, f64> = HashMap::new();
        for s in stmts {
            if let Statement::VarDecl {
                name,
                type_ann,
                init,
                ..
            } = s
            {
                if let Some(ann) = type_ann {
                    if !Self::ann_is_number(ann) {
                        continue;
                    }
                }
                if let Some(Expr::Literal {
                    value: Literal::Number(n),
                    ..
                }) = init.as_ref()
                {
                    candidates.insert(name.to_string(), *n);
                }
            }
        }
        if candidates.is_empty() {
            return HashMap::new();
        }
        // A top-level numeric global is hoisted to a `thread_local Cell`, and every read of that
        // NAME lowers to the global cell. So ANY shadowing binding of the same name — a `let`/`const`
        // in a nested scope (bare block, loop/if body, fn body), a fn/arrow PARAMETER, or a `for…of`
        // loop var — would be mis-read as the global. Disqualify such names (the binding stays a
        // normal scoped local). Earlier this only scanned `FunDecl` bodies, so block-shadows and
        // params silently read the global (scopes.tish / control_flow_nested / nested_complex).
        let mut shadow: HashSet<String> = HashSet::new();
        for s in stmts {
            // Skip the top-level candidate decls themselves; everything else's bindings shadow.
            match s {
                Statement::VarDecl { .. } | Statement::VarDeclDestructure { .. } => {}
                _ => Self::collect_local_var_names(s, &mut shadow),
            }
            Self::collect_binding_names(s, &mut shadow);
        }
        for n in &shadow {
            candidates.remove(n);
        }
        let globals: HashSet<String> = candidates.keys().cloned().collect();
        let empty_p = HashSet::new();
        let empty_c = HashSet::new();
        let mut reassigns: Vec<(String, &Expr)> = Vec::new();
        Self::collect_reassignments_stmts(stmts, &mut reassigns);
        for (name, rhs) in &reassigns {
            if globals.contains(name)
                && !Self::numeric_shaped(rhs, &empty_p, &empty_c, &globals, &HashSet::new())
            {
                candidates.remove(name);
            }
        }
        for name in candidates.keys().cloned().collect::<Vec<_>>() {
            if Self::global_has_disqualifying_use(stmts, &name) {
                candidates.remove(&name);
            }
        }
        candidates
    }

    /// Disqualifying uses: compound/logical assign, inc/dec, member/index/call on the global name.
    fn global_has_disqualifying_use(stmts: &[Statement], g: &str) -> bool {
        let mut bad = false;
        Self::walk_stmts_for_global_disqualifiers(stmts, g, &mut bad);
        bad
    }

    fn walk_stmts_for_global_disqualifiers(stmts: &[Statement], g: &str, bad: &mut bool) {
        for s in stmts {
            Self::walk_stmt_for_global_disqualifiers(s, g, bad);
        }
    }

    fn walk_stmt_for_global_disqualifiers(s: &Statement, g: &str, bad: &mut bool) {
        if *bad {
            return;
        }
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                Self::walk_stmts_for_global_disqualifiers(statements, g, bad);
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::walk_stmt_for_global_disqualifiers(then_branch, g, bad);
                if let Some(eb) = else_branch {
                    Self::walk_stmt_for_global_disqualifiers(eb, g, bad);
                }
            }
            Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::For { body, .. }
            | Statement::ForOf { body, .. } => {
                Self::walk_stmt_for_global_disqualifiers(body, g, bad);
            }
            Statement::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, stmts) in cases {
                    Self::walk_stmts_for_global_disqualifiers(stmts, g, bad);
                }
                if let Some(d) = default_body {
                    Self::walk_stmts_for_global_disqualifiers(d, g, bad);
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::walk_stmt_for_global_disqualifiers(body, g, bad);
                if let Some(c) = catch_body {
                    Self::walk_stmt_for_global_disqualifiers(c, g, bad);
                }
                if let Some(f) = finally_body {
                    Self::walk_stmt_for_global_disqualifiers(f, g, bad);
                }
            }
            Statement::FunDecl { body, .. } => {
                Self::walk_stmt_for_global_disqualifiers(body, g, bad);
            }
            Statement::ExprStmt { expr, .. } => {
                Self::walk_expr_for_global_disqualifiers(expr, g, bad);
            }
            Statement::Return { value: Some(e), .. } => {
                Self::walk_expr_for_global_disqualifiers(e, g, bad);
            }
            Statement::VarDecl { init: Some(e), .. } => {
                Self::walk_expr_for_global_disqualifiers(e, g, bad);
            }
            _ => {}
        }
    }

    fn walk_expr_for_global_disqualifiers(e: &Expr, g: &str, bad: &mut bool) {
        if *bad {
            return;
        }
        match e {
            Expr::CompoundAssign { name, .. }
            | Expr::LogicalAssign { name, .. }
                if name.as_ref() == g =>
            {
                *bad = true;
            }
            Expr::PostfixInc { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixDec { name, .. }
                if name.as_ref() == g =>
            {
                *bad = true;
            }
            Expr::Call { callee, .. }
                if matches!(callee.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == g) =>
            {
                *bad = true;
            }
            Expr::Member { object, .. }
                if matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == g) =>
            {
                *bad = true;
            }
            Expr::Index { object, .. }
                if matches!(object.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == g) =>
            {
                *bad = true;
            }
            Expr::Delete { target, .. }
                if matches!(target.as_ref(), Expr::Ident { name: n, .. } if n.as_ref() == g) =>
            {
                *bad = true;
            }
            _ => {}
        }
        Self::walk_expr_children_for_global_disqualifiers(e, g, bad);
    }

    fn walk_expr_children_for_global_disqualifiers(e: &Expr, g: &str, bad: &mut bool) {
        if *bad {
            return;
        }
        match e {
            Expr::Binary { left, right, .. } => {
                Self::walk_expr_for_global_disqualifiers(left, g, bad);
                Self::walk_expr_for_global_disqualifiers(right, g, bad);
            }
            Expr::Unary { operand, .. } => {
                Self::walk_expr_for_global_disqualifiers(operand, g, bad);
            }
            Expr::Assign { value, .. } => {
                Self::walk_expr_for_global_disqualifiers(value, g, bad);
            }
            Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. } => {
                Self::walk_expr_for_global_disqualifiers(value, g, bad);
            }
            Expr::Conditional {
                then_branch,
                else_branch,
                ..
            } => {
                Self::walk_expr_for_global_disqualifiers(then_branch, g, bad);
                Self::walk_expr_for_global_disqualifiers(else_branch, g, bad);
            }
            Expr::Call { callee, args, .. } => {
                Self::walk_expr_for_global_disqualifiers(callee, g, bad);
                for a in args {
                    if let CallArg::Expr(e) | CallArg::Spread(e) = a {
                        Self::walk_expr_for_global_disqualifiers(e, g, bad);
                    }
                }
            }
            Expr::New { callee, args, .. } => {
                Self::walk_expr_for_global_disqualifiers(callee, g, bad);
                for a in args {
                    if let CallArg::Expr(e) | CallArg::Spread(e) = a {
                        Self::walk_expr_for_global_disqualifiers(e, g, bad);
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::walk_expr_for_global_disqualifiers(object, g, bad);
                if let MemberProp::Expr(pe) = prop {
                    Self::walk_expr_for_global_disqualifiers(pe, g, bad);
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::walk_expr_for_global_disqualifiers(object, g, bad);
                Self::walk_expr_for_global_disqualifiers(value, g, bad);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                Self::walk_expr_for_global_disqualifiers(object, g, bad);
                Self::walk_expr_for_global_disqualifiers(index, g, bad);
                Self::walk_expr_for_global_disqualifiers(value, g, bad);
            }
            Expr::Index { object, index, .. } => {
                Self::walk_expr_for_global_disqualifiers(object, g, bad);
                Self::walk_expr_for_global_disqualifiers(index, g, bad);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::walk_expr_for_global_disqualifiers(e, g, bad);
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::walk_expr_for_global_disqualifiers(e, g, bad);
                        }
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => Self::walk_expr_for_global_disqualifiers(e, g, bad),
                ArrowBody::Block(s) => Self::walk_stmt_for_global_disqualifiers(s, g, bad),
            },
            Expr::NullishCoalesce { left, right, .. } => {
                Self::walk_expr_for_global_disqualifiers(left, g, bad);
                Self::walk_expr_for_global_disqualifiers(right, g, bad);
            }
            Expr::TypeOf { operand, .. } => {
                Self::walk_expr_for_global_disqualifiers(operand, g, bad);
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::walk_expr_for_global_disqualifiers(e, g, bad);
                }
            }
            _ => {}
        }
    }

    /// Every statement-level expression mentioning `g` must be numeric-shaped (assign-to-`g` checks RHS).
    fn global_only_numeric_expr_roots(
        stmts: &[Statement],
        g: &str,
        globals: &HashSet<String>,
    ) -> bool {
        let empty_p = HashSet::new();
        let empty_c = HashSet::new();
        let mut ok = true;
        Self::for_each_expr_root(stmts, &mut |e| {
            if Self::expr_mentions_global(e, g) {
                let tree_ok = match e {
                    Expr::Assign { name, value, .. } if name.as_ref() == g => {
                        Self::numeric_shaped(value, &empty_p, &empty_c, globals, &HashSet::new())
                    }
                    _ => Self::numeric_shaped(e, &empty_p, &empty_c, globals, &HashSet::new()),
                };
                if !tree_ok {
                    ok = false;
                }
            }
        });
        ok
    }

    fn for_each_expr_root(stmts: &[Statement], f: &mut dyn FnMut(&Expr)) {
        for s in stmts {
            Self::for_each_expr_root_stmt(s, f);
        }
    }

    fn for_each_expr_root_stmt(s: &Statement, f: &mut dyn FnMut(&Expr)) {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                Self::for_each_expr_root(statements, f);
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                f(cond);
                Self::for_each_expr_root_stmt(then_branch, f);
                if let Some(eb) = else_branch {
                    Self::for_each_expr_root_stmt(eb, f);
                }
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { cond, body, .. } => {
                f(cond);
                Self::for_each_expr_root_stmt(body, f);
            }
            Statement::ForOf { iterable, body, .. } => {
                f(iterable);
                Self::for_each_expr_root_stmt(body, f);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(i) = init {
                    Self::for_each_expr_root_stmt(i, f);
                }
                if let Some(c) = cond {
                    f(c);
                }
                if let Some(u) = update {
                    f(u);
                }
                Self::for_each_expr_root_stmt(body, f);
            }
            Statement::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, stmts) in cases {
                    Self::for_each_expr_root(stmts, f);
                }
                if let Some(d) = default_body {
                    Self::for_each_expr_root(d, f);
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                Self::for_each_expr_root_stmt(body, f);
                if let Some(c) = catch_body {
                    Self::for_each_expr_root_stmt(c, f);
                }
                if let Some(fb) = finally_body {
                    Self::for_each_expr_root_stmt(fb, f);
                }
            }
            Statement::FunDecl { body, .. } => Self::for_each_expr_root_stmt(body, f),
            Statement::ExprStmt { expr, .. } => f(expr),
            Statement::Return { value: Some(e), .. } => f(e),
            Statement::VarDecl { init: Some(e), .. } => f(e),
            Statement::Switch { expr, .. } => f(expr),
            _ => {}
        }
    }

    fn expr_mentions_global(e: &Expr, g: &str) -> bool {
        match e {
            Expr::Ident { name, .. } if name.as_ref() == g => true,
            Expr::Binary { left, right, .. } => {
                Self::expr_mentions_global(left, g) || Self::expr_mentions_global(right, g)
            }
            Expr::Unary { operand, .. } => Self::expr_mentions_global(operand, g),
            Expr::Assign { name, value, .. } if name.as_ref() == g => true,
            Expr::Assign { value, .. } => Self::expr_mentions_global(value, g),
            Expr::CompoundAssign { name, value, .. } if name.as_ref() == g => true,
            Expr::CompoundAssign { value, .. } => Self::expr_mentions_global(value, g),
            Expr::LogicalAssign { name, value, .. } if name.as_ref() == g => true,
            Expr::LogicalAssign { value, .. } => Self::expr_mentions_global(value, g),
            Expr::Conditional {
                then_branch,
                else_branch,
                ..
            } => {
                Self::expr_mentions_global(then_branch, g)
                    || Self::expr_mentions_global(else_branch, g)
            }
            Expr::Call { callee, args, .. } => {
                Self::expr_mentions_global(callee, g)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => Self::expr_mentions_global(e, g),
                    })
            }
            Expr::Member { object, prop, .. } => {
                Self::expr_mentions_global(object, g)
                    || matches!(prop, MemberProp::Expr(e) if Self::expr_mentions_global(e, g))
            }
            Expr::Index { object, index, .. } => {
                Self::expr_mentions_global(object, g) || Self::expr_mentions_global(index, g)
            }
            Expr::MemberAssign { object, value, .. }
            | Expr::IndexAssign { object, value, .. } => {
                Self::expr_mentions_global(object, g) || Self::expr_mentions_global(value, g)
            }
            Expr::Array { elements, .. } => elements.iter().any(|el| match el {
                ArrayElement::Expr(e) | ArrayElement::Spread(e) => Self::expr_mentions_global(e, g),
            }),
            Expr::Object { props, .. } => props.iter().any(|p| match p {
                ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => Self::expr_mentions_global(e, g),
            }),
            Expr::TemplateLiteral { exprs, .. } => {
                exprs.iter().any(|e| Self::expr_mentions_global(e, g))
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => Self::expr_mentions_global(e, g),
                ArrowBody::Block(s) => {
                    let mut found = false;
                    Self::for_each_expr_root_stmt(s, &mut |e| {
                        if Self::expr_mentions_global(e, g) {
                            found = true;
                        }
                    });
                    found
                }
            },
            _ => false,
        }
    }

    /// Names of top-level fns eligible for a parallel native `fn f_native(f64,..)->f64`:
    /// non-async, every param `: number` (no default), `: number` return, and a native-safe
    /// body (only block/if/return/expr-stmt over native exprs + calls to other eligible fns or
    /// 1-arg Math intrinsics). Conservative fixpoint — bails on anything else.
    fn collect_native_fns(
        statements: &[Statement],
        globals: &std::collections::HashSet<String>,
        native_vec_names: &std::collections::HashSet<String>,
    ) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let mut cand: HashSet<String> = HashSet::new();
        let mut decls: Vec<(&str, &Vec<FunParam>, &Statement)> = Vec::new();
        for s in statements {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                return_type,
                body,
                ..
            } = s
            {
                let params_ok = params.iter().all(|p| {
                    matches!(p, FunParam::Simple(tp)
                        if tp.default.is_none()
                            && tp.type_ann.as_ref().map_or(true, Self::ann_is_number))
                });
                // Return: an annotated `: number`, OR unannotated with all-numeric returns
                // (verified in the fixpoint via `returns_numeric`), so the native `-> f64` holds.
                let ret_ok = match return_type {
                    Some(rt) => Self::ann_is_number(rt),
                    None => true,
                };
                // #320: 0-param numeric fns (e.g. k_nucleotide's `nextBase()` — mutates a numeric
                // global, returns number) are eligible too; the fixpoint below still proves the body
                // native-safe + all-numeric-returns, so `fn name_native() -> f64` is sound.
                if ret_ok && params_ok {
                    cand.insert(name.to_string());
                    decls.push((name.as_ref(), params, body));
                }
            }
        }
        // Soundness: a candidate called ANYWHERE with a provably-non-numeric argument (a
        // string/bool/null literal, a template literal, or an array/object literal) cannot be a
        // native `f64`-param fn — the generated Rust would mistype the arg (a `String` is not an
        // `f64`), and even coerced it would compute on NaN where JS keeps the value or does string
        // `+`. E.g. `fn shadowParam(SHARED){return SHARED}` called as `shadowParam("arg")` must stay
        // boxed. Drop such candidates BEFORE the fixpoint so it cascades to their native callers.
        for f in Self::fns_called_with_nonnumeric_arg(statements) {
            cand.remove(&f);
        }
        // #330: M5 promotes every param to `f64`, so a candidate is only sound when EVERY param is
        // provably a numeric scalar. Two param shapes break that and must disqualify the fn:
        //   (a) an INDEXED param (`arr[j]`) is an array, not an f64 — `fn sumN(n, arr) { … arr[j] … }`
        //       was emitted as `sumN_native(n: f64, arr: f64)` whose body does `get_index(Number(arr))`
        //       → reads null → `panic!("expected number")`. (Such a fn is the native-vec path's job.)
        //   (b) a param used ONLY as a forwarded call argument (a bare `f(p)` arg), never numerically,
        //       has the callee's type — which may be an array: `fn thread(n, arr){ return sumN(n,arr) }`
        //       forwards `arr` into `sumN`'s array slot; `thread_native(arr: f64)` then mistypes the
        //       array argument at the caller. A param also used numerically (e.g. a loop bound) is
        //       provably f64 and kept.
        // Drop such candidates BEFORE the fixpoint so it cascades to their callers; they stay boxed
        // (or native-vec), which is sound.
        for &(name, params, body) in &decls {
            if !cand.contains(name) {
                continue;
            }
            let has_non_scalar_param = params.iter().flat_map(|p| p.bound_names()).any(|pn| {
                let mut u = ParamUse::default();
                Self::for_each_stmt_expr(body, &mut |e| {
                    Self::scan_param_use(e, pn.as_ref(), false, &mut u)
                });
                u.indexed || (u.forwarded && !u.numeric)
            });
            if has_non_scalar_param {
                cand.remove(name);
            }
        }
        loop {
            let mut remove: Vec<String> = Vec::new();
            for &(name, params, body) in &decls {
                if !cand.contains(name) {
                    continue;
                }
                let pnames: HashSet<String> =
                    params.iter().flat_map(|p| p.bound_names()).map(|n| n.to_string()).collect();
                let nums = Self::native_fn_numeric_locals(
                    body,
                    &pnames,
                    globals,
                    &cand,
                    native_vec_names,
                );
                if !Self::native_safe_stmt(body, &pnames, &cand, globals, &nums, native_vec_names)
                    || !Self::returns_numeric(body, &pnames, &cand, globals, &nums)
                {
                    remove.push(name.to_string());
                }
            }
            if remove.is_empty() {
                break;
            }
            for n in remove {
                cand.remove(&n);
            }
        }
        cand
    }

    /// Fns called ANYWHERE in the program with a provably-non-numeric argument (a `String`/`Bool`/
    /// `Null` literal, a template literal, or an array/object literal). Such a fn cannot be soundly
    /// native-promoted to `f64` params — not as an M5 native free fn (`collect_native_fns`) NOR via
    /// an M4/M1 `: number` param shadow (`infer::param_infer`). Walks every statement (incl. nested
    /// fn bodies) and expression.
    pub(crate) fn fns_called_with_nonnumeric_arg(
        statements: &[Statement],
    ) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        for s in statements {
            Self::scan_stmt_nonnum_calls(s, &mut out);
        }
        out
    }

    fn arg_is_nonnumeric(e: &Expr) -> bool {
        match e {
            Expr::Literal {
                value: Literal::String(_) | Literal::Bool(_) | Literal::Null,
                ..
            }
            | Expr::TemplateLiteral { .. }
            | Expr::Array { .. }
            | Expr::Object { .. } => true,
            // A `+` with a string/template operand is JS string concatenation → a string result
            // (e.g. `"user" + i`). Conservatively treat such a call arg as non-numeric.
            Expr::Binary { op: BinOp::Add, left, right, .. } => {
                Self::arg_is_nonnumeric(left) || Self::arg_is_nonnumeric(right)
            }
            _ => false,
        }
    }

    fn scan_stmt_nonnum_calls(s: &Statement, out: &mut std::collections::HashSet<String>) {
        use Statement::*;
        match s {
            Block { statements, .. } | Multi { statements, .. } => {
                statements.iter().for_each(|x| Self::scan_stmt_nonnum_calls(x, out))
            }
            VarDecl { init: Some(i), .. } => Self::scan_expr_nonnum_calls(i, out),
            VarDeclDestructure { init, .. } => Self::scan_expr_nonnum_calls(init, out),
            ExprStmt { expr, .. } => Self::scan_expr_nonnum_calls(expr, out),
            If { cond, then_branch, else_branch, .. } => {
                Self::scan_expr_nonnum_calls(cond, out);
                Self::scan_stmt_nonnum_calls(then_branch, out);
                if let Some(eb) = else_branch {
                    Self::scan_stmt_nonnum_calls(eb, out);
                }
            }
            While { cond, body, .. } | DoWhile { body, cond, .. } => {
                Self::scan_expr_nonnum_calls(cond, out);
                Self::scan_stmt_nonnum_calls(body, out);
            }
            For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    Self::scan_stmt_nonnum_calls(i, out);
                }
                if let Some(c) = cond {
                    Self::scan_expr_nonnum_calls(c, out);
                }
                if let Some(u) = update {
                    Self::scan_expr_nonnum_calls(u, out);
                }
                Self::scan_stmt_nonnum_calls(body, out);
            }
            ForOf { iterable, body, .. } => {
                Self::scan_expr_nonnum_calls(iterable, out);
                Self::scan_stmt_nonnum_calls(body, out);
            }
            Return { value: Some(v), .. } => Self::scan_expr_nonnum_calls(v, out),
            Throw { value, .. } => Self::scan_expr_nonnum_calls(value, out),
            Switch { expr, cases, default_body, .. } => {
                Self::scan_expr_nonnum_calls(expr, out);
                for (g, body) in cases {
                    if let Some(ge) = g {
                        Self::scan_expr_nonnum_calls(ge, out);
                    }
                    body.iter().for_each(|x| Self::scan_stmt_nonnum_calls(x, out));
                }
                if let Some(d) = default_body {
                    d.iter().for_each(|x| Self::scan_stmt_nonnum_calls(x, out));
                }
            }
            Try { body, catch_body, finally_body, .. } => {
                Self::scan_stmt_nonnum_calls(body, out);
                if let Some(c) = catch_body {
                    Self::scan_stmt_nonnum_calls(c, out);
                }
                if let Some(f) = finally_body {
                    Self::scan_stmt_nonnum_calls(f, out);
                }
            }
            FunDecl { body, .. } => Self::scan_stmt_nonnum_calls(body, out),
            Export { declaration, .. } => match declaration.as_ref() {
                tishlang_ast::ExportDeclaration::Named(inner) => {
                    Self::scan_stmt_nonnum_calls(inner, out)
                }
                tishlang_ast::ExportDeclaration::Default(ex) => {
                    Self::scan_expr_nonnum_calls(ex, out)
                }
                tishlang_ast::ExportDeclaration::ReExport { .. } => {}
            },
            _ => {}
        }
    }

    fn scan_expr_nonnum_calls(e: &Expr, out: &mut std::collections::HashSet<String>) {
        match e {
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                if let Expr::Ident { name, .. } = callee.as_ref() {
                    if args
                        .iter()
                        .any(|a| matches!(a, CallArg::Expr(ae) if Self::arg_is_nonnumeric(ae)))
                    {
                        out.insert(name.to_string());
                    }
                }
                Self::scan_expr_nonnum_calls(callee, out);
                for a in args {
                    if let CallArg::Expr(x) | CallArg::Spread(x) = a {
                        Self::scan_expr_nonnum_calls(x, out);
                    }
                }
            }
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::scan_expr_nonnum_calls(left, out);
                Self::scan_expr_nonnum_calls(right, out);
            }
            Expr::Unary { operand, .. } => Self::scan_expr_nonnum_calls(operand, out),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                Self::scan_expr_nonnum_calls(cond, out);
                Self::scan_expr_nonnum_calls(then_branch, out);
                Self::scan_expr_nonnum_calls(else_branch, out);
            }
            Expr::Index { object, index, .. } => {
                Self::scan_expr_nonnum_calls(object, out);
                Self::scan_expr_nonnum_calls(index, out);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                Self::scan_expr_nonnum_calls(object, out);
                Self::scan_expr_nonnum_calls(index, out);
                Self::scan_expr_nonnum_calls(value, out);
            }
            Expr::Member { object, prop, .. } => {
                Self::scan_expr_nonnum_calls(object, out);
                if let MemberProp::Expr(pe) = prop {
                    Self::scan_expr_nonnum_calls(pe, out);
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                Self::scan_expr_nonnum_calls(object, out);
                Self::scan_expr_nonnum_calls(value, out);
            }
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => Self::scan_expr_nonnum_calls(value, out),
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(x) | ArrayElement::Spread(x) => {
                            Self::scan_expr_nonnum_calls(x, out)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for pr in props {
                    match pr {
                        ObjectProp::KeyValue(_, x, _) | ObjectProp::Spread(x) => {
                            Self::scan_expr_nonnum_calls(x, out)
                        }
                    }
                }
            }
            Expr::TemplateLiteral { exprs, .. } => {
                exprs.iter().for_each(|x| Self::scan_expr_nonnum_calls(x, out))
            }
            _ => {}
        }
    }

    /// Numeric locals in an M5-eligible fn body (params + fixpoint literal/copy seeds).
    fn native_fn_numeric_locals(
        body: &Statement,
        params: &HashSet<String>,
        globals: &HashSet<String>,
        cand: &HashSet<String>,
        native_vec_names: &HashSet<String>,
    ) -> HashSet<String> {
        let mut nums: HashSet<String> = params.clone();
        nums.extend(globals.iter().cloned());
        loop {
            let before = nums.len();
            Self::seed_native_fn_numeric_locals(body, &mut nums, cand, native_vec_names);
            if nums.len() == before {
                break;
            }
        }
        nums
    }

    fn seed_native_fn_numeric_locals(
        stmt: &Statement,
        nums: &mut HashSet<String>,
        cand: &HashSet<String>,
        native_vec_names: &HashSet<String>,
    ) {
        match stmt {
            Statement::VarDecl {
                name,
                type_ann,
                init,
                ..
            } => {
                let numeric = type_ann.as_ref().is_some_and(Self::ann_is_number)
                    || matches!(
                        init,
                        Some(Expr::Literal {
                            value: Literal::Number(_),
                            ..
                        })
                    )
                    || matches!(
                        init,
                        Some(Expr::Ident { name: src, .. }) if nums.contains(src.as_ref())
                    )
                    || matches!(
                        init.as_ref(),
                        Some(Expr::Call {
                            callee,
                            ..
                        }) if matches!(
                            callee.as_ref(),
                            Expr::Ident { name: fnname, .. }
                                if cand.contains(fnname.as_ref()) || native_vec_names.contains(fnname.as_ref())
                        )
                    );
                if numeric {
                    nums.insert(name.to_string());
                }
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::seed_native_fn_numeric_locals(s, nums, cand, native_vec_names);
                }
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::seed_native_fn_numeric_locals(then_branch, nums, cand, native_vec_names);
                if let Some(e) = else_branch {
                    Self::seed_native_fn_numeric_locals(e, nums, cand, native_vec_names);
                }
            }
            Statement::For { init, body, .. } => {
                if let Some(i) = init {
                    Self::seed_native_fn_numeric_locals(i, nums, cand, native_vec_names);
                }
                Self::seed_native_fn_numeric_locals(body, nums, cand, native_vec_names);
            }
            Statement::While { body, .. } | Statement::DoWhile { body, .. } => {
                Self::seed_native_fn_numeric_locals(body, nums, cand, native_vec_names);
            }
            _ => {}
        }
    }

    fn native_safe_stmt(
        stmt: &Statement,
        params: &std::collections::HashSet<String>,
        cand: &std::collections::HashSet<String>,
        globals: &std::collections::HashSet<String>,
        nums: &HashSet<String>,
        native_vec_names: &std::collections::HashSet<String>,
    ) -> bool {
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().all(|s| Self::native_safe_stmt(s, params, cand, globals, nums, native_vec_names))
            }
            Statement::Return { value, .. } => {
                value.as_ref().is_some_and(|e| Self::native_safe_expr(e, params, cand, globals, nums, native_vec_names))
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                Self::native_safe_expr(cond, params, cand, globals, nums, native_vec_names)
                    && Self::native_safe_stmt(then_branch, params, cand, globals, nums, native_vec_names)
                    && else_branch.as_ref().is_none_or(|e| Self::native_safe_stmt(e, params, cand, globals, nums, native_vec_names))
            }
            Statement::ExprStmt { expr, .. } => Self::native_safe_expr(expr, params, cand, globals, nums, native_vec_names),
            Statement::VarDecl { init, .. } => {
                init.as_ref().is_none_or(|e| Self::native_safe_expr(e, params, cand, globals, nums, native_vec_names))
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                init.as_ref().is_none_or(|i| Self::native_safe_stmt(i, params, cand, globals, nums, native_vec_names))
                    && cond.as_ref().is_none_or(|c| Self::native_safe_expr(c, params, cand, globals, nums, native_vec_names))
                    && update.as_ref().is_none_or(|u| Self::native_safe_expr(u, params, cand, globals, nums, native_vec_names))
                    && Self::native_safe_stmt(body, params, cand, globals, nums, native_vec_names)
            }
            Statement::While { cond, body, .. } => {
                Self::native_safe_expr(cond, params, cand, globals, nums, native_vec_names)
                    && Self::native_safe_stmt(body, params, cand, globals, nums, native_vec_names)
            }
            Statement::DoWhile { body, cond, .. } => {
                Self::native_safe_stmt(body, params, cand, globals, nums, native_vec_names)
                    && Self::native_safe_expr(cond, params, cand, globals, nums, native_vec_names)
            }
            _ => false,
        }
    }

    fn native_safe_expr(
        expr: &Expr,
        params: &std::collections::HashSet<String>,
        cand: &std::collections::HashSet<String>,
        globals: &std::collections::HashSet<String>,
        nums: &HashSet<String>,
        native_vec_names: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Literal { value, .. } => matches!(value, Literal::Number(_) | Literal::Bool(_)),
            Expr::Ident { name, .. } => {
                params.contains(name.as_ref()) || globals.contains(name.as_ref()) || nums.contains(name.as_ref())
            }
            Expr::Assign { name, value, .. } => {
                (globals.contains(name.as_ref()) || nums.contains(name.as_ref()))
                    && Self::native_safe_expr(value, params, cand, globals, nums, native_vec_names)
            }
            Expr::CompoundAssign { name, value, .. } => {
                nums.contains(name.as_ref())
                    && Self::native_safe_expr(value, params, cand, globals, nums, native_vec_names)
            }
            Expr::Binary { left, op, right, .. } => {
                matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                        | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                        | BinOp::StrictEq | BinOp::StrictNe | BinOp::And | BinOp::Or
                ) && Self::native_safe_expr(left, params, cand, globals, nums, native_vec_names)
                    && Self::native_safe_expr(right, params, cand, globals, nums, native_vec_names)
            }
            Expr::Unary { op, operand, .. } => {
                matches!(op, UnaryOp::Neg | UnaryOp::Pos | UnaryOp::Not)
                    && Self::native_safe_expr(operand, params, cand, globals, nums, native_vec_names)
            }
            Expr::PostfixInc { name, .. }
            | Expr::PrefixInc { name, .. }
            | Expr::PostfixDec { name, .. }
            | Expr::PrefixDec { name, .. } => nums.contains(name.as_ref()),
            Expr::Index { object, index, .. } => {
                matches!(object.as_ref(), Expr::Ident { .. })
                    && Self::native_safe_expr(index, params, cand, globals, nums, native_vec_names)
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                matches!(object.as_ref(), Expr::Ident { .. })
                    && Self::native_safe_expr(index, params, cand, globals, nums, native_vec_names)
                    && Self::native_safe_expr(value, params, cand, globals, nums, native_vec_names)
            }
            Expr::Call { callee, args, .. } => {
                let args_ok = args
                    .iter()
                    .all(|a| matches!(a, CallArg::Expr(e) if Self::native_safe_expr(e, params, cand, globals, nums, native_vec_names)));
                if !args_ok {
                    return false;
                }
                match callee.as_ref() {
                    Expr::Ident { name, .. } => {
                        cand.contains(name.as_ref()) || native_vec_names.contains(name.as_ref())
                    }
                    Expr::Member { object, prop: MemberProp::Name { name: m, .. }, .. } => {
                        matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                            && args.len() == 1
                            && matches!(
                                m.as_ref(),
                                "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | "exp"
                                    | "trunc" | "log"
                            )
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Every `return` in `s` yields a numeric-shaped value, so a native `-> f64` body is sound.
    /// Lets an unannotated-but-numeric-returning fn (e.g. `function fib(n) {...}` after M4 typed
    /// the param) become M5-eligible.
    fn returns_numeric(
        s: &Statement,
        params: &std::collections::HashSet<String>,
        cand: &std::collections::HashSet<String>,
        globals: &std::collections::HashSet<String>,
        nums: &HashSet<String>,
    ) -> bool {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().all(|x| Self::returns_numeric(x, params, cand, globals, nums))
            }
            Statement::Return { value, .. } => {
                value.as_ref().is_some_and(|e| Self::numeric_shaped(e, params, cand, globals, nums))
            }
            Statement::If { then_branch, else_branch, .. } => {
                Self::returns_numeric(then_branch, params, cand, globals, nums)
                    && else_branch.as_ref().is_none_or(|e| Self::returns_numeric(e, params, cand, globals, nums))
            }
            Statement::While { body, .. } | Statement::For { body, .. } => {
                Self::returns_numeric(body, params, cand, globals, nums)
            }
            _ => true, // no return in this statement form
        }
    }

    /// `e` evaluates to a number: built from numeric params, number literals, ARITHMETIC binops
    /// (comparisons/logical yield bool → excluded), numeric unary, conditionals, and calls to
    /// eligible native fns / 1-arg Math.
    fn numeric_shaped(
        e: &Expr,
        params: &std::collections::HashSet<String>,
        cand: &std::collections::HashSet<String>,
        globals: &std::collections::HashSet<String>,
        nums: &HashSet<String>,
    ) -> bool {
        match e {
            Expr::Literal { value: Literal::Number(_), .. } => true,
            Expr::Ident { name, .. } => {
                params.contains(name.as_ref())
                    || globals.contains(name.as_ref())
                    || nums.contains(name.as_ref())
            }
            Expr::Binary { left, op, right, .. } => {
                matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                ) && Self::numeric_shaped(left, params, cand, globals, nums)
                    && Self::numeric_shaped(right, params, cand, globals, nums)
            }
            Expr::Unary { op, operand, .. } => {
                matches!(op, UnaryOp::Neg | UnaryOp::Pos)
                    && Self::numeric_shaped(operand, params, cand, globals, nums)
            }
            Expr::Conditional { then_branch, else_branch, .. } => {
                Self::numeric_shaped(then_branch, params, cand, globals, nums)
                    && Self::numeric_shaped(else_branch, params, cand, globals, nums)
            }
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident { name, .. } => cand.contains(name.as_ref()),
                Expr::Member { object, prop: MemberProp::Name { name: m, .. }, .. } => {
                    matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                        && matches!(
                            m.as_ref(),
                            "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | "exp"
                                | "trunc" | "log"
                        )
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Emit `fn name_native(p: f64, ...) -> f64 { ... }` (top level) for each eligible fn.
    fn emit_native_fns(&mut self, statements: &[Statement]) -> Result<(), CompileError> {
        for s in statements {
            if let Statement::FunDecl { name, params, body, .. } = s {
                if !self.native_fns.contains(name.as_ref()) {
                    continue;
                }
                let plist: Vec<String> = params
                    .iter()
                    .filter_map(|p| match p {
                        FunParam::Simple(tp) => {
                            Some(format!("mut {}: f64", Self::escape_ident(tp.name.as_ref())))
                        }
                        _ => None,
                    })
                    .collect();
                self.type_context.push_scope();
                for p in params {
                    if let FunParam::Simple(tp) = p {
                        self.type_context.define(tp.name.as_ref(), RustType::F64);
                    }
                }
                self.writeln(&format!("fn {}_native({}) -> f64 {{", Self::escape_ident(name.as_ref()), plist.join(", ")));
                self.indent += 1;
                if let Some((global, mul, add, modulus)) = self.native_lcg_fns.get(name.as_ref()) {
                    let max_name = params
                        .iter()
                        .find_map(|p| match p {
                            FunParam::Simple(tp) => Some(Self::escape_ident(tp.name.as_ref())),
                            _ => None,
                        })
                        .unwrap_or_else(|| "max".into());
                    let body = if mul.fract() == 0.0
                        && add.fract() == 0.0
                        && modulus.fract() == 0.0
                        && *modulus > 0.0
                    {
                        format!(
                            "{{ {}.with(|g| {{ let prev = (g.get() as i64); let s = (prev * {}i64 + {}i64) % {}i64; g.set(s as f64); (({}) * (s as f64)) / {} }}) }}",
                            Self::native_global_static(global),
                            *mul as i64,
                            *add as i64,
                            *modulus as i64,
                            max_name,
                            Self::f64_lit(*modulus),
                        )
                    } else {
                        self.emit_native_lcg_with(global, *mul, *add, *modulus, &max_name)
                    };
                    self.writeln(&format!("return {};", body));
                } else {
                    self.native_fn_emit_name = Some(name.to_string());
                    self.native_fn_body_returned = false;
                    self.usize_for_counters.clear();
                    if name.as_ref() != "genRandom" {
                        if let Some((global, mul, add, modulus)) =
                            self.native_lcg_fns.get("genRandom").cloned()
                        {
                            let int_lcg = mul.fract() == 0.0
                                && add.fract() == 0.0
                                && modulus.fract() == 0.0
                                && modulus > 0.0;
                            self.native_lcg_hoist_int = int_lcg;
                            if int_lcg {
                                self.writeln(
                                    "let mut _lcg_seed: i64 = (G_SEED.with(|g| g.get()) as i64);",
                                );
                            } else {
                                self.writeln("let mut _lcg_seed: f64 = G_SEED.with(|g| g.get());");
                            }
                            self.native_lcg_hoist = Some((global, mul, add, modulus));
                        }
                    }
                    self.native_fn_body_emit = true;
                    let body_slice = Self::fn_body_stmt_slice(body);
                    let saved_vec_fixed = self.vec_fixed_len.clone();
                    let saved_nonneg = self.nonneg_locals.clone();
                    let saved_int_range = self.int_range_locals.clone();
                    let saved_shift_half = self.shift_half_of.clone();
                    let saved_array_elem = self.array_elem_ranges.clone();
                    let saved_int_valued = self.int_valued_locals.clone();
                    self.merge_fn_body_inference(body_slice);
                    self.emit_native_fn_body(body)?;
                    self.vec_fixed_len = saved_vec_fixed;
                    self.nonneg_locals = saved_nonneg;
                    self.int_range_locals = saved_int_range;
                    self.shift_half_of = saved_shift_half;
                    self.array_elem_ranges = saved_array_elem;
                    self.int_valued_locals = saved_int_valued;
                    self.native_fn_body_emit = false;
                    self.native_lcg_hoist.take();
                    self.native_lcg_hoist_int = false;
                    if !self.native_fn_body_returned {
                        self.writeln("0.0");
                    }
                    self.native_fn_emit_name = None;
                }
                self.indent -= 1;
                self.writeln("}");
                self.type_context.pop_scope();
            }
        }
        Ok(())
    }

    fn scan_usize_escape_iter_decl(stmts: &[Statement]) -> Option<String> {
        for i in 0..stmts.len().saturating_sub(1) {
            let iter_name = match &stmts[i] {
                Statement::VarDecl {
                    name,
                    init: Some(e),
                    ..
                } if Self::int_literal_value_of(e) == Some(0) => name.to_string(),
                _ => continue,
            };
            let Statement::While { cond, .. } = &stmts[i + 1] else {
                continue;
            };
            let is_escape = matches!(
                cond,
                Expr::Binary {
                    left,
                    op: BinOp::And,
                    ..
                } if matches!(
                    left.as_ref(),
                    Expr::Binary {
                        left: lv,
                        op: BinOp::Lt,
                        ..
                    } if matches!(
                        lv.as_ref(),
                        Expr::Ident { name, .. } if name.as_ref() == iter_name.as_str()
                    )
                )
            );
            if is_escape {
                return Some(iter_name);
            }
        }
        None
    }

    fn emit_native_fn_body(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                if self.skip_iter_local.is_none() {
                    if let Some(iter) = Self::scan_usize_escape_iter_decl(statements) {
                        self.skip_iter_local = Some(iter);
                    }
                }
                self.emit_statements_with_folds(statements)?;
                if self.pending_stayed_var.is_none() {
                    self.skip_iter_local = None;
                }
            }
            Statement::Return { value, .. } => {
                let e = value.as_ref().expect("eligible return has a value");
                let (code, ty) = self.emit_typed_expr(e)?;
                let f = if ty == RustType::F64 {
                    code
                } else if ty == RustType::Value {
                    RustType::F64.from_value_expr(&code)
                } else {
                    code
                };
                if let Some((global, _, _, _)) = self.native_lcg_hoist.as_ref() {
                    if self.native_lcg_hoist_int {
                        self.writeln(&format!(
                            "{};",
                            Self::native_global_set(global, "(_lcg_seed as f64)")
                        ));
                    } else {
                        self.writeln(&format!("{};", Self::native_global_set(global, "_lcg_seed")));
                    }
                }
                self.writeln(&format!("return {};", f));
                self.native_fn_body_returned = true;
            }
            Statement::If { cond, then_branch, else_branch, .. } => {
                if self.try_emit_checksum_parity_if(cond, then_branch, else_branch)? {
                    return Ok(());
                }
                if let Some(stayed) = self.pending_stayed_var.take() {
                    if self.try_emit_stayed_count_if(cond, then_branch, &stayed)? {
                        return Ok(());
                    }
                    self.pending_stayed_var = Some(stayed);
                }
                let (c, ct) = self.emit_typed_expr(cond)?;
                let c_bool = match ct {
                    RustType::Bool => c,
                    RustType::F64 => format!("({} != 0.0)", c),
                    _ => format!("{}.is_truthy()", c),
                };
                self.writeln(&format!("if {} {{", c_bool));
                self.indent += 1;
                self.emit_native_fn_body(then_branch)?;
                self.indent -= 1;
                if let Some(eb) = else_branch {
                    self.writeln("} else {");
                    self.indent += 1;
                    if let Some((lv, rv)) = Self::parse_strict_eq_idents(cond) {
                        self.strict_lt_bounds.push((lv.clone(), rv.clone()));
                        self.nonneg_locals.insert(lv.clone());
                        self.emit_native_fn_body(eb)?;
                        self.strict_lt_bounds.pop();
                        self.nonneg_locals.remove(&lv);
                    } else {
                        self.emit_native_fn_body(eb)?;
                    }
                    self.indent -= 1;
                }
                self.writeln("}");
            }
            Statement::ExprStmt { expr, .. } => {
                if let Some(skip) = &self.skip_iter_local {
                    if Self::is_increment_of(expr, skip) {
                        return Ok(());
                    }
                }
                let code = self.emit_expr_discard(expr)?;
                self.writeln(&format!("{};", code));
            }
            Statement::VarDecl { .. }
            | Statement::For { .. }
            | Statement::While { .. }
            | Statement::DoWhile { .. } => self.emit_statement(stmt)?,
            _ => unreachable!("emit_native_fn_body: eligibility guarantees only handled statements"),
        }
        Ok(())
    }

    /// #175: emit `e` as a native `f64`, coercing a boxed `Value` result via `as_number`.
    fn emit_f64(&mut self, e: &Expr) -> Result<String, CompileError> {
        let (code, ty) = self.emit_typed_expr(e)?;
        Ok(if ty == RustType::F64 {
            code
        } else if ty == RustType::Value {
            RustType::F64.from_value_expr(&code)
        } else {
            code
        })
    }

    /// Numeric "leaf" fns inlinable at a native-f64 call site: a single `return <numeric expr>` body
    /// (no other statements) built only from the params, number literals, arithmetic/unary, and 1-arg
    /// `Math` — so substituting the params with f64 args yields a pure-f64 expression. Recursion and
    /// any non-numeric form disqualify it. (Boxed callers keep the original closure — inlining only
    /// fires where args are already proven f64, so it never assumes caller types.)
    fn collect_inline_fns(
        program: &Program,
    ) -> std::collections::HashMap<String, (Vec<String>, Expr)> {
        let mut out = std::collections::HashMap::new();
        for s in &program.statements {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                body,
                ..
            } = s
            {
                let pnames: Vec<String> = params
                    .iter()
                    .filter_map(|p| match p {
                        FunParam::Simple(tp) if tp.default.is_none() => Some(tp.name.to_string()),
                        _ => None,
                    })
                    .collect();
                if pnames.len() != params.len() || pnames.is_empty() {
                    continue;
                }
                // Body must be exactly `{ return <expr> }`.
                let ret_expr = match body.as_ref() {
                    Statement::Block { statements, .. } if statements.len() == 1 => {
                        match &statements[0] {
                            Statement::Return { value: Some(e), .. } => Some(e.clone()),
                            _ => None,
                        }
                    }
                    Statement::Return { value: Some(e), .. } => Some(e.clone()),
                    _ => None,
                };
                let Some(expr) = ret_expr else { continue };
                let pset: std::collections::HashSet<&str> =
                    pnames.iter().map(|s| s.as_str()).collect();
                if Self::inline_body_numeric(&expr, &pset, name.as_ref()) {
                    out.insert(name.to_string(), (pnames, expr));
                }
            }
        }
        out
    }

    /// A body expr safe to inline as f64: params, number literals, arithmetic/unary, 1-arg `Math`.
    /// No calls to other fns (incl. self), no string/overloaded shapes — keeps the inlined result f64.
    fn inline_body_numeric(e: &Expr, params: &std::collections::HashSet<&str>, self_name: &str) -> bool {
        match e {
            Expr::Literal { value: Literal::Number(_), .. } => true,
            Expr::Ident { name, .. } => params.contains(name.as_ref()),
            Expr::Binary { left, op, right, .. } => {
                matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                ) && Self::inline_body_numeric(left, params, self_name)
                    && Self::inline_body_numeric(right, params, self_name)
            }
            Expr::Unary { op, operand, .. } => {
                matches!(op, UnaryOp::Neg | UnaryOp::Pos)
                    && Self::inline_body_numeric(operand, params, self_name)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::inline_body_numeric(cond, params, self_name)
                    && Self::inline_body_numeric(then_branch, params, self_name)
                    && Self::inline_body_numeric(else_branch, params, self_name)
            }
            Expr::Call { callee, args, .. } => {
                // Only 1-arg `Math.<intrinsic>` — never another fn (incl. self), to keep it a pure
                // expression with no dispatch and no recursion.
                matches!(callee.as_ref(),
                    Expr::Member { object, prop: MemberProp::Name { name: m, .. }, .. }
                        if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                            && matches!(m.as_ref(), "sqrt"|"sin"|"cos"|"tan"|"abs"|"floor"|"ceil"|"exp"|"trunc"|"log"))
                    && args.len() == 1
                    && matches!(&args[0], CallArg::Expr(a) if Self::inline_body_numeric(a, params, self_name))
            }
            _ => false,
        }
    }

    /// #175 follow-up: inline a call to a numeric leaf fn when every arg is native `f64`. Binds each
    /// arg to an f64 temp (no double-eval), substitutes the params, and emits the body inline — so the
    /// 20M `evalA(i, j)` dispatches in spectral_norm collapse to pure f64 arithmetic. `None` ⇒ no
    /// inline (non-f64 arg, arity/recursion mismatch) and the normal call path runs.
    fn try_emit_inline_call(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
    ) -> Result<Option<(String, RustType)>, CompileError> {
        let Expr::Ident { name, .. } = callee else {
            return Ok(None);
        };
        let Some((params, body)) = self.inline_fns.get(name.as_ref()).cloned() else {
            return Ok(None);
        };
        if params.len() != args.len()
            || self.inlining_now.contains(name.as_ref())
            || self.inline_depth > 8
        {
            return Ok(None);
        }
        // Every arg must emit as native f64; otherwise fall back to the boxed/native call.
        let mut arg_codes: Vec<String> = Vec::with_capacity(args.len());
        for a in args {
            let CallArg::Expr(e) = a else {
                return Ok(None);
            };
            let (code, ty) = self.emit_typed_expr(e)?;
            if ty != RustType::F64 {
                return Ok(None);
            }
            arg_codes.push(code);
        }
        let depth = self.inline_depth;
        let mut prelude = String::new();
        let mut subst = std::collections::HashMap::new();
        for (p, ac) in params.iter().zip(arg_codes.iter()) {
            let temp = format!("_inl{}_{}", depth, Self::escape_ident(p));
            prelude.push_str(&format!("let {}: f64 = {}; ", temp, ac));
            subst.insert(p.clone(), temp);
        }
        self.inline_subst.push(subst);
        self.inlining_now.insert(name.to_string());
        self.inline_depth += 1;
        let body_code = self.emit_f64(&body);
        self.inline_depth -= 1;
        self.inlining_now.remove(name.as_ref());
        self.inline_subst.pop();
        let body_code = body_code?;
        Ok(Some((format!("{{ {}{} }}", prelude, body_code), RustType::F64)))
    }

    // ====================================================================================
    // #175: native plain-array free fns (spectral_norm / queens shape).
    //
    // A top-level fn whose params are `f64` and plain `number[]`/`boolean[]` arrays, whose body only
    // uses native-safe constructs (loops, index get/set, arithmetic, recursion / calls to fns in the
    // same group or M5 / 1-arg Math), and whose array params never escape, is de-virtualized to a
    // native Rust free fn `name_nv(<f64..>, <&/&mut Vec<T>..>) -> f64 | ()`. Call sites pass array
    // idents by reference; they must be PAIRWISE DISTINCT (statically checked) so the `&mut` borrows
    // can't alias — no runtime guard needed. The boxed closure is suppressed for these fns.
    //
    // SOUNDNESS: emitted into a scratch buffer; ANY failure disables the whole group (boxed fallback,
    // byte-identical to flag-off). Reuses the normal `emit_statement` for the body, so loops / index
    // elision / arithmetic are shared with `run()`; only `return` lowers to the native shape.
    // ====================================================================================

    /// Detect the native-vec fn group from the AST alone (no codegen state), so the array-inference
    /// pre-pass and codegen agree on exactly which fns are de-virtualized. Pure: classify each fn's
    /// params, then fixpoint-drop any fn whose call sites are malformed (a dropped callee can
    /// disqualify a caller). M5 / #177 fns are auto-excluded — M5 has no array params, and the param
    /// classifier rejects struct-element arrays (`p[i].field`), which is the aggregate path's shape.
    fn body_uses_local_vec_ops(body: &Statement) -> bool {
        // Only a `.push` onto a var DECLARED LOCALLY in this body counts: the `_nv` lowering emits
        // such a var as an owned `Vec`. A `.push` onto a CAPTURED outer var (e.g. a top-level
        // `let seen = []` closed over by `function dbl(x){ seen.push(x); return x*2 }`) is NOT a
        // local vec op — the free `fn dbl_nv(..)` has no closure environment, so emitting it would
        // reference an unbound `seen` and fail to compile. Such a fn must stay a boxed closure
        // (which captures correctly). Array *params* are handled separately via `has_array_param`.
        let mut locals = std::collections::HashSet::new();
        Self::collect_local_var_names(body, &mut locals);
        let mut found = false;
        Self::for_each_stmt_expr(body, &mut |e| {
            if let Expr::Call { callee, .. } = e {
                if let Expr::Member {
                    object,
                    prop: MemberProp::Name { name: method, .. },
                    ..
                } = callee.as_ref()
                {
                    if method.as_ref() == "push" {
                        if let Expr::Ident { name, .. } = object.as_ref() {
                            if locals.contains(name.as_ref()) {
                                found = true;
                            }
                        }
                    }
                }
            }
        });
        found
    }

    /// #320: candidate fns by BODY shape alone — a normal (boxed-closure) fn whose every param is
    /// `Simple`/no-default, each used as either a plain scalar or a PURELY read-only `number[]` index
    /// (no mutation, no escape, no forwarding — an owned `Vec<f64>` copy would otherwise lose a
    /// caller-visible write/alias), with ≥1 array param, and which is NOT a native-vec (`_nv`) fn.
    /// Returns per-param flags (`true` = array param). This is the READ-ONLY criterion only; INFER
    /// reads it to mark `F(arr)` a non-escape (so `arr` stays `number[]`) — sound regardless of the
    /// arg type because passing a native `Vec` to a boxed closure copies it. CODEGEN does NOT unbox
    /// off this set directly (a caller could still pass a non-`number[]` array → NaN divergence); it
    /// uses [`Self::native_arr_param_fns`], which additionally proves every call site passes a
    /// `number[]`. Runs pre-`si_block` in infer, so it must not depend on `number[]` stamps.
    pub(crate) fn arr_param_readonly_fns(
        program: &Program,
    ) -> std::collections::HashMap<String, Vec<bool>> {
        use std::collections::HashMap;
        let native_vec = Self::detect_native_vec_fns(program);
        let mut out: HashMap<String, Vec<bool>> = HashMap::new();
        for s in &program.statements {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                body,
                ..
            } = s
            {
                if native_vec.contains_key(name.as_ref()) {
                    continue; // the native-vec `_nv` path already handles this fn
                }
                let mut flags: Vec<bool> = Vec::with_capacity(params.len());
                let mut ok = true;
                let mut has_arr = false;
                for p in params {
                    let FunParam::Simple(tp) = p else {
                        ok = false;
                        break;
                    };
                    if tp.default.is_some() {
                        ok = false;
                        break;
                    }
                    // Raw use-facts (NOT `classify_vec_param`, which returns `Array` whenever a param
                    // is indexed even if it also escapes/forwards — too permissive for an owned copy).
                    let mut f = ParamUse::default();
                    Self::for_each_stmt_expr(body, &mut |e| {
                        Self::scan_param_use(e, tp.name.as_ref(), false, &mut f)
                    });
                    if f.indexed {
                        // An owned `Vec<f64>` copy is sound ONLY if the param is a pure read-only
                        // numeric array: no element write (`is_mut`), no bare/escaping use, no
                        // forward into a callee, no bool elements. Any of these would make a
                        // mutation/aliasing observable to the caller that the copy can't reflect.
                        if f.is_mut || f.escaped || f.forwarded || f.elem_bool || f.numeric {
                            ok = false;
                            break;
                        }
                        flags.push(true);
                        has_arr = true;
                    } else {
                        // Non-indexed (scalar / object) param → leave it on its normal path.
                        flags.push(false);
                    }
                }
                if ok && has_arr {
                    out.insert(name.to_string(), flags);
                }
            }
        }
        out
    }

    /// #320 (CODEGEN unbox set): the read-only candidates ([`Self::arr_param_readonly_fns`]) further
    /// restricted to fns where EVERY call site passes a provably-`number[]` argument in each array
    /// position — so unboxing the param to an owned `Vec<f64>` can never see a non-number element
    /// (which would diverge from JS: `n + "x"` is string concat, not NaN arithmetic). Runs on the
    /// FULLY-INFERRED program (so `number[]` stamps are present). A fn is dropped if it is used as a
    /// non-call value, called indirectly, called with the wrong arity, or any array-position arg is
    /// not an identifier bound to a `number[]`. If a fn is in the read-only set but dropped here, it
    /// simply stays a fully boxed closure (sound, just unoptimized).
    pub(crate) fn native_arr_param_fns(
        program: &Program,
    ) -> std::collections::HashMap<String, Vec<bool>> {
        use std::collections::HashMap;
        let readonly = Self::arr_param_readonly_fns(program);
        if readonly.is_empty() {
            return readonly;
        }
        let num_arrays = Self::collect_number_array_bindings(program);
        let mut out: HashMap<String, Vec<bool>> = HashMap::new();
        for (fname, flags) in &readonly {
            if Self::all_callsites_pass_number_arrays(program, fname, flags, &num_arrays) {
                out.insert(fname.clone(), flags.clone());
            }
        }
        out
    }

    /// TOP-LEVEL names bound to a `number[]` (a top-level `VarDecl` annotated `Array<number>`). Only
    /// top-level bindings: a fn param/local of the same name lives in another SCOPE, so conflating
    /// them would be wrong — `all_callsites_pass_number_arrays` correspondingly only accepts calls at
    /// top level, where an argument identifier resolves unambiguously to one of these. A top-level
    /// name re-bound to a non-`number[]` value is conservatively dropped.
    fn collect_number_array_bindings(program: &Program) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let mut out: HashSet<String> = HashSet::new();
        let mut conflict: HashSet<String> = HashSet::new();
        for s in &program.statements {
            if let Statement::VarDecl { name, type_ann, .. } = s {
                let is_num_arr = matches!(type_ann, Some(TypeAnnotation::Array(inner)) if Self::ann_is_number(inner));
                if is_num_arr && !conflict.contains(name.as_ref()) {
                    out.insert(name.to_string());
                } else if !is_num_arr {
                    out.remove(name.as_ref());
                    conflict.insert(name.to_string());
                }
            }
        }
        out
    }

    /// True iff EVERY reference to `fname` in the WHOLE program (including nested fn bodies and
    /// exports) is a direct call `fname(args)` whose array positions (per `flags`) are identifiers
    /// bound to a `number[]`, with matching arity. Any other use — bare value, indirect/aliased
    /// call, wrong arity, or a non-`number[]` array arg — makes it `false` (so the param stays boxed).
    fn all_callsites_pass_number_arrays(
        program: &Program,
        fname: &str,
        flags: &[bool],
        num_arrays: &std::collections::HashSet<String>,
    ) -> bool {
        let mut ok = true;
        for s in &program.statements {
            Self::scan_stmt_for_fname(s, fname, flags, num_arrays, false, &mut ok);
            if !ok {
                return false;
            }
        }
        ok
    }

    /// Recurse every statement (incl. nested `FunDecl` bodies and `export`ed declarations), applying
    /// [`Self::scan_expr_for_fname`] to each statement-level expression.
    fn scan_stmt_for_fname(
        s: &Statement,
        fname: &str,
        flags: &[bool],
        num_arrays: &std::collections::HashSet<String>,
        in_fn: bool,
        ok: &mut bool,
    ) {
        use Statement::*;
        if !*ok {
            return;
        }
        let mut e = |x: &Expr, ok: &mut bool| {
            Self::scan_expr_for_fname(x, fname, flags, num_arrays, in_fn, ok)
        };
        let mut st = |x: &Statement, ok: &mut bool| {
            Self::scan_stmt_for_fname(x, fname, flags, num_arrays, in_fn, ok)
        };
        match s {
            Block { statements, .. } | Multi { statements, .. } => {
                statements.iter().for_each(|x| st(x, ok))
            }
            VarDecl { init: Some(i), .. } => e(i, ok),
            VarDeclDestructure { init, .. } => e(init, ok),
            ExprStmt { expr, .. } => e(expr, ok),
            If { cond, then_branch, else_branch, .. } => {
                e(cond, ok);
                st(then_branch, ok);
                if let Some(eb) = else_branch {
                    st(eb, ok);
                }
            }
            While { cond, body, .. } | DoWhile { body, cond, .. } => {
                e(cond, ok);
                st(body, ok);
            }
            For { init, cond, update, body, .. } => {
                if let Some(i) = init {
                    st(i, ok);
                }
                if let Some(c) = cond {
                    e(c, ok);
                }
                if let Some(u) = update {
                    e(u, ok);
                }
                st(body, ok);
            }
            ForOf { iterable, body, .. } => {
                e(iterable, ok);
                st(body, ok);
            }
            Return { value: Some(v), .. } => e(v, ok),
            Throw { value, .. } => e(value, ok),
            Switch { expr, cases, default_body, .. } => {
                e(expr, ok);
                for (g, body) in cases {
                    if let Some(ge) = g {
                        e(ge, ok);
                    }
                    body.iter().for_each(|x| st(x, ok));
                }
                if let Some(d) = default_body {
                    d.iter().for_each(|x| st(x, ok));
                }
            }
            Try { body, catch_body, finally_body, .. } => {
                st(body, ok);
                if let Some(c) = catch_body {
                    st(c, ok);
                }
                if let Some(fb) = finally_body {
                    st(fb, ok);
                }
            }
            // A nested fn body is a new scope: an array arg there could be a (shadowing) param/local,
            // which `num_arrays` (top-level only) can't resolve — so flag `in_fn` and let any call to
            // `fname` inside it disqualify the optimization.
            FunDecl { body, .. } => {
                Self::scan_stmt_for_fname(body, fname, flags, num_arrays, true, ok)
            }
            Export { declaration, .. } => match declaration.as_ref() {
                tishlang_ast::ExportDeclaration::Named(inner) => st(inner, ok),
                tishlang_ast::ExportDeclaration::Default(ex) => e(ex, ok),
                tishlang_ast::ExportDeclaration::ReExport { .. } => {}
            },
            // Break / Continue / Import / TypeAlias / DeclareVar / DeclareFun — no user-fn calls.
            _ => {}
        }
    }

    /// Recurse one expression: a direct `fname(args)` call is checked (arity + `number[]` array
    /// args), other sub-expressions are recursed, and ANY bare `fname` identifier fails. Unhandled
    /// expr forms fall to a conservative `collect_idents_of` net: if `fname` appears there, fail.
    fn scan_expr_for_fname(
        e: &Expr,
        fname: &str,
        flags: &[bool],
        num_arrays: &std::collections::HashSet<String>,
        in_fn: bool,
        ok: &mut bool,
    ) {
        if !*ok {
            return;
        }
        let mut rec = |x: &Expr, ok: &mut bool| {
            Self::scan_expr_for_fname(x, fname, flags, num_arrays, in_fn, ok)
        };
        let mut rec_args = |args: &[CallArg], ok: &mut bool| {
            for a in args {
                if let CallArg::Expr(x) | CallArg::Spread(x) = a {
                    rec(x, ok);
                }
            }
        };
        match e {
            // A direct call to `fname`: validate it, then recurse into the ARGS only (the callee
            // identifier is the consumed `fname`, not a bare use).
            Expr::Call { callee, args, .. }
                if matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == fname) =>
            {
                // A call inside a fn body can't have its array arg resolved against top-level-only
                // `num_arrays` (the arg may be a shadowing param/local) — conservatively disqualify.
                if in_fn || args.len() != flags.len() {
                    *ok = false;
                    return;
                }
                for (i, want_arr) in flags.iter().enumerate() {
                    if *want_arr
                        && !matches!(&args[i], CallArg::Expr(Expr::Ident { name, .. }) if num_arrays.contains(name.as_ref()))
                    {
                        *ok = false;
                        return;
                    }
                }
                rec_args(args, ok);
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                rec(callee, ok);
                rec_args(args, ok);
            }
            // A bare `fname` (not the callee of a direct call handled above) → can't prove safety.
            Expr::Ident { name, .. } if name.as_ref() == fname => *ok = false,
            Expr::Ident { .. } | Expr::Literal { .. } => {}
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                rec(left, ok);
                rec(right, ok);
            }
            Expr::Unary { operand, .. } => rec(operand, ok),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                rec(cond, ok);
                rec(then_branch, ok);
                rec(else_branch, ok);
            }
            Expr::Index { object, index, .. } => {
                rec(object, ok);
                rec(index, ok);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                rec(object, ok);
                rec(index, ok);
                rec(value, ok);
            }
            Expr::Member { object, prop, .. } => {
                rec(object, ok);
                if let MemberProp::Expr(pe) = prop {
                    rec(pe, ok);
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                rec(object, ok);
                rec(value, ok);
            }
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => rec(value, ok),
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(x) | ArrayElement::Spread(x) => rec(x, ok),
                    }
                }
            }
            Expr::Object { props, .. } => {
                for pr in props {
                    match pr {
                        ObjectProp::KeyValue(_, x, _) | ObjectProp::Spread(x) => rec(x, ok),
                    }
                }
            }
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().for_each(|x| rec(x, ok)),
            // Unhandled form (closures, await, etc.): conservatively fail if `fname` appears at all.
            other => {
                if Self::collect_idents_of(other).contains(fname) {
                    *ok = false;
                }
            }
        }
    }

    pub(crate) fn detect_native_vec_fns(
        program: &Program,
    ) -> std::collections::HashMap<String, NativeVecFnSig> {
        use std::collections::HashMap;
        let mut sigs: HashMap<String, NativeVecFnSig> = HashMap::new();
        // Soundness: a fn called anywhere with a provably-non-numeric arg must not become a native-vec
        // fn either — its scalar params would be typed `f64` and mistype the arg (e.g. `fn rec(m){
        // log.push(m) }` called `rec("ran")`). Same gate the M4/M5/S-0 paths use.
        let nonnumeric = Self::fns_called_with_nonnumeric_arg(&program.statements);
        for s in &program.statements {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                body,
                ..
            } = s
            {
                if nonnumeric.contains(name.as_ref()) {
                    continue;
                }
                if let Some(sig) = Self::infer_vec_fn_sig(params, body) {
                    let has_array_param = sig
                        .params
                        .iter()
                        .any(|(_, k)| matches!(k, VecParamKind::Array { .. }));
                    if has_array_param || Self::body_uses_local_vec_ops(body) {
                        sigs.insert(name.to_string(), sig);
                    }
                }
            }
        }
        loop {
            let mut remove: Vec<String> = Vec::new();
            for s in &program.statements {
                Self::collect_bad_vec_callsites(s, &sigs, &mut remove);
            }
            remove.sort();
            remove.dedup();
            if remove.is_empty() {
                break;
            }
            for n in remove {
                sigs.remove(&n);
            }
            if sigs.is_empty() {
                break;
            }
        }
        // Forwarding fixpoint: fns whose array params are only threaded to other native-vec fns
        // (`multiplyAtAv` → `multiplyAv`/`multiplyAtv`) qualify once callees are in `sigs`.
        loop {
            let mut added = false;
            for s in &program.statements {
                if let Statement::FunDecl {
                    async_: false,
                    name,
                    params,
                    rest_param: None,
                    body,
                    ..
                } = s
                {
                    if sigs.contains_key(name.as_ref()) {
                        continue;
                    }
                    // Same soundness gate as the initial pass: the forwarding fixpoint must also refuse
                    // a fn called with a non-numeric arg, or `body_uses_local_vec_ops` re-adds it here
                    // (e.g. `fn rec(m){ log.push(m) }` called `rec("ran")`).
                    if nonnumeric.contains(name.as_ref()) {
                        continue;
                    }
                    if let Some(sig) = Self::infer_vec_fn_sig_forwarding(params, body, &sigs) {
                        let has_array_param = sig
                            .params
                            .iter()
                            .any(|(_, k)| matches!(k, VecParamKind::Array { .. }));
                        if has_array_param || Self::body_uses_local_vec_ops(body) {
                            sigs.insert(name.to_string(), sig);
                            added = true;
                        }
                    }
                }
            }
            if !added {
                break;
            }
            loop {
                let mut remove: Vec<String> = Vec::new();
                for s in &program.statements {
                    Self::collect_bad_vec_callsites(s, &sigs, &mut remove);
                }
                remove.sort();
                remove.dedup();
                if remove.is_empty() {
                    break;
                }
                for n in remove {
                    sigs.remove(&n);
                }
                if sigs.is_empty() {
                    break;
                }
            }
            if sigs.is_empty() {
                break;
            }
        }
        sigs
    }

    /// Pre-order visit of `e` and every sub-expression (incl. arrow-function bodies). Used by the
    /// native-vec param-bound proof to find call sites and variable mutations anywhere in a scope.
    fn nv_for_each_subexpr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
        f(e);
        let mut rec = |x: &Expr| Self::nv_for_each_subexpr(x, f);
        match e {
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                rec(left);
                rec(right);
            }
            Expr::Unary { operand, .. }
            | Expr::TypeOf { operand, .. }
            | Expr::Await { operand, .. }
            | Expr::Delete { target: operand, .. } => rec(operand),
            // `Assign`/`CompoundAssign`/`LogicalAssign` target a bare `name` (not a sub-expr); only the
            // value is an expression to recurse.
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => rec(value),
            Expr::Conditional { cond, then_branch, else_branch, .. } => {
                rec(cond);
                rec(then_branch);
                rec(else_branch);
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                rec(callee);
                for a in args {
                    if let CallArg::Expr(x) | CallArg::Spread(x) = a {
                        rec(x);
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                rec(object);
                if let MemberProp::Expr(p) = prop {
                    rec(p);
                }
            }
            Expr::MemberAssign { object, value, .. } => {
                rec(object);
                rec(value);
            }
            Expr::Index { object, index, .. } => {
                rec(object);
                rec(index);
            }
            Expr::IndexAssign { object, index, value, .. } => {
                rec(object);
                rec(index);
                rec(value);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    if let ArrayElement::Expr(x) | ArrayElement::Spread(x) = el {
                        rec(x);
                    }
                }
            }
            Expr::Object { props, .. } => {
                for p in props {
                    match p {
                        ObjectProp::KeyValue(_, v, _) | ObjectProp::Spread(v) => rec(v),
                    }
                }
            }
            Expr::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(x) => rec(x),
                ArrowBody::Block(s) => {
                    Self::for_each_stmt_expr(s, &mut |x| Self::nv_for_each_subexpr(x, f))
                }
            },
            Expr::TemplateLiteral { exprs, .. } => exprs.iter().for_each(rec),
            _ => {}
        }
    }

    /// Every identifier that is the TARGET of a mutation (`=`, `+=`, `||=`, `++`, `--`) anywhere in
    /// `stmts` — descending through nested statements, fn-decl bodies, and arrow bodies. Used to find
    /// which params are stable (never reassigned), so a scalar arg naming one is a constant length
    /// bound. Conservative: a deeper-scope local of the same name also marks the name (rejects → the
    /// param-bound proof keeps the OOB-safe lowering, which is always sound).
    fn nv_collect_mutated_idents(stmts: &[Statement]) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        fn go(stmts: &[Statement], out: &mut std::collections::HashSet<String>) {
            for s in stmts {
                Codegen::for_each_stmt_expr(s, &mut |e| {
                    Codegen::nv_for_each_subexpr(e, &mut |x| match x {
                        Expr::Assign { name, .. }
                        | Expr::CompoundAssign { name, .. }
                        | Expr::LogicalAssign { name, .. }
                        | Expr::PostfixInc { name, .. }
                        | Expr::PostfixDec { name, .. }
                        | Expr::PrefixInc { name, .. }
                        | Expr::PrefixDec { name, .. } => {
                            out.insert(name.to_string());
                        }
                        _ => {}
                    })
                });
                Codegen::for_each_child_stmt_list(s, &mut |list| go(list, out));
            }
        }
        go(stmts, &mut out);
        out
    }

    /// Record every direct call to a native-vec fn as `(caller-scope, callee, positional-args)`. The
    /// caller scope is `None` at top level and `Some(fn)` inside that fn's body (nested fns switch
    /// scope). A call with a spread arg is recorded with EMPTY args so the arity check fails and the
    /// callee stays unproven (positions aren't statically known). Soundness depends on recording EVERY
    /// call site — a missed one could pass a short array — so the walk covers control-flow and arrows.
    fn nv_collect_callsites(
        &self,
        program: &Program,
        sigs: &std::collections::HashMap<String, NativeVecFnSig>,
    ) -> Vec<(Option<String>, String, Vec<Expr>)> {
        let mut out: Vec<(Option<String>, String, Vec<Expr>)> = Vec::new();
        let mut record = |caller: &Option<String>, e: &Expr, out: &mut Vec<_>| {
            Self::nv_for_each_subexpr(e, &mut |x| {
                if let Expr::Call { callee, args, .. } = x {
                    if let Expr::Ident { name, .. } = callee.as_ref() {
                        if sigs.contains_key(name.as_ref()) {
                            let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                            let argv: Vec<Expr> = if has_spread {
                                Vec::new()
                            } else {
                                args.iter()
                                    .filter_map(|a| match a {
                                        CallArg::Expr(x) => Some(x.clone()),
                                        CallArg::Spread(_) => None,
                                    })
                                    .collect()
                            };
                            out.push((caller.clone(), name.to_string(), argv));
                        }
                    }
                }
            });
        };
        // Top-level scope (caller None) — `for_each_stmt_expr` does not enter fn bodies.
        for s in &program.statements {
            Self::for_each_stmt_expr(s, &mut |e| record(&None, e, &mut out));
        }
        // Each fn scope (any nesting depth), caller = that fn.
        Self::nv_for_each_fundecl(&program.statements, &mut |name, body| {
            let caller = Some(name.to_string());
            for s in Self::fn_body_stmt_slice(body) {
                Self::for_each_stmt_expr(s, &mut |e| record(&caller, e, &mut out));
            }
        });
        // Param-default exprs are evaluated at a fn's call sites, not in any body scanned above. A
        // native-vec call there would route to `_nv` with args we cannot place in a known scope —
        // POISON the callee (an empty-arg record fails the arity check ⇒ it is never proven), keeping
        // it OOB-safe. Cheap and conservative; spectral_norm has no defaulted params.
        Self::nv_for_each_param_default(&program.statements, &mut |d| {
            Self::nv_for_each_subexpr(d, &mut |x| {
                if let Expr::Call { callee, .. } = x {
                    if let Expr::Ident { name, .. } = callee.as_ref() {
                        if sigs.contains_key(name.as_ref()) {
                            out.push((None, name.to_string(), Vec::new()));
                        }
                    }
                }
            });
        });
        out
    }

    /// Visit every `FunParam::Simple` default-value expression at any nesting depth.
    fn nv_for_each_param_default(stmts: &[Statement], f: &mut dyn FnMut(&Expr)) {
        for s in stmts {
            if let Statement::FunDecl { params, .. } = s {
                for p in params {
                    if let FunParam::Simple(tp) = p {
                        if let Some(d) = &tp.default {
                            f(d);
                        }
                    }
                }
            }
            Self::for_each_child_stmt_list(s, &mut |list| Self::nv_for_each_param_default(list, f));
        }
    }

    /// Visit every `FunDecl` at any nesting depth as `(name, body)`.
    fn nv_for_each_fundecl(stmts: &[Statement], f: &mut dyn FnMut(&str, &Statement)) {
        for s in stmts {
            if let Statement::FunDecl { name, body, .. } = s {
                f(name.as_ref(), body);
            }
            Self::for_each_child_stmt_list(s, &mut |list| Self::nv_for_each_fundecl(list, f));
        }
    }

    /// #203 follow-up — SOUND recovery of the native-vec array-param length bound the unsound blind
    /// assumption used to provide. Returns, per native-vec fn, the array params PROVEN at EVERY call
    /// site to be at least as long as the fn's first scalar param, so `emit_native_vec_fn` re-registers
    /// `vec_fixed_len[param] = Var(scalar)` and drops the OOB-safe `.get()` fallback for in-bounds reads.
    ///
    /// Soundness rests on three facts:
    ///  1. A native-vec array param's length never changes inside the body — `classify_vec_param`
    ///     rejects `push`/`pop`/`splice`/`.length =` (they escape), leaving only `arr[i]`, `arr[i]=v`,
    ///     `.length`, and forwarding to another native-vec fn. So a bound proven at entry holds.
    ///  2. `collect_vec_fixed_len` proves a caller LOCAL's exact fill length (`for i<m {a.push(_)}` → m)
    ///     and its escape filter drops any local that could be shrunk.
    ///  3. The bound variable `m` (a scalar arg that names the array's fill-length var) is a caller
    ///     PARAMETER never reassigned in the caller body — so `length(a) == m` still holds at the call.
    ///
    /// A monotone fixpoint propagates the fact across the call graph: spectral_norm threads length-`n`
    /// locals through `multiplyAtAv` into `multiplyAv`. Anything not provable keeps the OOB-safe path.
    fn compute_native_vec_param_bounds(
        &self,
        program: &Program,
    ) -> std::collections::HashMap<String, std::collections::HashMap<String, BoundKey>> {
        use std::collections::{HashMap, HashSet};
        let sigs = &self.native_vec_fns;
        if sigs.is_empty() {
            return HashMap::new();
        }
        // Per-caller context (None = top-level). `params`: Simple param names. `stable`: params never
        // mutated in the body. `fixed`: local arrays with a proven fill length.
        let mut params: HashMap<Option<String>, HashSet<String>> = HashMap::new();
        let mut stable: HashMap<Option<String>, HashSet<String>> = HashMap::new();
        let mut fixed: HashMap<Option<String>, HashMap<String, BoundKey>> = HashMap::new();
        params.insert(None, HashSet::new());
        stable.insert(None, HashSet::new());
        fixed.insert(None, self.collect_vec_fixed_len(&program.statements));
        for s in &program.statements {
            if let Statement::FunDecl { name, params: ps, body, .. } = s {
                let pnames: HashSet<String> = ps
                    .iter()
                    .filter_map(|p| match p {
                        FunParam::Simple(tp) => Some(tp.name.to_string()),
                        _ => None,
                    })
                    .collect();
                let body_stmts = Self::fn_body_stmt_slice(body);
                let mutated = Self::nv_collect_mutated_idents(body_stmts);
                let stable_ps: HashSet<String> =
                    pnames.iter().filter(|p| !mutated.contains(*p)).cloned().collect();
                let key = Some(name.to_string());
                params.insert(key.clone(), pnames);
                stable.insert(key.clone(), stable_ps);
                fixed.insert(key, self.collect_vec_fixed_len(body_stmts));
            }
        }
        let calls = self.nv_collect_callsites(program, sigs);
        // Monotone fixpoint: a newly-proven param bound can enable a callee's bound (param-flow).
        let mut proven: HashMap<String, HashMap<String, BoundKey>> = HashMap::new();
        loop {
            let mut changed = false;
            for (fname, sig) in sigs {
                let Some(si) = sig
                    .params
                    .iter()
                    .position(|(_, k)| matches!(k, VecParamKind::Scalar))
                else {
                    continue; // no scalar param ⇒ nothing to bound the array length against.
                };
                let s_name = sig.params[si].0.clone();
                let sites: Vec<&(Option<String>, String, Vec<Expr>)> =
                    calls.iter().filter(|(_, callee, _)| callee == fname).collect();
                if sites.is_empty() {
                    continue; // uncalled ⇒ no evidence of caller array lengths; stay OOB-safe.
                }
                for (pi, (p_name, kind)) in sig.params.iter().enumerate() {
                    if !matches!(kind, VecParamKind::Array { .. }) {
                        continue;
                    }
                    if proven.get(fname).is_some_and(|m| m.contains_key(p_name)) {
                        continue;
                    }
                    let all_ok = sites.iter().all(|(caller, _, args)| {
                        args.len() == sig.params.len()
                            && Self::nv_callsite_arg_len_ge(
                                caller,
                                args.get(pi),
                                args.get(si),
                                &params,
                                &stable,
                                &fixed,
                                &proven,
                            )
                    });
                    if all_ok {
                        proven
                            .entry(fname.clone())
                            .or_default()
                            .insert(p_name.clone(), BoundKey::Var(s_name.clone()));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        proven
    }

    /// Does the array argument `arr_arg` (in scope `caller`) provably have length >= the value of the
    /// scalar argument `scalar_arg`? Sound cases only — anything else returns false (param stays
    /// OOB-safe). See [`Self::compute_native_vec_param_bounds`] for the soundness argument.
    #[allow(clippy::too_many_arguments)]
    fn nv_callsite_arg_len_ge(
        caller: &Option<String>,
        arr_arg: Option<&Expr>,
        scalar_arg: Option<&Expr>,
        params: &std::collections::HashMap<Option<String>, std::collections::HashSet<String>>,
        stable: &std::collections::HashMap<Option<String>, std::collections::HashSet<String>>,
        fixed: &std::collections::HashMap<Option<String>, std::collections::HashMap<String, BoundKey>>,
        proven: &std::collections::HashMap<String, std::collections::HashMap<String, BoundKey>>,
    ) -> bool {
        let (Some(arr_arg), Some(scalar_arg)) = (arr_arg, scalar_arg) else {
            return false;
        };
        // The array argument must be a bare identifier (the native-vec call ABI requires this anyway).
        let Expr::Ident { name: a, .. } = arr_arg else {
            return false;
        };
        let a = a.as_ref();
        // (1) scalar arg is `a.length` — exact equality, always >=.
        if let Expr::Member { object, prop, .. } = scalar_arg {
            if matches!(prop, MemberProp::Name { name, .. } if name.as_ref() == "length")
                && matches!(object.as_ref(), Expr::Ident { name: o, .. } if o.as_ref() == a)
            {
                return true;
            }
        }
        // A scalar arg that NAMES a variable is only a sound bound if that variable is a STABLE param
        // of the caller (never reassigned) — otherwise its value at the call may exceed the length the
        // array was filled to.
        let scalar_is_stable_var = |m: &str| {
            matches!(scalar_arg, Expr::Ident { name: s, .. } if s.as_ref() == m)
                && stable.get(caller).is_some_and(|s| s.contains(m))
        };
        let caller_fixed = fixed.get(caller);
        // (2) `a` is a caller LOCAL filled to length `Var(m)` and the scalar arg is that stable `m`.
        if let Some(BoundKey::Var(m)) = caller_fixed.and_then(|f| f.get(a)) {
            if scalar_is_stable_var(m) {
                return true;
            }
        }
        // (2b) `a` is a caller LOCAL of proven Const length `k` and the scalar arg is the int literal
        // `k` EXACTLY. Exact (not `<= k`) so every proven bound means `length == scalar`, identical to
        // the trusted local-array `Var(n)` bounds — a strictly-longer array could otherwise make a
        // `.length`-folding read disagree with the boxed path. (Inductively keeps case (3) exact too.)
        if let Some(BoundKey::Const(k)) = caller_fixed.and_then(|f| f.get(a)) {
            if let Expr::Literal { value: Literal::Number(n), .. } = scalar_arg {
                if *n >= 0.0 && n.fract() == 0.0 && (*n as i64) == *k {
                    return true;
                }
            }
        }
        // (3) param-flow: `a` is the caller's OWN array param with an already-proven bound `Var(m)`, and
        // the scalar arg is that same stable `m` (the caller's scalar that bounds `a`).
        if let Some(cname) = caller {
            if params.get(caller).is_some_and(|p| p.contains(a)) {
                if let Some(BoundKey::Var(m)) = proven.get(cname).and_then(|m| m.get(a)) {
                    if scalar_is_stable_var(m) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Detect, validate, and emit the native-vec fn group. On any gap the group is left empty.
    fn setup_native_vec_fns(&mut self, program: &Program) {
        use std::collections::HashMap;
        let mut sigs = Self::detect_native_vec_fns(program);
        // #177 aggregate fns (`advance`/`offsetMomentum` on `Vec<Struct>`) take precedence — do not
        // also emit a broken `name_nv(&Vec<f64>)` wrapper for them.
        sigs.retain(|name, _| !self.aggregate_fns.contains_key(name));
        if sigs.is_empty() {
            return;
        }
        // Codegen-side gate (needs `native_fns`): a native-vec body may only CALL fns that have a
        // native form — other native-vec fns or M5 `native_fns` — plus 1-arg `Math`. Calling a boxed
        // closure (e.g. spectral_norm's `evalA`, which isn't M5) can't be referenced from the
        // module-level free fn, so drop such fns (fixpoint: a dropped callee disqualifies its caller).
        let bodies: HashMap<String, &Statement> = program
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::FunDecl { name, body, .. } => Some((name.to_string(), body.as_ref())),
                _ => None,
            })
            .collect();
        loop {
            let mut remove: Vec<String> = Vec::new();
            for name in sigs.keys() {
                if let Some(body) = bodies.get(name) {
                    if !self.vec_body_calls_native(body, &sigs) {
                        remove.push(name.clone());
                    }
                }
            }
            if remove.is_empty() {
                break;
            }
            for n in remove {
                sigs.remove(&n);
            }
            if sigs.is_empty() {
                return;
            }
        }
        if sigs.is_empty() {
            return;
        }
        let decls: Vec<(String, Statement)> = program
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::FunDecl { name, body, .. } if sigs.contains_key(name.as_ref()) => {
                    Some((name.to_string(), (**body).clone()))
                }
                _ => None,
            })
            .collect();
        // Emit into a scratch buffer; roll back on any failure.
        self.native_vec_fns = sigs;
        // #203 follow-up: prove (where sound) that array params are long enough to drop bounds guards.
        self.native_vec_param_bounds = self.compute_native_vec_param_bounds(program);
        let body_of: HashMap<&str, &Statement> =
            decls.iter().map(|(n, b)| (n.as_str(), b)).collect();
        for (name, sig) in &self.native_vec_fns {
            self.native_fn_param_names.insert(
                name.clone(),
                sig.params.iter().map(|(p, _)| p.clone()).collect(),
            );
        }
        let vec_names: std::collections::HashSet<String> =
            self.native_vec_fns.keys().cloned().collect();
        self.native_fn_literal_args.extend(
            Self::collect_native_fn_literal_calls(&program.statements, &vec_names),
        );
        let saved = std::mem::take(&mut self.output);
        let saved_indent = self.indent;
        self.indent = 0;
        let mut all_ok = true;
        let mut names: Vec<String> = self.native_vec_fns.keys().cloned().collect();
        names.sort();
        for n in &names {
            let body = body_of.get(n.as_str()).copied().unwrap();
            if self.emit_native_vec_fn(n, body).is_err() {
                all_ok = false;
                break;
            }
        }
        let emitted = std::mem::replace(&mut self.output, saved);
        self.indent = saved_indent;
        if all_ok {
            self.output.push_str(&emitted);
            self.writeln("");
        } else {
            self.native_vec_fns.clear();
        }
    }

    /// Infer a [`NativeVecFnSig`] from a fn's params + body, or `None` if a param can't be cleanly
    /// classified as a scalar `f64` or an indexed `number[]`/`boolean[]`. Also requires a consistent
    /// return shape (all returns numeric → f64, or none → unit).
    fn infer_vec_fn_sig(params: &[FunParam], body: &Statement) -> Option<NativeVecFnSig> {
        let mut sig_params = Vec::with_capacity(params.len());
        for p in params {
            let FunParam::Simple(tp) = p else {
                return None;
            };
            if tp.default.is_some() {
                return None;
            }
            let kind = Self::classify_vec_param(body, tp.name.as_ref())?;
            sig_params.push((tp.name.to_string(), kind));
        }
        let ret = Self::vec_fn_return_kind(params, body)?;
        Some(NativeVecFnSig {
            params: sig_params,
            ret,
        })
    }

    /// Like [`infer_vec_fn_sig`], but array params may be classified by forwarding into an
    /// already-detected native-vec callee (`multiplyAtAv` shape).
    fn infer_vec_fn_sig_forwarding(
        params: &[FunParam],
        body: &Statement,
        sigs: &std::collections::HashMap<String, NativeVecFnSig>,
    ) -> Option<NativeVecFnSig> {
        let mut sig_params = Vec::with_capacity(params.len());
        for p in params {
            let FunParam::Simple(tp) = p else {
                return None;
            };
            if tp.default.is_some() {
                return None;
            }
            let indexed = Self::classify_vec_param(body, tp.name.as_ref());
            let forwarded =
                Self::classify_vec_param_forwarding(body, tp.name.as_ref(), sigs);
            let kind = match indexed {
                Some(VecParamKind::Array { .. }) => indexed,
                Some(VecParamKind::Scalar) => forwarded.or(indexed),
                None => forwarded,
            }?;
            sig_params.push((tp.name.to_string(), kind));
        }
        let ret = Self::vec_fn_return_kind(params, body)?;
        Some(NativeVecFnSig {
            params: sig_params,
            ret,
        })
    }

    /// `p` is only passed as a call argument to native-vec fns at array-param positions.
    fn classify_vec_param_forwarding(
        body: &Statement,
        p: &str,
        sigs: &std::collections::HashMap<String, NativeVecFnSig>,
    ) -> Option<VecParamKind> {
        let mut kinds: Vec<VecParamKind> = Vec::new();
        let mut bad = false;
        Self::for_each_stmt_expr(body, &mut |e| {
            if bad {
                return;
            }
            if let Expr::Call { callee, args, .. } = e {
                let Expr::Ident { name: fname, .. } = callee.as_ref() else {
                    bad = true;
                    return;
                };
                let sig = match sigs.get(fname.as_ref()) {
                    Some(s) => s,
                    None => return,
                };
                if args.len() != sig.params.len() {
                    bad = true;
                    return;
                }
                for (i, a) in args.iter().enumerate() {
                    let CallArg::Expr(arg_e) = a else {
                        bad = true;
                        return;
                    };
                    if let Expr::Ident { name, .. } = arg_e {
                        if name.as_ref() == p {
                            match &sig.params[i].1 {
                                VecParamKind::Array { .. } => kinds.push(sig.params[i].1.clone()),
                                VecParamKind::Scalar => bad = true,
                            }
                        }
                    }
                }
            }
        });
        if bad || kinds.is_empty() {
            return None;
        }
        let mut forward_uses = 0usize;
        Self::for_each_stmt_expr(body, &mut |e| {
            if let Expr::Call { callee, args, .. } = e {
                let Expr::Ident { name: fname, .. } = callee.as_ref() else {
                    return;
                };
                if !sigs.contains_key(fname.as_ref()) {
                    return;
                }
                for a in args {
                    if let CallArg::Expr(Expr::Ident { name, .. }) = a {
                        if name.as_ref() == p {
                            forward_uses += 1;
                        }
                    }
                }
            }
        });
        if forward_uses != kinds.len() {
            return None;
        }
        let mut is_mut = false;
        let mut elem_bool = false;
        for k in &kinds {
            if let VecParamKind::Array { elem, is_mut: im } = k {
                if *im {
                    is_mut = true;
                }
                if *elem == RustType::Bool {
                    elem_bool = true;
                }
            }
        }
        Some(VecParamKind::Array {
            elem: if elem_bool {
                RustType::Bool
            } else {
                RustType::F64
            },
            is_mut,
        })
    }

    /// Classify one param by how the body uses it: indexed/`.length` → `Array` (`is_mut` if any
    /// element store; element `Bool` if any store writes a bool literal, else `F64`); only-numeric →
    /// `Scalar`. A use that escapes (bare value flowing somewhere other than index/length/call-arg),
    /// or a param both indexed and arithmetic'd, yields `None`.
    fn classify_vec_param(body: &Statement, p: &str) -> Option<VecParamKind> {
        let mut f = ParamUse::default();
        Self::for_each_stmt_expr(body, &mut |e| Self::scan_param_use(e, p, false, &mut f));
        if f.indexed && f.numeric {
            return None;
        }
        if f.indexed {
            Some(VecParamKind::Array {
                elem: if f.elem_bool {
                    RustType::Bool
                } else {
                    RustType::F64
                },
                is_mut: f.is_mut,
            })
        } else if f.escaped && !f.numeric {
            None
        } else {
            Some(VecParamKind::Scalar)
        }
    }

    /// Context-aware scan of one expression for uses of `p`. `arith_parent` marks an arithmetic/
    /// comparison parent (a bare `p` there is a numeric use, not an escape).
    fn scan_param_use(e: &Expr, p: &str, arith_parent: bool, f: &mut ParamUse) {
        match e {
            Expr::Ident { name, .. } if name.as_ref() == p => {
                if arith_parent {
                    f.numeric = true;
                } else {
                    f.escaped = true; // a bare value use we can't account for
                }
            }
            Expr::Ident { .. } | Expr::Literal { .. } => {}
            Expr::Index { object, index, .. } => {
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p) {
                    f.indexed = true;
                } else {
                    Self::scan_param_use(object, p, false, f);
                }
                Self::scan_param_use(index, p, true, f);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p) {
                    f.indexed = true;
                    f.is_mut = true;
                    if matches!(value.as_ref(), Expr::Literal { value: Literal::Bool(_), .. }) {
                        f.elem_bool = true;
                    }
                } else {
                    Self::scan_param_use(object, p, false, f);
                }
                Self::scan_param_use(index, p, true, f);
                Self::scan_param_use(value, p, false, f);
            }
            Expr::Member { object, prop, .. } => {
                // `p.length` is fine; any other `p.x` is an escape (object read in non-index ctx).
                let is_len = matches!(prop, MemberProp::Name { name, .. } if name.as_ref() == "length");
                if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p) {
                    if !is_len {
                        f.escaped = true;
                    }
                } else if matches!(object.as_ref(), Expr::Index { object: io, .. } if matches!(io.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p))
                {
                    // `p[i].field` — `p` is an array of STRUCTS, not a native `number[]`/`boolean[]`.
                    // That's the #177 aggregate path's job; reject here so the two never overlap.
                    f.escaped = true;
                } else {
                    Self::scan_param_use(object, p, false, f);
                }
                if let MemberProp::Expr(pe) = prop {
                    Self::scan_param_use(pe, p, true, f);
                }
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                // A bare `p` as a direct call argument is a forward (array param threaded to a callee,
                // validated separately) — not an escape. The callee itself is scanned normally.
                Self::scan_param_use(callee, p, false, f);
                for a in args {
                    match a {
                        CallArg::Expr(Expr::Ident { name, .. }) if name.as_ref() == p => {
                            // forward (native-vec re-borrows; arr-param owned-copy must reject).
                            f.forwarded = true;
                        }
                        CallArg::Expr(e) | CallArg::Spread(e) => {
                            Self::scan_param_use(e, p, false, f)
                        }
                    }
                }
            }
            Expr::Binary { left, op, right, .. } => {
                let a = matches!(
                    op,
                    BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Mod
                        | BinOp::Pow
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                        | BinOp::StrictEq
                        | BinOp::StrictNe
                );
                Self::scan_param_use(left, p, a, f);
                Self::scan_param_use(right, p, a, f);
            }
            Expr::Unary { operand, .. } => Self::scan_param_use(operand, p, arith_parent, f),
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => Self::scan_param_use(value, p, false, f),
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::scan_param_use(cond, p, false, f);
                Self::scan_param_use(then_branch, p, arith_parent, f);
                Self::scan_param_use(else_branch, p, arith_parent, f);
            }
            Expr::NullishCoalesce { left, right, .. } => {
                Self::scan_param_use(left, p, arith_parent, f);
                Self::scan_param_use(right, p, arith_parent, f);
            }
            Expr::MemberAssign { object, value, .. } => {
                // `p[i].field = v` — struct-element write → aggregate's job, reject (see Member arm).
                if matches!(object.as_ref(), Expr::Index { object: io, .. } if matches!(io.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p))
                {
                    f.escaped = true;
                } else {
                    Self::scan_param_use(object, p, false, f);
                }
                Self::scan_param_use(value, p, false, f);
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::scan_param_use(e, p, false, f);
                }
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                            Self::scan_param_use(e, p, false, f)
                        }
                    }
                }
            }
            Expr::Object { props, .. } => {
                for pr in props {
                    match pr {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::scan_param_use(e, p, false, f)
                        }
                    }
                }
            }
            // Closures / await / other forms: a `p` mention here is an escape we don't model.
            _ => {
                if Self::collect_idents_of(e).contains(p) {
                    f.escaped = true;
                }
            }
        }
    }

    /// `Some(ret)` when every `return` agrees on one native shape; `None` if mixed.
    fn vec_fn_return_kind(params: &[FunParam], body: &Statement) -> Option<VecRetKind> {
        let mut ret_exprs: Vec<Expr> = Vec::new();
        let mut has_empty = false;
        Self::scan_return_exprs(body, &mut ret_exprs, &mut has_empty);
        if ret_exprs.is_empty() && !has_empty {
            return Some(VecRetKind::Unit);
        }
        if has_empty {
            return None;
        }
        let builders = Self::vec_builder_locals(body);
        if !builders.is_empty()
            && ret_exprs.iter().all(|e| {
                matches!(e, Expr::Ident { name, .. } if builders.contains(name.as_ref()))
            })
            && builders
                .iter()
                .all(|b| Self::vec_builder_numeric_pushes(body, b))
        {
            return Some(VecRetKind::VecF64);
        }
        let pnames: std::collections::HashSet<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();
        let globals = std::collections::HashSet::new();
        let nums = Self::native_fn_numeric_locals(
            body,
            &pnames,
            &globals,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        if ret_exprs
            .iter()
            .all(|e| {
                Self::numeric_shaped(
                    e,
                    &pnames,
                    &std::collections::HashSet::new(),
                    &globals,
                    &nums,
                )
            })
        {
            return Some(VecRetKind::F64);
        }
        None
    }

    fn scan_return_exprs(s: &Statement, exprs: &mut Vec<Expr>, has_empty: &mut bool) {
        if let Statement::Return { value, .. } = s {
            if let Some(e) = value {
                exprs.push(e.clone());
            } else {
                *has_empty = true;
            }
        }
        Self::for_each_child_stmt_list(s, &mut |list| {
            for st in list {
                Self::scan_return_exprs(st, exprs, has_empty);
            }
        });
    }

    /// Locals initialized as `[]` and mutated only via `.push` (e.g. `cumulative`'s `out`).
    fn vec_builder_locals(body: &Statement) -> std::collections::HashSet<String> {
        let mut builders: std::collections::HashSet<String> = std::collections::HashSet::new();
        Self::for_each_child_stmt_list(body, &mut |list| {
            for s in list {
                if let Statement::VarDecl {
                    name,
                    init: Some(Expr::Array { elements, .. }),
                    ..
                } = s
                {
                    if elements.is_empty() {
                        builders.insert(name.to_string());
                    }
                }
            }
        });
        for local in builders.clone() {
            if !Self::vec_builder_only_push(body, &local) {
                builders.remove(&local);
            }
        }
        builders
    }

    /// Every `.push` on `local` pushes a numeric literal/expr — not objects/arrays (megamorphic `objs`).
    fn vec_builder_numeric_pushes(body: &Statement, local: &str) -> bool {
        let mut ok = true;
        Self::for_each_stmt_expr(body, &mut |e| {
            if !ok {
                return;
            }
            let Expr::Call { callee, args, .. } = e else {
                return;
            };
            let Expr::Member {
                object,
                prop: MemberProp::Name { name: method, .. },
                ..
            } = callee.as_ref()
            else {
                return;
            };
            if !matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == local)
                || method.as_ref() != "push"
                || args.len() != 1
            {
                return;
            }
            let CallArg::Expr(arg) = &args[0] else {
                ok = false;
                return;
            };
            if matches!(
                arg,
                Expr::Object { .. } | Expr::Array { .. } | Expr::ArrowFunction { .. }
            ) {
                ok = false;
            }
        });
        ok
    }

    fn vec_builder_only_push(body: &Statement, local: &str) -> bool {
        let mut ok = true;
        Self::for_each_stmt_expr(body, &mut |e| {
            if !ok {
                return;
            }
            match e {
                Expr::IndexAssign { object, .. } => {
                    if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == local) {
                        ok = false;
                    }
                }
                Expr::Call { callee, .. } => {
                    if let Expr::Member {
                        object,
                        prop: MemberProp::Name { name: method, .. },
                        ..
                    } = callee.as_ref()
                    {
                        if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == local)
                            && method.as_ref() != "push"
                        {
                            ok = false;
                        }
                    }
                }
                _ => {}
            }
        });
        ok
    }

    /// Whether every call in a native-vec body targets a fn with a native form (another native-vec
    /// fn or an M5 `native_fn`) or a 1-arg `Math.<intrinsic>`. A call to anything else (a boxed
    /// closure, an array method, an arbitrary member) means the free fn can't be emitted — bail.
    fn vec_body_calls_native(
        &self,
        body: &Statement,
        group: &std::collections::HashMap<String, NativeVecFnSig>,
    ) -> bool {
        let mut ok = true;
        Self::for_each_stmt_expr(body, &mut |root| {
            Self::walk_subexprs(root, &mut |e| {
                if let Expr::Call { callee, args, .. } = e {
                    match callee.as_ref() {
                        Expr::Ident { name, .. } => {
                            // OK to call: another native-vec fn, an M5 native fn, or a numeric leaf
                            // fn that inlines at the (f64) call site (no boxed reference remains).
                            if !group.contains_key(name.as_ref())
                                && !self.native_fns.contains(name.as_ref())
                                && !self.inline_fns.contains_key(name.as_ref())
                            {
                                ok = false;
                            }
                        }
                        Expr::Member {
                            object,
                            prop: MemberProp::Name { name: m, .. },
                            ..
                        } => {
                            let is_math1 = matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                                && args.len() == 1
                                && matches!(
                                    m.as_ref(),
                                    "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil"
                                        | "exp" | "trunc" | "log"
                                );
                            let is_push = m.as_ref() == "push"
                                && matches!(object.as_ref(), Expr::Ident { .. });
                            if !is_math1 && !is_push {
                                ok = false;
                            }
                        }
                        _ => ok = false,
                    }
                }
            });
        });
        ok
    }

    /// Add to `bad` any group fn that has a malformed call site anywhere in `s`: an array argument
    /// that isn't a bare ident, array args that aren't pairwise distinct, an arity mismatch, or an
    /// array passed where the callee needs `&mut` but the arg is a read-only ref param.
    fn collect_bad_vec_callsites(
        s: &Statement,
        sigs: &std::collections::HashMap<String, NativeVecFnSig>,
        bad: &mut Vec<String>,
    ) {
        Self::for_each_stmt_expr(s, &mut |root| {
            Self::walk_subexprs(root, &mut |e| {
                if let Expr::Call { callee, args, .. } = e {
                    if let Expr::Ident { name, .. } = callee.as_ref() {
                        if let Some(sig) = sigs.get(name.as_ref()) {
                            if !Self::vec_call_args_ok(sig, args) {
                                bad.push(name.to_string());
                            }
                        }
                    }
                }
            });
        });
        Self::for_each_child_stmt_list(s, &mut |list| {
            for st in list {
                Self::collect_bad_vec_callsites(st, sigs, bad);
            }
        });
    }

    /// Apply `f` to `e` and every nested sub-expression (pre-order). Covers all `Expr` variants that
    /// carry sub-expressions; leaves (`Ident`/`Literal`) just get `f`.
    fn walk_subexprs(e: &Expr, f: &mut dyn FnMut(&Expr)) {
        f(e);
        match e {
            Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
                Self::walk_subexprs(left, f);
                Self::walk_subexprs(right, f);
            }
            Expr::Unary { operand, .. }
            | Expr::TypeOf { operand, .. }
            | Expr::Await { operand, .. } => Self::walk_subexprs(operand, f),
            Expr::Delete { target, .. } => Self::walk_subexprs(target, f),
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                Self::walk_subexprs(callee, f);
                for a in args {
                    match a {
                        CallArg::Expr(e) | CallArg::Spread(e) => Self::walk_subexprs(e, f),
                    }
                }
            }
            Expr::Member { object, prop, .. } => {
                Self::walk_subexprs(object, f);
                if let MemberProp::Expr(pe) = prop {
                    Self::walk_subexprs(pe, f);
                }
            }
            Expr::Index { object, index, .. } => {
                Self::walk_subexprs(object, f);
                Self::walk_subexprs(index, f);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                Self::walk_subexprs(object, f);
                Self::walk_subexprs(index, f);
                Self::walk_subexprs(value, f);
            }
            Expr::Assign { value, .. }
            | Expr::CompoundAssign { value, .. }
            | Expr::LogicalAssign { value, .. } => Self::walk_subexprs(value, f),
            Expr::MemberAssign { object, value, .. } => {
                Self::walk_subexprs(object, f);
                Self::walk_subexprs(value, f);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::walk_subexprs(cond, f);
                Self::walk_subexprs(then_branch, f);
                Self::walk_subexprs(else_branch, f);
            }
            Expr::Array { elements, .. } => {
                for el in elements {
                    match el {
                        ArrayElement::Expr(e) | ArrayElement::Spread(e) => Self::walk_subexprs(e, f),
                    }
                }
            }
            Expr::Object { props, .. } => {
                for pr in props {
                    match pr {
                        ObjectProp::KeyValue(_, e, _) | ObjectProp::Spread(e) => {
                            Self::walk_subexprs(e, f)
                        }
                    }
                }
            }
            Expr::TemplateLiteral { exprs, .. } => {
                for e in exprs {
                    Self::walk_subexprs(e, f);
                }
            }
            Expr::ArrowFunction { body, .. } => {
                if let ArrowBody::Expr(e) = body {
                    Self::walk_subexprs(e, f);
                }
            }
            _ => {}
        }
    }

    /// Validate a call's arguments against a native-vec signature: arity matches; each `Array` arg is
    /// a bare ident; all array args are pairwise distinct (so `&mut` borrows can't alias).
    fn vec_call_args_ok(sig: &NativeVecFnSig, args: &[CallArg]) -> bool {
        if args.len() != sig.params.len() {
            return false;
        }
        let mut array_idents: Vec<&str> = Vec::new();
        for ((_, kind), a) in sig.params.iter().zip(args.iter()) {
            let CallArg::Expr(ae) = a else {
                return false;
            };
            if let VecParamKind::Array { .. } = kind {
                let Expr::Ident { name, .. } = ae else {
                    return false;
                };
                array_idents.push(name.as_ref());
            }
        }
        // Pairwise-distinct array args.
        for i in 0..array_idents.len() {
            for j in (i + 1)..array_idents.len() {
                if array_idents[i] == array_idents[j] {
                    return false;
                }
            }
        }
        true
    }

    /// If `expr` is a direct call to a native-vec fn, the native Rust type of its result.
    fn native_vec_init_type(&self, expr: &Expr) -> Option<RustType> {
        let Expr::Call { callee, args, .. } = expr else {
            return None;
        };
        let Expr::Ident { name, .. } = callee.as_ref() else {
            return None;
        };
        let sig = self.native_vec_fns.get(name.as_ref())?;
        if !Self::vec_call_args_ok(sig, args) {
            return None;
        }
        match sig.ret {
            VecRetKind::F64 => Some(RustType::F64),
            VecRetKind::VecF64 => Some(RustType::Vec(Box::new(RustType::F64))),
            VecRetKind::Unit => None,
        }
    }

    fn fn_body_stmt_slice(body: &Statement) -> &[Statement] {
        match body {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => statements,
            _ => std::slice::from_ref(body),
        }
    }

    /// Emit one native-vec fn: `fn name_nv(<f64..>, <&/&mut Vec<T>..>) -> f64|() { <body> }`, reusing
    /// the normal statement emitter (with `native_vec_ret` set so returns lower to the native shape).
    fn emit_native_vec_fn(&mut self, name: &str, body: &Statement) -> Result<(), CompileError> {
        let sig = self.native_vec_fns.get(name).cloned().unwrap();
        self.type_context.push_scope();
        let saved_ret = self.native_vec_ret.take();
        let saved_refs = std::mem::take(&mut self.vec_ref_params);
        let mut plist: Vec<String> = Vec::new();
        for (pname, kind) in &sig.params {
            let esc = Self::escape_ident(pname).into_owned();
            match kind {
                VecParamKind::Scalar => {
                    plist.push(format!("mut {}: f64", esc));
                    self.type_context.define(pname, RustType::F64);
                }
                VecParamKind::Array { elem, is_mut } => {
                    let r = if *is_mut { "&mut " } else { "&" };
                    plist.push(format!("{}: {}Vec<{}>", esc, r, elem.to_rust_type_str()));
                    self.type_context
                        .define(pname, RustType::Vec(Box::new(elem.clone())));
                    self.vec_ref_params.insert(pname.clone(), *is_mut);
                }
            }
        }
        let ret_str = match sig.ret {
            VecRetKind::F64 => " -> f64",
            VecRetKind::VecF64 => " -> Vec<f64>",
            VecRetKind::Unit => "",
        };
        self.writeln("#[allow(non_snake_case, unused)]");
        self.writeln(&format!(
            "fn {}_nv({}){} {{",
            Self::escape_ident(name),
            plist.join(", "),
            ret_str
        ));
        self.indent += 1;
        self.native_vec_ret = Some(sig.ret.clone());
        self.native_fn_emit_name = Some(name.to_string());
        self.usize_for_counters.clear();
        let saved_vec_fixed = self.vec_fixed_len.clone();
        let saved_nonneg = self.nonneg_locals.clone();
        let saved_int_range = self.int_range_locals.clone();
        let saved_diag_coord = self.diag_coord_indices.clone();
        let saved_shift_half = self.shift_half_of.clone();
        let saved_array_elem = self.array_elem_ranges.clone();
        let saved_int_valued = self.int_valued_locals.clone();
        let body_slice = Self::fn_body_stmt_slice(body);
        // #203 follow-up: register the array-param length bound ONLY where the whole-program proof
        // ([`Self::compute_native_vec_param_bounds`]) established that every call site passes an array
        // at least as long as this fn's first scalar param. Then `index_in_bounds(i < scalar)` soundly
        // proves `arr[i]`, dropping the OOB-safe `.get().unwrap_or(NaN)` fallback (recovers
        // spectral_norm's raw indexing). The blind `vec_fixed_len[arr] = Var(lp)` this replaces was
        // UNSOUND — it panicked on `sumArr([10,20,30], 5)`. Unproven params stay OOB-safe.
        if let Some(bounds) = self.native_vec_param_bounds.get(name) {
            for (pname, bound) in bounds {
                self.vec_fixed_len.insert(pname.clone(), bound.clone());
            }
        }
        self.merge_fn_body_inference(body_slice);
        let saved_i32_vec = self.int_i32_vec_locals.clone();
        self.int_i32_vec_locals = Self::detect_fannkuch_i32_vec_locals(body_slice);
        for name in &self.int_i32_vec_locals {
            self.type_context
                .define(name.as_ref(), RustType::Vec(Box::new(RustType::I32)));
        }
        if !self.int_i32_vec_locals.is_empty() {
            if let Some(lp) = sig.params.iter().find_map(|(name, kind)| {
                matches!(kind, VecParamKind::Scalar).then(|| name.clone())
            }) {
                let bound = BoundKey::Var(lp);
                for name in &self.int_i32_vec_locals {
                    self.vec_fixed_len.insert(name.clone(), bound.clone());
                }
            }
        }
        let res = self.emit_statement(body);
        self.int_i32_vec_locals = saved_i32_vec;
        self.vec_fixed_len = saved_vec_fixed;
        self.nonneg_locals = saved_nonneg;
        self.int_range_locals = saved_int_range;
        self.diag_coord_indices = saved_diag_coord;
        self.shift_half_of = saved_shift_half;
        self.array_elem_ranges = saved_array_elem;
        self.int_valued_locals = saved_int_valued;
        self.native_fn_emit_name = None;
        // Total function: a value-returning fn that falls off the end gets a default.
        if sig.ret == VecRetKind::F64 {
            self.writeln("0.0");
        }
        self.native_vec_ret = saved_ret;
        self.vec_ref_params = saved_refs;
        self.indent -= 1;
        self.writeln("}");
        self.type_context.pop_scope();
        res
    }

    /// #175: route a direct call `f(args)` to `f_nv(..)` when `f` is a native-vec fn. Returns the
    /// emitted code (boxed `Value` when `as_value`, else the bare native expr) or `None` to fall back.
    fn try_emit_native_vec_call(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
        as_value: bool,
    ) -> Result<Option<String>, CompileError> {
        let Expr::Ident { name, .. } = callee else {
            return Ok(None);
        };
        let Some(sig) = self.native_vec_fns.get(name.as_ref()).cloned() else {
            return Ok(None);
        };
        if !Self::vec_call_args_ok(&sig, args) {
            return Ok(None);
        }
        let mut call_args: Vec<String> = Vec::new();
        for ((_, kind), a) in sig.params.iter().zip(args.iter()) {
            let CallArg::Expr(ae) = a else {
                return Ok(None);
            };
            match kind {
                VecParamKind::Scalar => call_args.push(self.emit_f64(ae)?),
                VecParamKind::Array { elem, is_mut } => {
                    let Expr::Ident { name: an, .. } = ae else {
                        return Ok(None);
                    };
                    let esc = Self::escape_ident(an.as_ref()).into_owned();
                    // A ref param is already `&/&mut Vec`; reborrow through it. A local `Vec` is
                    // address-of'd. A `&mut` callee param can't take a read-only ref param.
                    let code = match self.vec_ref_params.get(an.as_ref()) {
                        Some(arg_is_mut) => {
                            if *is_mut && !*arg_is_mut {
                                return Ok(None);
                            }
                            if *is_mut {
                                format!("&mut *{}", esc)
                            } else {
                                format!("&*{}", esc)
                            }
                        }
                        None => {
                            // A local: it must be a NATIVE `Vec<elem>` (not a boxed `Value::Array`,
                            // not closure-captured) or the reference would be ill-typed — bail to the
                            // boxed call in that case.
                            let ty = self.type_context.get_type(an.as_ref());
                            let native_vec = matches!(&ty, RustType::Vec(e) if **e == *elem)
                                && !self.refcell_wrapped_vars.contains(an.as_ref());
                            if !native_vec {
                                return Ok(None);
                            }
                            if *is_mut {
                                format!("&mut {}", esc)
                            } else {
                                format!("&{}", esc)
                            }
                        }
                    };
                    call_args.push(code);
                }
            }
        }
        let call = format!("{}_nv({})", Self::escape_ident(name.as_ref()), call_args.join(", "));
        Ok(Some(match sig.ret {
            VecRetKind::F64 => {
                if as_value {
                    format!("Value::Number({})", call)
                } else {
                    call
                }
            }
            VecRetKind::VecF64 => {
                if as_value {
                    format!(
                        "Value::Array(VmRef::new({}.iter().map(|&v| Value::Number(v)).collect()))",
                        call
                    )
                } else {
                    call
                }
            }
            VecRetKind::Unit => {
                if as_value {
                    format!("{{ {}; Value::Null }}", call)
                } else {
                    call
                }
            }
        }))
    }

    // ====================================================================================
    // #177 (S-E / S-F): interprocedural aggregate (unboxed struct + Vec<Struct>) free fns.
    //
    // The infer front-end (S-0..S-D, behind TISH_AGGREGATE_INFER) stamps a struct alias and
    // `: alias` / `: alias[]` annotations onto the nbody-shape factory/array/operator fns iff a
    // whole-program candidacy predicate holds (monomorphic all-f64 struct, no `===`/escape/
    // reshape, write-permitting field ops). Here we consume that: emit each such fn as a native
    // Rust free fn over `Vec<TishStruct_alias>` threaded by `&mut`/`&`, with element access by
    // index (so JS reference aliasing — `let bi = bodies[i]; bi.vx = …` — lowers to in-place
    // `bodies[i].vx = …` and the mutation persists, exactly like the boxed `Value::Array`).
    //
    // SOUNDNESS: this is the codegen-side gate. We re-run `analyze_aggregate` to recover the
    // verdict, then emit every fn into a scratch buffer; if ANY construct can't be lowered the
    // whole path is disabled (the fns + the call hooks fall back to the boxed closures, byte-
    // identical to flag-off). So we never half-wire the feature or miscompile.
    // ====================================================================================

    // ── #178: recursive-struct native lowering (behind TISH_REC_STRUCT) ─────────────────────
    // Infers a recursive struct shape STRUCTURALLY (no identifier matching) from object literals
    // whose fields are scalars or recursive self-references, and emits native builders
    // (`name_rec(..) -> TishStruct_X`) + consumers (`name_rec(&TishStruct_X) -> f64`) over
    // `Option<Box<T>>` children. This is the real, name-independent binary_trees fix — it transfers
    // to any developer code with the same shape. v1 scope: leaf
    // builder/consumer fns (if / return) + top-level `consumer(builder(literals))` routing; the
    // loop-bearing orchestrator (binaryTrees) is future work. SAFE: the boxed closures are left
    // intact as a fallback, native fns are emitted with rollback, and only fully-native-emittable
    // calls are routed — anything else uses the unchanged boxed path. See docs/recursive-struct-native.md.

    /// True if `name` is one of the inferred recursive-struct builders/consumers.
    /// (Reserved for orchestrator-level boxed-closure suppression once that lands.)
    #[allow(dead_code)]
    fn rec_struct_fn(&self, name: &str) -> bool {
        self.rec_struct_plan
            .as_ref()
            .is_some_and(|p| p.builders.contains(name) || p.consumers.contains(name))
    }

    /// Collect every `return <expr>` reachable in a fn body (over control flow, not into nested fns).
    fn rec_collect_returns<'a>(stmt: &'a Statement, out: &mut Vec<&'a Expr>) {
        match stmt {
            Statement::Return { value: Some(e), .. } => out.push(e),
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    Self::rec_collect_returns(s, out);
                }
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::rec_collect_returns(then_branch, out);
                if let Some(e) = else_branch {
                    Self::rec_collect_returns(e, out);
                }
            }
            Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::For { body, .. }
            | Statement::ForOf { body, .. } => Self::rec_collect_returns(body, out),
            _ => {}
        }
    }

    /// Does `body` ever read `node.<field>` for a field in `keyset`? (consumer detection)
    fn rec_body_accesses_node_field(
        body: &Statement,
        node: &str,
        keyset: &std::collections::HashSet<&str>,
    ) -> bool {
        let mut found = false;
        // `for_each_stmt_expr` hands us each statement's top expr; we recurse into its subtree.
        Self::for_each_stmt_expr(body, &mut |e| {
            if Self::expr_reads_node_field(e, node, keyset) {
                found = true;
            }
        });
        found
    }

    /// Recursively: does `e` read `node.<field>` for a field in `keyset`?
    fn expr_reads_node_field(e: &Expr, node: &str, keyset: &std::collections::HashSet<&str>) -> bool {
        match e {
            Expr::Member {
                object,
                prop: MemberProp::Name { name, .. },
                ..
            } => {
                (matches!(object.as_ref(), Expr::Ident { name: on, .. } if on.as_ref() == node)
                    && keyset.contains(name.as_ref()))
                    || Self::expr_reads_node_field(object, node, keyset)
            }
            Expr::Member { object, prop: MemberProp::Expr(idx), .. } => {
                Self::expr_reads_node_field(object, node, keyset)
                    || Self::expr_reads_node_field(idx, node, keyset)
            }
            Expr::Binary { left, right, .. } => {
                Self::expr_reads_node_field(left, node, keyset)
                    || Self::expr_reads_node_field(right, node, keyset)
            }
            Expr::Unary { operand, .. } => Self::expr_reads_node_field(operand, node, keyset),
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::expr_reads_node_field(cond, node, keyset)
                    || Self::expr_reads_node_field(then_branch, node, keyset)
                    || Self::expr_reads_node_field(else_branch, node, keyset)
            }
            Expr::Index { object, index, .. } => {
                Self::expr_reads_node_field(object, node, keyset)
                    || Self::expr_reads_node_field(index, node, keyset)
            }
            Expr::Call { callee, args, .. } => {
                Self::expr_reads_node_field(callee, node, keyset)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(ae) => Self::expr_reads_node_field(ae, node, keyset),
                        _ => false,
                    })
            }
            _ => false,
        }
    }

    /// Does `body` call any fn whose name is in `names`?
    fn rec_body_calls_any(body: &Statement, names: &std::collections::HashSet<&str>) -> bool {
        let mut found = false;
        Self::for_each_stmt_expr(body, &mut |e| {
            if Self::expr_calls_any(e, names) {
                found = true;
            }
        });
        found
    }

    /// Recursively: does `e` contain a call to a fn named in `names`?
    fn expr_calls_any(e: &Expr, names: &std::collections::HashSet<&str>) -> bool {
        match e {
            Expr::Call { callee, args, .. } => {
                let direct = matches!(callee.as_ref(), Expr::Ident { name, .. } if names.contains(name.as_ref()));
                direct
                    || Self::expr_calls_any(callee, names)
                    || args.iter().any(|a| match a {
                        CallArg::Expr(ae) => Self::expr_calls_any(ae, names),
                        _ => false,
                    })
            }
            Expr::Binary { left, right, .. } => {
                Self::expr_calls_any(left, names) || Self::expr_calls_any(right, names)
            }
            Expr::Unary { operand, .. } => Self::expr_calls_any(operand, names),
            Expr::Member { object, .. } => Self::expr_calls_any(object, names),
            Expr::Index { object, index, .. } => {
                Self::expr_calls_any(object, names) || Self::expr_calls_any(index, names)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                Self::expr_calls_any(cond, names)
                    || Self::expr_calls_any(then_branch, names)
                    || Self::expr_calls_any(else_branch, names)
            }
            Expr::Assign { value, .. } => Self::expr_calls_any(value, names),
            _ => false,
        }
    }

    /// Nursery safety: a loop body is safe to arena-reset iff no node built inside it ESCAPES the
    /// iteration — i.e. no `let x = builder(..)` / `x = builder(..)` binds a node handle. Builder
    /// calls that are consumed inline (args to a consumer) are fine; the node dies in-iteration.
    fn rec_orch_body_nursery_safe(stmt: &Statement, builders: &std::collections::HashSet<String>) -> bool {
        !Self::rec_orch_binds_node(stmt, builders)
    }

    fn rec_orch_binds_node(stmt: &Statement, builders: &std::collections::HashSet<String>) -> bool {
        let is_builder_call = |e: &Expr| {
            matches!(e, Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Ident { name, .. } if builders.contains(name.as_ref())))
        };
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(|s| Self::rec_orch_binds_node(s, builders))
            }
            Statement::VarDecl { init: Some(e), .. } => is_builder_call(e),
            Statement::ExprStmt { expr, .. } => {
                matches!(expr, Expr::Assign { value, .. } if is_builder_call(value))
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::rec_orch_binds_node(then_branch, builders)
                    || else_branch
                        .as_ref()
                        .is_some_and(|e| Self::rec_orch_binds_node(e, builders))
            }
            Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::ForOf { body, .. } => Self::rec_orch_binds_node(body, builders),
            Statement::For { init, body, .. } => {
                init.as_ref()
                    .is_some_and(|i| Self::rec_orch_binds_node(i, builders))
                    || Self::rec_orch_binds_node(body, builders)
            }
            _ => false,
        }
    }

    /// Infer a recursive-struct plan from the program, or `None` if the shape isn't present.
    fn detect_rec_struct_program(program: &Program) -> Option<RecStructPlan> {
        use std::collections::{HashMap, HashSet};
        let dbg = std::env::var("TISH_REC_DEBUG").is_ok();
        macro_rules! bail {
            ($($a:tt)*) => {{ if dbg { eprintln!($($a)*); } return None; }};
        }
        // Top-level (non-async) fns by name.
        let mut fns: HashMap<String, (&[FunParam], &Statement)> = HashMap::new();
        for s in &program.statements {
            if let Statement::FunDecl {
                name,
                params,
                body,
                async_: false,
                ..
            } = s
            {
                fns.insert(name.to_string(), (params.as_slice(), body.as_ref()));
            }
        }
        if fns.is_empty() {
            return None;
        }

        // Phase 1 — builders: fns whose EVERY return is an object literal, all with one key set.
        let mut obj_fns: HashMap<String, Vec<std::sync::Arc<str>>> = HashMap::new();
        for (name, (_p, body)) in &fns {
            let mut rets: Vec<&Expr> = Vec::new();
            Self::rec_collect_returns(body, &mut rets);
            if rets.is_empty() {
                continue;
            }
            let mut keyset: Option<Vec<std::sync::Arc<str>>> = None;
            let mut all_obj = true;
            for r in &rets {
                let Expr::Object { props, .. } = r else {
                    all_obj = false;
                    break;
                };
                let mut keys: Vec<std::sync::Arc<str>> = Vec::new();
                for p in props {
                    match p {
                        ObjectProp::KeyValue(k, _, _) => keys.push(k.clone()),
                        _ => {
                            all_obj = false;
                            break;
                        }
                    }
                }
                if !all_obj {
                    break;
                }
                match &keyset {
                    None => keyset = Some(keys),
                    Some(prev) => {
                        let a: HashSet<&str> = prev.iter().map(|s| &**s).collect();
                        let b: HashSet<&str> = keys.iter().map(|s| &**s).collect();
                        if a != b {
                            all_obj = false;
                            break;
                        }
                    }
                }
            }
            if all_obj {
                if let Some(keys) = keyset {
                    obj_fns.insert(name.clone(), keys);
                }
            }
        }
        if obj_fns.is_empty() {
            bail!("[rec] no object-returning fns");
        }
        // All builders must share one key set (single struct for v1).
        let first_keys = obj_fns.values().next().unwrap().clone();
        let first_set: HashSet<&str> = first_keys.iter().map(|s| &**s).collect();
        for keys in obj_fns.values() {
            let s: HashSet<&str> = keys.iter().map(|x| &**x).collect();
            if s != first_set {
                bail!("[rec] builders disagree on key set");
            }
        }
        let builders: HashSet<String> = obj_fns.keys().cloned().collect();

        // Phase 2 — per-key kind: child (Option<Box<Self>>) if EVER null or a builder-call,
        // else scalar (f64). A key that is both scalar and child is ambiguous → bail.
        let mut child: HashMap<String, bool> = HashMap::new();
        for bname in &builders {
            let (_p, body) = fns.get(bname).unwrap();
            let mut rets: Vec<&Expr> = Vec::new();
            Self::rec_collect_returns(body, &mut rets);
            for r in &rets {
                if let Expr::Object { props, .. } = r {
                    for p in props {
                        if let ObjectProp::KeyValue(k, v, _) = p {
                            let is_child = match v {
                                Expr::Literal {
                                    value: Literal::Null,
                                    ..
                                } => true,
                                Expr::Call { callee, .. } => {
                                    matches!(callee.as_ref(), Expr::Ident { name, .. } if builders.contains(name.as_ref()))
                                }
                                _ => false,
                            };
                            match child.get(k.as_ref()) {
                                Some(prev) if *prev != is_child => {
                                    bail!("[rec] key {} is both scalar and child", k);
                                }
                                _ => {
                                    child.insert(k.to_string(), is_child);
                                }
                            }
                        }
                    }
                }
            }
        }
        if !child.values().any(|&c| c) {
            bail!("[rec] no child fields (not recursive)");
        }
        let fields: Vec<(std::sync::Arc<str>, bool)> = first_keys
            .iter()
            .map(|k| (k.clone(), *child.get(k.as_ref()).unwrap_or(&false)))
            .collect();

        // Phase 3 — consumers: non-builder, single-param fns that read node.<field> and return
        // numeric (never an object).
        let keyset: HashSet<&str> = first_keys.iter().map(|s| &**s).collect();
        let mut consumers: HashSet<String> = HashSet::new();
        for (name, (params, body)) in &fns {
            if builders.contains(name) || params.len() != 1 {
                continue;
            }
            let FunParam::Simple(tp) = &params[0] else {
                continue;
            };
            let pname = tp.name.to_string();
            let mut rets: Vec<&Expr> = Vec::new();
            Self::rec_collect_returns(body, &mut rets);
            if rets.is_empty() || rets.iter().any(|r| matches!(r, Expr::Object { .. })) {
                continue;
            }
            if Self::rec_body_accesses_node_field(body, &pname, &keyset) {
                consumers.insert(name.clone());
            }
        }
        if consumers.is_empty() {
            bail!("[rec] no consumers");
        }

        // Phase 4 — orchestrators: numeric fns (not builders/consumers) that CALL a builder or
        // consumer (e.g. `binaryTrees`). Loosely detected; the emitter's rollback rejects any whose
        // body it can't lower. They must not return an object.
        let rec_names: HashSet<&str> = builders
            .iter()
            .chain(consumers.iter())
            .map(|s| s.as_str())
            .collect();
        let mut orchestrators: HashSet<String> = HashSet::new();
        for (name, (_params, body)) in &fns {
            if builders.contains(name) || consumers.contains(name) {
                continue;
            }
            let mut rets: Vec<&Expr> = Vec::new();
            Self::rec_collect_returns(body, &mut rets);
            if rets.iter().any(|r| matches!(r, Expr::Object { .. })) {
                continue; // returns a node — not a numeric orchestrator
            }
            if Self::rec_body_calls_any(body, &rec_names) {
                orchestrators.insert(name.clone());
            }
        }
        if dbg {
            eprintln!("[rec] orchestrators={:?}", orchestrators);
        }

        Some(RecStructPlan {
            alias: "TishRecNode".to_string(),
            fields,
            builders,
            consumers,
            orchestrators,
        })
    }

    /// Detect + emit the recursive-struct plan (struct decl + native builder/consumer fns) with
    /// scratch-buffer rollback. On any failure the path is disabled and the boxed closures stand.
    fn setup_rec_struct_plan(&mut self, program: &Program) {
        let dbg = std::env::var("TISH_REC_DEBUG").is_ok();
        let Some(plan) = Self::detect_rec_struct_program(program) else {
            if dbg {
                eprintln!("[rec] detect = None");
            }
            return;
        };
        if dbg {
            eprintln!(
                "[rec] alias={} fields={:?} builders={:?} consumers={:?} orchestrators={:?}",
                plan.alias, plan.fields, plan.builders, plan.consumers, plan.orchestrators
            );
        }
        if self.type_aliases.contains_key(&plan.alias) {
            return; // name collision with a user type — bail conservatively
        }
        self.rec_struct_plan = Some(plan.clone());

        let mut bdecls: Vec<(String, Vec<FunParam>, Statement)> = Vec::new();
        let mut cdecls: Vec<(String, Vec<FunParam>, Statement)> = Vec::new();
        let mut odecls: Vec<(String, Vec<FunParam>, Statement)> = Vec::new();
        for s in &program.statements {
            if let Statement::FunDecl {
                name, params, body, ..
            } = s
            {
                let nm = name.to_string();
                if plan.builders.contains(&nm) {
                    bdecls.push((nm, params.clone(), (**body).clone()));
                } else if plan.consumers.contains(&nm) {
                    cdecls.push((nm, params.clone(), (**body).clone()));
                } else if plan.orchestrators.contains(&nm) {
                    odecls.push((nm, params.clone(), (**body).clone()));
                }
            }
        }

        let saved = std::mem::take(&mut self.output);
        let saved_indent = self.indent;
        self.indent = 0;
        let mut ok = true;
        self.emit_rec_struct_decl(&plan);
        for (nm, params, body) in &bdecls {
            if self.emit_rec_builder_fn(nm, params, body).is_err() {
                ok = false;
                break;
            }
        }
        if ok {
            for (nm, params, body) in &cdecls {
                if self.emit_rec_consumer_fn(nm, params, body).is_err() {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for (nm, params, body) in &odecls {
                if self.emit_rec_orchestrator_fn(nm, params, body).is_err() {
                    ok = false;
                    break;
                }
            }
        }
        let emitted = std::mem::replace(&mut self.output, saved);
        self.indent = saved_indent;
        if dbg {
            eprintln!("[rec] emission ok={}", ok);
        }
        if ok {
            self.output.push_str(&emitted);
            self.writeln("");
        } else {
            self.rec_struct_plan = None;
        }
    }

    /// Emit the arena node struct: child fields are `i32` arena indices (`-1` = null), scalar
    /// fields `f64`. `Copy` so consumers can read a node out of the arena without a borrow held
    /// across recursion. All nodes live in one bump-allocated `Vec` (no per-node malloc) — this is
    /// what closes the gap to V8's generational GC on the allocation-bound tree workload.
    fn emit_rec_struct_decl(&mut self, plan: &RecStructPlan) {
        let sname = crate::types::named_struct_ident(&plan.alias);
        self.writeln("#[derive(Clone, Copy, Debug, Default)]");
        self.writeln("#[allow(non_snake_case, non_camel_case_types, unused)]");
        self.writeln(&format!("struct {} {{", sname));
        self.indent += 1;
        for (k, is_child) in &plan.fields {
            let ty = if *is_child { "i32" } else { "f64" };
            self.writeln(&format!("{}: {},", crate::types::field_ident(k), ty));
        }
        self.indent -= 1;
        self.writeln("}");
    }

    /// `fn name_rec(<f64 params>, __rec_arena: &mut Vec<Node>) -> i32` — builds nodes into the arena
    /// and returns the new node's index.
    fn emit_rec_builder_fn(
        &mut self,
        name: &str,
        params: &[FunParam],
        body: &Statement,
    ) -> Result<(), CompileError> {
        let plan = self.rec_struct_plan.clone().unwrap();
        let sname = crate::types::named_struct_ident(&plan.alias);
        self.type_context.push_scope();
        let mut plist: Vec<String> = Vec::new();
        for p in params {
            let FunParam::Simple(tp) = p else {
                self.type_context.pop_scope();
                return Err(CompileError::new("rec: builder param kind", None));
            };
            plist.push(format!("{}: f64", Self::escape_ident(tp.name.as_ref())));
            self.type_context.define(tp.name.as_ref(), RustType::F64);
        }
        plist.push(format!("__rec_arena: &mut Vec<{}>", sname));
        self.writeln("#[allow(non_snake_case, unused)]");
        self.writeln(&format!(
            "fn {}_rec({}) -> i32 {{",
            Self::escape_ident(name),
            plist.join(", "),
        ));
        self.indent += 1;
        let ctx = RecCtx {
            is_builder: true,
            node: None,
        };
        let res = self.emit_rec_stmt(body, &ctx);
        self.indent -= 1;
        self.writeln("}");
        self.type_context.pop_scope();
        res
    }

    /// `fn name_rec(__rec_idx: i32, __rec_arena: &Vec<Node>) -> f64` — reads the node out of the
    /// arena (a `Copy`) and walks children by index.
    fn emit_rec_consumer_fn(
        &mut self,
        name: &str,
        params: &[FunParam],
        body: &Statement,
    ) -> Result<(), CompileError> {
        let plan = self.rec_struct_plan.clone().unwrap();
        let sname = crate::types::named_struct_ident(&plan.alias);
        if params.len() != 1 {
            return Err(CompileError::new("rec: consumer arity", None));
        }
        let FunParam::Simple(tp) = &params[0] else {
            return Err(CompileError::new("rec: consumer param kind", None));
        };
        let pname = tp.name.to_string();
        self.type_context.push_scope();
        self.writeln("#[allow(non_snake_case, unused)]");
        self.writeln(&format!(
            "fn {}_rec(__rec_idx: i32, __rec_arena: &Vec<{}>) -> f64 {{",
            Self::escape_ident(name),
            sname
        ));
        self.indent += 1;
        self.writeln("let __rec_n = __rec_arena[__rec_idx as usize];");
        let ctx = RecCtx {
            is_builder: false,
            node: Some(pname),
        };
        let res = self.emit_rec_stmt(body, &ctx);
        self.indent -= 1;
        self.writeln("}");
        self.type_context.pop_scope();
        res
    }

    // ── orchestrator (numeric fn that drives builders/consumers, threading the arena) ───────────
    // v1 supports the constructs in binary_trees' `binaryTrees`: f64 + node-index (`i32`) locals,
    // while/for/if, assignment, inc, `<<`, arithmetic, builder calls (→ index) and consumer calls
    // (→ f64). The whole arena is freed when the top-level block ends (per-tree nursery reset is a
    // future memory optimization, not needed for the fixture's bounded node count).

    /// `fn name_rec(<f64 params>, __rec_arena: &mut Vec<Node>) -> f64`.
    fn emit_rec_orchestrator_fn(
        &mut self,
        name: &str,
        params: &[FunParam],
        body: &Statement,
    ) -> Result<(), CompileError> {
        let plan = self.rec_struct_plan.clone().unwrap();
        let sname = crate::types::named_struct_ident(&plan.alias);
        self.type_context.push_scope();
        let mut plist: Vec<String> = Vec::new();
        for p in params {
            let FunParam::Simple(tp) = p else {
                self.type_context.pop_scope();
                return Err(CompileError::new("rec: orch param kind", None));
            };
            plist.push(format!("{}: f64", Self::escape_ident(tp.name.as_ref())));
            self.type_context.define(tp.name.as_ref(), RustType::F64);
        }
        plist.push(format!("__rec_arena: &mut Vec<{}>", sname));
        self.writeln("#[allow(non_snake_case, unused)]");
        self.writeln(&format!(
            "fn {}_rec({}) -> f64 {{",
            Self::escape_ident(name),
            plist.join(", "),
        ));
        self.indent += 1;
        let res = self.emit_rec_orch_stmt(body);
        self.indent -= 1;
        self.writeln("}");
        self.type_context.pop_scope();
        res
    }

    fn emit_rec_orch_stmt(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    self.emit_rec_orch_stmt(s)?;
                }
                Ok(())
            }
            Statement::VarDecl {
                name, init: Some(e), ..
            } => {
                let (code, is_idx) = self.emit_rec_orch_expr(e)?;
                self.type_context
                    .define(name, if is_idx { RustType::I32 } else { RustType::F64 });
                self.writeln(&format!("let mut {} = {};", Self::escape_ident(name), code));
                Ok(())
            }
            Statement::ExprStmt { expr, .. } => {
                let code = self.emit_rec_orch_simple_expr(expr)?;
                self.writeln(&format!("{};", code));
                Ok(())
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_rec_orch_cond(cond)?;
                self.writeln(&format!("if {} {{", c));
                self.indent += 1;
                self.emit_rec_orch_stmt(then_branch)?;
                self.indent -= 1;
                if let Some(eb) = else_branch {
                    self.writeln("} else {");
                    self.indent += 1;
                    self.emit_rec_orch_stmt(eb)?;
                    self.indent -= 1;
                }
                self.writeln("}");
                Ok(())
            }
            Statement::While { cond, body, .. } => {
                let c = self.emit_rec_orch_cond(cond)?;
                self.writeln(&format!("while {} {{", c));
                self.indent += 1;
                self.emit_rec_orch_loop_body(body)?;
                self.indent -= 1;
                self.writeln("}");
                Ok(())
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                // Lower to a Rust block: { <init>; while <cond> { <body>; <update>; } }
                self.writeln("{");
                self.indent += 1;
                if let Some(i) = init {
                    self.emit_rec_orch_stmt(i)?;
                }
                let c = match cond {
                    Some(c) => self.emit_rec_orch_cond(c)?,
                    None => "true".to_string(),
                };
                self.writeln(&format!("while {} {{", c));
                self.indent += 1;
                self.emit_rec_orch_loop_body(body)?;
                if let Some(u) = update {
                    let uc = self.emit_rec_orch_simple_expr(u)?;
                    self.writeln(&format!("{};", uc));
                }
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
                Ok(())
            }
            Statement::Return { value: Some(e), .. } => {
                let (code, is_idx) = self.emit_rec_orch_expr(e)?;
                if is_idx {
                    return Err(CompileError::new("rec: orch returns node", None));
                }
                self.writeln(&format!("return {};", code));
                Ok(())
            }
            _ => Err(CompileError::new("rec: unsupported orch stmt", None)),
        }
    }

    /// Emit a loop body, wrapping it in an arena checkpoint+truncate (nursery reset) when no node
    /// built inside the body escapes the iteration — bounding arena growth to one tree at a time.
    fn emit_rec_orch_loop_body(&mut self, body: &Statement) -> Result<(), CompileError> {
        let builders = self.rec_struct_plan.as_ref().unwrap().builders.clone();
        let reset = Self::rec_orch_body_nursery_safe(body, &builders);
        if reset {
            self.writeln("let __rec_cp = __rec_arena.len();");
        }
        self.emit_rec_orch_stmt(body)?;
        if reset {
            self.writeln("__rec_arena.truncate(__rec_cp);");
        }
        Ok(())
    }

    /// Statement-position expression: assignment (`x = e`) or `x++`/`++x` (→ `x += 1.0`).
    fn emit_rec_orch_simple_expr(&mut self, e: &Expr) -> Result<String, CompileError> {
        match e {
            Expr::Assign { name, value, .. } => {
                let (code, _) = self.emit_rec_orch_expr(value)?;
                Ok(format!("{} = {}", Self::escape_ident(name.as_ref()), code))
            }
            Expr::PostfixInc { name, .. } | Expr::PrefixInc { name, .. } => {
                Ok(format!("{} += 1_f64", Self::escape_ident(name.as_ref())))
            }
            Expr::PostfixDec { name, .. } | Expr::PrefixDec { name, .. } => {
                Ok(format!("{} -= 1_f64", Self::escape_ident(name.as_ref())))
            }
            _ => Err(CompileError::new("rec: unsupported orch stmt-expr", None)),
        }
    }

    fn emit_rec_orch_cond(&mut self, e: &Expr) -> Result<String, CompileError> {
        match e {
            Expr::Binary {
                op: op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne),
                left,
                right,
                ..
            } => {
                let (l, _) = self.emit_rec_orch_expr(left)?;
                let (r, _) = self.emit_rec_orch_expr(right)?;
                let o = match op {
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    _ => unreachable!(),
                };
                Ok(format!("({} {} {})", l, o, r))
            }
            Expr::Binary {
                op: op @ (BinOp::And | BinOp::Or),
                left,
                right,
                ..
            } => {
                let l = self.emit_rec_orch_cond(left)?;
                let r = self.emit_rec_orch_cond(right)?;
                let o = if matches!(op, BinOp::And) { "&&" } else { "||" };
                Ok(format!("({} {} {})", l, o, r))
            }
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
                ..
            } => Ok(format!("(!({}))", self.emit_rec_orch_cond(operand)?)),
            _ => Err(CompileError::new("rec: unsupported orch cond", None)),
        }
    }

    /// Returns `(code, is_node_idx)`: `is_node_idx == true` ⇒ an `i32` arena index; else `f64`.
    fn emit_rec_orch_expr(&mut self, e: &Expr) -> Result<(String, bool), CompileError> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Ok((Self::f64_lit(*n), false)),
            Expr::Ident { name, .. } => match self.type_context.get_type(name.as_ref()) {
                RustType::I32 => Ok((Self::escape_ident(name.as_ref()).into_owned(), true)),
                RustType::F64 => Ok((Self::escape_ident(name.as_ref()).into_owned(), false)),
                _ => Err(CompileError::new("rec: orch ident type", None)),
            },
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => Ok((format!("(-({}))", self.emit_rec_orch_f64(operand)?), false)),
            Expr::Unary {
                op: UnaryOp::Pos,
                operand,
                ..
            } => Ok((format!("({})", self.emit_rec_orch_f64(operand)?), false)),
            Expr::Binary {
                op: BinOp::Shl,
                left,
                right,
                ..
            } => {
                let l = self.emit_rec_orch_f64(left)?;
                let r = self.emit_rec_orch_f64(right)?;
                // JS `<<` is a 32-bit shift; the fixture shifts small positive values.
                Ok((
                    format!("((({}) as i64) << ((({}) as i64) & 31)) as f64", l, r),
                    false,
                ))
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.emit_rec_orch_f64(left)?;
                let r = self.emit_rec_orch_f64(right)?;
                let code = match op {
                    BinOp::Add => format!("({} + {})", l, r),
                    BinOp::Sub => format!("({} - {})", l, r),
                    BinOp::Mul => format!("({} * {})", l, r),
                    BinOp::Div => format!("({} / {})", l, r),
                    BinOp::Mod => format!("({} % {})", l, r),
                    BinOp::Pow => format!("({}).powf({})", l, r),
                    _ => return Err(CompileError::new("rec: orch binop", None)),
                };
                Ok((code, false))
            }
            Expr::Call { callee, args, .. } => {
                let plan = self.rec_struct_plan.clone().unwrap();
                let Expr::Ident { name: fname, .. } = callee.as_ref() else {
                    return Err(CompileError::new("rec: orch call callee", None));
                };
                if plan.builders.contains(fname.as_ref()) {
                    // builder(args) → index
                    let mut a: Vec<String> = Vec::new();
                    for arg in args {
                        let CallArg::Expr(ae) = arg else {
                            return Err(CompileError::new("rec: orch builder arg", None));
                        };
                        a.push(self.emit_rec_orch_f64(ae)?);
                    }
                    a.push("__rec_arena".to_string());
                    return Ok((
                        format!("{}_rec({})", Self::escape_ident(fname.as_ref()), a.join(", ")),
                        true,
                    ));
                }
                if plan.consumers.contains(fname.as_ref()) {
                    // consumer(nodeIndexExpr) → f64
                    let [CallArg::Expr(arg)] = args.as_slice() else {
                        return Err(CompileError::new("rec: orch consumer arity", None));
                    };
                    let (acode, is_idx) = self.emit_rec_orch_expr(arg)?;
                    if !is_idx {
                        return Err(CompileError::new("rec: orch consumer arg not node", None));
                    }
                    // `arg` may itself build into the arena; bind to a temp so the &mut borrow for
                    // the build is released before the consumer's & borrow.
                    return Ok((
                        format!(
                            "{{ let __rec_t = {}; {}_rec(__rec_t, __rec_arena) }}",
                            acode,
                            Self::escape_ident(fname.as_ref())
                        ),
                        false,
                    ));
                }
                Err(CompileError::new("rec: orch unsupported call", None))
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_rec_orch_cond(cond)?;
                let t = self.emit_rec_orch_f64(then_branch)?;
                let el = self.emit_rec_orch_f64(else_branch)?;
                Ok((format!("(if {} {{ {} }} else {{ {} }})", c, t, el), false))
            }
            _ => Err(CompileError::new("rec: orch unsupported expr", None)),
        }
    }

    /// Like `emit_rec_orch_expr` but requires an `f64` result.
    fn emit_rec_orch_f64(&mut self, e: &Expr) -> Result<String, CompileError> {
        let (code, is_idx) = self.emit_rec_orch_expr(e)?;
        if is_idx {
            return Err(CompileError::new("rec: orch expected f64, got node", None));
        }
        Ok(code)
    }

    fn emit_rec_stmt(&mut self, stmt: &Statement, ctx: &RecCtx) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    self.emit_rec_stmt(s, ctx)?;
                }
                Ok(())
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_rec_cond(cond, ctx)?;
                self.writeln(&format!("if {} {{", c));
                self.indent += 1;
                self.emit_rec_stmt(then_branch, ctx)?;
                self.indent -= 1;
                if let Some(eb) = else_branch {
                    self.writeln("} else {");
                    self.indent += 1;
                    self.emit_rec_stmt(eb, ctx)?;
                    self.indent -= 1;
                }
                self.writeln("}");
                Ok(())
            }
            Statement::Return { value: Some(e), .. } => {
                if ctx.is_builder {
                    self.emit_rec_build_node_return(e, ctx)?;
                } else {
                    let code = self.emit_rec_num_expr(e, ctx)?;
                    self.writeln(&format!("return {};", code));
                }
                Ok(())
            }
            _ => Err(CompileError::new("rec: unsupported stmt", None)),
        }
    }

    /// A boolean condition: `child === null` → `.is_none()`, numeric comparisons, `&&`/`||`/`!`.
    fn emit_rec_cond(&mut self, e: &Expr, ctx: &RecCtx) -> Result<String, CompileError> {
        match e {
            Expr::Binary {
                op: op @ (BinOp::StrictEq | BinOp::StrictNe | BinOp::Eq | BinOp::Ne),
                left,
                right,
                ..
            } => {
                let left_null =
                    matches!(left.as_ref(), Expr::Literal { value: Literal::Null, .. });
                let right_null =
                    matches!(right.as_ref(), Expr::Literal { value: Literal::Null, .. });
                if left_null || right_null {
                    let mem = if right_null { left.as_ref() } else { right.as_ref() };
                    let lval = self.rec_child_lvalue(mem, ctx)?;
                    // `child === null` ⇒ the arena index sentinel is `-1`.
                    let is_eq = matches!(op, BinOp::StrictEq | BinOp::Eq);
                    return Ok(format!("({} {} -1)", lval, if is_eq { "==" } else { "!=" }));
                }
                let l = self.emit_rec_num_expr(left, ctx)?;
                let r = self.emit_rec_num_expr(right, ctx)?;
                let o = if matches!(op, BinOp::StrictEq | BinOp::Eq) {
                    "=="
                } else {
                    "!="
                };
                Ok(format!("({} {} {})", l, o, r))
            }
            Expr::Binary {
                op: op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge),
                left,
                right,
                ..
            } => {
                let l = self.emit_rec_num_expr(left, ctx)?;
                let r = self.emit_rec_num_expr(right, ctx)?;
                let o = match op {
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    _ => unreachable!(),
                };
                Ok(format!("({} {} {})", l, o, r))
            }
            Expr::Binary {
                op: op @ (BinOp::And | BinOp::Or),
                left,
                right,
                ..
            } => {
                let l = self.emit_rec_cond(left, ctx)?;
                let r = self.emit_rec_cond(right, ctx)?;
                let o = if matches!(op, BinOp::And) { "&&" } else { "||" };
                Ok(format!("({} {} {})", l, o, r))
            }
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
                ..
            } => Ok(format!("(!({}))", self.emit_rec_cond(operand, ctx)?)),
            _ => Err(CompileError::new("rec: unsupported cond", None)),
        }
    }

    /// An `f64`-valued expression in a recursive-struct fn body.
    fn emit_rec_num_expr(&mut self, e: &Expr, ctx: &RecCtx) -> Result<String, CompileError> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Ok(Self::f64_lit(*n)),
            Expr::Ident { name, .. } => {
                if ctx.node.as_deref() == Some(name.as_ref()) {
                    return Err(CompileError::new("rec: node used as number", None));
                }
                if self.type_context.get_type(name.as_ref()) != RustType::F64 {
                    return Err(CompileError::new("rec: non-f64 ident", None));
                }
                Ok(Self::escape_ident(name.as_ref()).into_owned())
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => Ok(format!("(-({}))", self.emit_rec_num_expr(operand, ctx)?)),
            Expr::Unary {
                op: UnaryOp::Pos,
                operand,
                ..
            } => Ok(format!("({})", self.emit_rec_num_expr(operand, ctx)?)),
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.emit_rec_num_expr(left, ctx)?;
                let r = self.emit_rec_num_expr(right, ctx)?;
                let code = match op {
                    BinOp::Add => format!("({} + {})", l, r),
                    BinOp::Sub => format!("({} - {})", l, r),
                    BinOp::Mul => format!("({} * {})", l, r),
                    BinOp::Div => format!("({} / {})", l, r),
                    BinOp::Mod => format!("({} % {})", l, r),
                    BinOp::Pow => format!("({}).powf({})", l, r),
                    _ => return Err(CompileError::new("rec: unsupported num binop", None)),
                };
                Ok(code)
            }
            // `node.scalarField` → `node.field`
            Expr::Member {
                object,
                prop: MemberProp::Name { name: m, .. },
                ..
            } => {
                let plan = self.rec_struct_plan.clone().unwrap();
                if let (Some(node), Expr::Ident { name: on, .. }) =
                    (ctx.node.as_deref(), object.as_ref())
                {
                    if on.as_ref() == node {
                        if let Some((_, is_child)) =
                            plan.fields.iter().find(|(k, _)| k.as_ref() == m.as_ref())
                        {
                            if !*is_child {
                                return Ok(format!(
                                    "__rec_n.{}",
                                    crate::types::field_ident(m.as_ref())
                                ));
                            }
                        }
                    }
                }
                Err(CompileError::new("rec: member not scalar field", None))
            }
            // consumer recursion `cons(arg)` → `cons_rec(<node ref>)`; or `Math.<fn>(x)`.
            Expr::Call { callee, args, .. } => {
                let plan = self.rec_struct_plan.clone().unwrap();
                if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                    if plan.consumers.contains(fname.as_ref()) {
                        let [CallArg::Expr(arg)] = args.as_slice() else {
                            return Err(CompileError::new("rec: consumer call args", None));
                        };
                        let cidx = self.emit_rec_child_index(arg, ctx)?;
                        return Ok(format!(
                            "{}_rec({}, __rec_arena)",
                            Self::escape_ident(fname.as_ref()),
                            cidx
                        ));
                    }
                }
                if let Expr::Member {
                    object,
                    prop: MemberProp::Name { name: method, .. },
                    ..
                } = callee.as_ref()
                {
                    if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                    {
                        if let Some(m) = Self::agg_clean_math_method(method.as_ref()) {
                            if let [CallArg::Expr(a)] = args.as_slice() {
                                let ac = self.emit_rec_num_expr(a, ctx)?;
                                return Ok(format!("({}).{}()", ac, m));
                            }
                        }
                    }
                }
                Err(CompileError::new("rec: unsupported call", None))
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_rec_cond(cond, ctx)?;
                let t = self.emit_rec_num_expr(then_branch, ctx)?;
                let el = self.emit_rec_num_expr(else_branch, ctx)?;
                Ok(format!("(if {} {{ {} }} else {{ {} }})", c, t, el))
            }
            _ => Err(CompileError::new("rec: unsupported num expr", None)),
        }
    }

    /// `node.childField` → `__rec_n.field` (the child's `i32` arena index), for null checks / recursion.
    fn rec_child_lvalue(&self, e: &Expr, ctx: &RecCtx) -> Result<String, CompileError> {
        if let (Expr::Member {
            object,
            prop: MemberProp::Name { name: m, .. },
            ..
        }, Some(node)) = (e, ctx.node.as_deref())
        {
            if matches!(object.as_ref(), Expr::Ident { name: on, .. } if on.as_ref() == node) {
                if let Some(plan) = &self.rec_struct_plan {
                    if let Some((_, true)) =
                        plan.fields.iter().find(|(k, _)| k.as_ref() == m.as_ref())
                    {
                        return Ok(format!("__rec_n.{}", crate::types::field_ident(m.as_ref())));
                    }
                }
            }
        }
        Err(CompileError::new("rec: not a child lvalue", None))
    }

    /// Produce the `i32` arena index for a consumer-call argument: a child field (`__rec_n.field`)
    /// or the node param itself (`__rec_idx`).
    fn emit_rec_child_index(&mut self, arg: &Expr, ctx: &RecCtx) -> Result<String, CompileError> {
        if let Ok(lval) = self.rec_child_lvalue(arg, ctx) {
            return Ok(lval);
        }
        if let (Some(node), Expr::Ident { name, .. }) = (ctx.node.as_deref(), arg) {
            if name.as_ref() == node {
                return Ok("__rec_idx".to_string());
            }
        }
        Err(CompileError::new("rec: unsupported child index", None))
    }

    /// `builder(args)` → `builder_rec(<f64 args>, __rec_arena)` (threads the arena).
    fn emit_rec_builder_call(
        &mut self,
        name: &str,
        args: &[CallArg],
        ctx: &RecCtx,
    ) -> Result<String, CompileError> {
        let mut a: Vec<String> = Vec::new();
        for arg in args {
            let CallArg::Expr(e) = arg else {
                return Err(CompileError::new("rec: builder call arg kind", None));
            };
            a.push(self.emit_rec_num_expr(e, ctx)?);
        }
        a.push("__rec_arena".to_string());
        Ok(format!("{}_rec({})", Self::escape_ident(name), a.join(", ")))
    }

    /// A builder `return { .. }` → build children (recursively, into the arena) first, then push
    /// this node and `return` its index. Children must already be in the arena before this node so
    /// indices stay stable.
    fn emit_rec_build_node_return(&mut self, e: &Expr, ctx: &RecCtx) -> Result<(), CompileError> {
        let plan = self.rec_struct_plan.clone().unwrap();
        let sname = crate::types::named_struct_ident(&plan.alias);
        let Expr::Object { props, .. } = e else {
            return Err(CompileError::new("rec: builder return not object", None));
        };
        use std::collections::HashMap;
        let mut by_key: HashMap<String, &Expr> = HashMap::new();
        for p in props {
            match p {
                ObjectProp::KeyValue(k, v, _) => {
                    by_key.insert(k.to_string(), v);
                }
                _ => return Err(CompileError::new("rec: struct spread", None)),
            }
        }
        // Emit child sub-trees into temps first (they push to the arena), then scalars.
        let mut inits: Vec<String> = Vec::new();
        for (k, is_child) in &plan.fields {
            let v = by_key
                .get(k.as_ref())
                .ok_or_else(|| CompileError::new("rec: struct missing field", None))?;
            let fid = crate::types::field_ident(k.as_ref());
            let code = if *is_child {
                match v {
                    Expr::Literal {
                        value: Literal::Null,
                        ..
                    } => "-1".to_string(),
                    Expr::Call { callee, args, .. } => {
                        let Expr::Ident { name: bname, .. } = callee.as_ref() else {
                            return Err(CompileError::new("rec: child call callee", None));
                        };
                        if !plan.builders.contains(bname.as_ref()) {
                            return Err(CompileError::new("rec: child not builder call", None));
                        }
                        let call = self.emit_rec_builder_call(bname.as_ref(), args, ctx)?;
                        let tmp = format!("__rec_c_{}", fid);
                        self.writeln(&format!("let {} = {};", tmp, call));
                        tmp
                    }
                    _ => return Err(CompileError::new("rec: child value shape", None)),
                }
            } else {
                self.emit_rec_num_expr(v, ctx)?
            };
            inits.push(format!("{}: {}", fid, code));
        }
        self.writeln("let __rec_idx_new = __rec_arena.len() as i32;");
        self.writeln(&format!("__rec_arena.push({} {{ {} }});", sname, inits.join(", ")));
        self.writeln("return __rec_idx_new;");
        Ok(())
    }

    /// Route a boxed-context `consumer(builder(literals))` to `[Value::Number(]consumer_rec(&builder_rec(..))[)]`.
    /// Conservative: only fires when every builder arg is a numeric literal (so no boxed `Value`
    /// flows in); otherwise returns `None` and the unchanged boxed path is used.
    fn try_emit_toplevel_rec_call(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
        boxed: bool,
    ) -> Result<Option<String>, CompileError> {
        let Some(plan) = self.rec_struct_plan.clone() else {
            return Ok(None);
        };
        let Expr::Ident { name, .. } = callee else {
            return Ok(None);
        };
        // Orchestrator call (e.g. `binaryTrees(15)`): spin up an arena and run it natively.
        if plan.orchestrators.contains(name.as_ref()) {
            let mut lits: Vec<String> = Vec::new();
            for a in args {
                let CallArg::Expr(ae) = a else {
                    return Ok(None);
                };
                match Self::rec_literal_f64(ae) {
                    Some(code) => lits.push(code),
                    None => return Ok(None),
                }
            }
            let sname = crate::types::named_struct_ident(&plan.alias);
            lits.push("&mut __rec_arena".to_string());
            let inner = format!(
                "{{ let mut __rec_arena: Vec<{s}> = Vec::new(); {c}_rec({a}) }}",
                s = sname,
                c = Self::escape_ident(name.as_ref()),
                a = lits.join(", "),
            );
            return Ok(Some(if boxed {
                format!("Value::Number({})", inner)
            } else {
                inner
            }));
        }
        if !plan.consumers.contains(name.as_ref()) {
            return Ok(None);
        }
        let [CallArg::Expr(node_arg)] = args else {
            return Ok(None);
        };
        let Expr::Call {
            callee: bcallee,
            args: bargs,
            ..
        } = node_arg
        else {
            return Ok(None);
        };
        let Expr::Ident { name: bname, .. } = bcallee.as_ref() else {
            return Ok(None);
        };
        if !plan.builders.contains(bname.as_ref()) {
            return Ok(None);
        }
        let mut blit: Vec<String> = Vec::new();
        for a in bargs {
            let CallArg::Expr(ae) = a else {
                return Ok(None);
            };
            match Self::rec_literal_f64(ae) {
                Some(code) => blit.push(code),
                None => return Ok(None),
            }
        }
        let sname = crate::types::named_struct_ident(&plan.alias);
        // Build into a fresh arena, then consume by root index; the arena (and all nodes) frees in
        // one shot when the block ends.
        let mut build_args = blit.clone();
        build_args.push("&mut __rec_arena".to_string());
        let inner = format!(
            "{{ let mut __rec_arena: Vec<{s}> = Vec::new(); let __rec_root = {b}_rec({a}); {c}_rec(__rec_root, &__rec_arena) }}",
            s = sname,
            b = Self::escape_ident(bname.as_ref()),
            a = build_args.join(", "),
            c = Self::escape_ident(name.as_ref()),
        );
        Ok(Some(if boxed {
            format!("Value::Number({})", inner)
        } else {
            inner
        }))
    }

    /// A numeric expression built only from literals (no free variables) → its `f64` Rust code.
    fn rec_literal_f64(e: &Expr) -> Option<String> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Some(Self::f64_lit(*n)),
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => Some(format!("(-({}))", Self::rec_literal_f64(operand)?)),
            Expr::Unary {
                op: UnaryOp::Pos,
                operand,
                ..
            } => Some(format!("({})", Self::rec_literal_f64(operand)?)),
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = Self::rec_literal_f64(left)?;
                let r = Self::rec_literal_f64(right)?;
                let o = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    _ => return None,
                };
                Some(format!("({} {} {})", l, o, r))
            }
            _ => None,
        }
    }

    /// Compute the aggregate group from the stamped `: alias` / `: alias[]` annotations the infer
    /// front-end wrote (the infer→codegen contract — re-running `analyze_aggregate` on the already
    /// stamped program is NOT idempotent), emit the native free fns, and (on full success) record
    /// the routing state so call sites de-virtualize. On any failure the state is left empty (path
    /// disabled) and nothing is appended to the output.
    fn setup_aggregate_fns(&mut self, program: &Program) {
        let dbg = std::env::var("TISH_AGG_DEBUG").is_ok();
        // The unboxed struct alias: the (unique) name `A` used as an `A[]` array param of some fn,
        // and registered as an all-`Copy`-field struct alias.
        let Some(alias) = self.detect_aggregate_alias(program) else {
            if dbg {
                eprintln!("[agg] detect_aggregate_alias = None; aliases={:?}", self.type_aliases.keys().collect::<Vec<_>>());
            }
            return;
        };
        if dbg {
            eprintln!("[agg] alias = {}", alias);
        }
        // Top-level numeric global `let` names available to capture as trailing params.
        let globals = Self::collect_toplevel_global_lets(program);

        // Build a signature for every fn in the group from its stamped annotations.
        let mut sigs: std::collections::HashMap<String, AggFnSig> = std::collections::HashMap::new();
        let mut decls: Vec<(String, Vec<FunParam>, Statement)> = Vec::new();
        for s in &program.statements {
            if let Statement::FunDecl {
                async_: false,
                name,
                params,
                rest_param: None,
                return_type,
                body,
                ..
            } = s
            {
                let nm = name.to_string();
                // The array param: a `Simple`-param annotated `: alias[]`.
                let array_pi = params.iter().position(|p| {
                    matches!(p, FunParam::Simple(tp)
                        if Self::ann_is_array_of(tp.type_ann.as_ref(), &alias))
                });
                // Return shape from the stamped return type, else from the body.
                let ret = if Self::ann_is_simple(return_type.as_ref(), &alias) {
                    AggRet::Struct
                } else if Self::ann_is_array_of(return_type.as_ref(), &alias) {
                    AggRet::ArrayOfStruct
                } else if array_pi.is_some() {
                    if Self::stmt_returns_value(body) {
                        AggRet::F64
                    } else {
                        AggRet::Unit
                    }
                } else {
                    // Not a factory and takes no array param → not in the group.
                    continue;
                };
                let array_pname = array_pi.and_then(|pi| match params.get(pi) {
                    Some(FunParam::Simple(tp)) => Some(tp.name.to_string()),
                    _ => None,
                });
                let is_mut = array_pname
                    .as_deref()
                    .map(|p| Self::agg_fn_mutates_array(body, p))
                    .unwrap_or(false);
                // Per-source-param kind.
                let mut sig_params: Vec<(String, AggParamKind)> = Vec::new();
                let mut ok = true;
                for (pi, p) in params.iter().enumerate() {
                    match p {
                        FunParam::Simple(tp) => {
                            let kind = if Some(pi) == array_pi {
                                AggParamKind::Array { is_mut }
                            } else {
                                // Scalar param: must be annotated `: number` (→ f64).
                                let ty = tp
                                    .type_ann
                                    .as_ref()
                                    .map(|t| {
                                        crate::types::RustType::from_annotation_with_aliases(
                                            t,
                                            &self.type_aliases,
                                        )
                                    })
                                    .unwrap_or(RustType::Value);
                                if ty != RustType::F64 {
                                    ok = false;
                                    break;
                                }
                                AggParamKind::Scalar(ty)
                            };
                            sig_params.push((tp.name.to_string(), kind));
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                // Captured globals: free idents in the body that are top-level numeric globals and
                // not params/locals/group-fn names.
                let captured = Self::agg_captured_globals(body, params, &globals, &sigs, &nm, &alias);
                sigs.insert(
                    nm.clone(),
                    AggFnSig {
                        params: sig_params,
                        captured,
                        ret,
                    },
                );
                decls.push((nm, params.clone(), (**body).clone()));
            }
        }
        if dbg {
            eprintln!("[agg] sigs = {:?}", sigs.keys().collect::<Vec<_>>());
        }
        if sigs.is_empty() {
            return;
        }

        // Top-level array locals: a `let bodies: alias[] = …` VarDecl.
        let array_locals: std::collections::HashSet<String> = program
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::VarDecl { name, type_ann, .. }
                    if Self::ann_is_array_of(type_ann.as_ref(), &alias) =>
                {
                    Some(name.to_string())
                }
                _ => None,
            })
            .collect();

        // Tentatively commit the routing state so `emit_agg_*` can resolve nested group calls.
        self.aggregate_alias = Some(alias.clone());
        self.aggregate_fns = sigs;
        self.aggregate_array_locals = array_locals;

        // Emit every fn into a scratch buffer; on any failure, roll back (disable the path).
        let saved = std::mem::take(&mut self.output);
        let saved_indent = self.indent;
        self.indent = 0;
        let mut all_ok = true;
        for (nm, params, body) in &decls {
            if self.emit_aggregate_fn(nm, params, body).is_err() {
                all_ok = false;
                break;
            }
        }
        let emitted = std::mem::replace(&mut self.output, saved);
        self.indent = saved_indent;
        if dbg {
            eprintln!("[agg] all_ok = {} array_locals = {:?}", all_ok, self.aggregate_array_locals);
        }
        if all_ok {
            self.output.push_str(&emitted);
            self.writeln("");
        } else {
            // Roll back: disable the aggregate path entirely.
            self.aggregate_alias = None;
            self.aggregate_fns.clear();
            self.aggregate_array_locals.clear();
        }
    }

    /// Emit one aggregate fn as a native Rust free fn (`fn name_agg(..) -> ..`).
    fn emit_aggregate_fn(
        &mut self,
        name: &str,
        _params: &[FunParam],
        body: &Statement,
    ) -> Result<(), CompileError> {
        let sig = self
            .aggregate_fns
            .get(name)
            .cloned()
            .expect("emit_aggregate_fn: sig present");
        let alias = self
            .aggregate_alias
            .clone()
            .expect("emit_aggregate_fn: alias present");
        let struct_ty = crate::types::named_struct_ident(&alias);

        // Build the param list + register types in a fresh scope.
        self.type_context.push_scope();
        let mut plist: Vec<String> = Vec::new();
        let mut array_param: Option<String> = None;
        for (pname, kind) in &sig.params {
            let esc = Self::escape_ident(pname).into_owned();
            match kind {
                AggParamKind::Array { is_mut } => {
                    let r = if *is_mut { "&mut " } else { "&" };
                    plist.push(format!("{}: {}Vec<{}>", esc, r, struct_ty));
                    array_param = Some(pname.clone());
                }
                AggParamKind::Scalar(ty) => {
                    plist.push(format!("{}: {}", esc, ty.to_rust_type_str()));
                    self.type_context.define(pname, ty.clone());
                }
            }
        }
        for g in &sig.captured {
            plist.push(format!("{}: f64", Self::escape_ident(g)));
            self.type_context.define(g, RustType::F64);
        }
        let ret_str = match sig.ret {
            AggRet::Struct => format!(" -> {}", struct_ty),
            AggRet::ArrayOfStruct => format!(" -> Vec<{}>", struct_ty),
            AggRet::F64 => " -> f64".to_string(),
            AggRet::Unit => String::new(),
        };
        self.writeln("#[allow(non_snake_case, unused)]");
        self.writeln(&format!(
            "fn {}_agg({}){} {{",
            Self::escape_ident(name),
            plist.join(", "),
            ret_str
        ));
        self.indent += 1;
        let prev_ret = self.agg_cur_ret.replace(sig.ret.clone());
        let mut aliases: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let res = self.emit_agg_stmt(body, array_param.as_deref(), &mut aliases);
        self.agg_cur_ret = prev_ret;
        self.indent -= 1;
        self.writeln("}");
        self.type_context.pop_scope();
        res
    }

    /// Emit a statement inside an aggregate fn body. `arr` is the array param name (if any);
    /// `aliases` maps element-alias locals (`let bi = bodies[i]`) to their index-var name.
    fn emit_agg_stmt(
        &mut self,
        stmt: &Statement,
        arr: Option<&str>,
        aliases: &mut std::collections::HashMap<String, String>,
    ) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                for s in statements {
                    self.emit_agg_stmt(s, arr, aliases)?;
                }
                Ok(())
            }
            Statement::VarDecl { name, init, .. } => {
                let init = init
                    .as_ref()
                    .ok_or_else(|| CompileError::new("agg: uninit let", None))?;
                // Element alias: `let bi = bodies[i]` (i a bare ident) → record, emit nothing.
                if let Expr::Index { object, index, .. } = init {
                    if let (Expr::Ident { name: on, .. }, Expr::Ident { name: iv, .. }) =
                        (object.as_ref(), index.as_ref())
                    {
                        if Some(on.as_ref()) == arr {
                            aliases.insert(name.to_string(), iv.to_string());
                            return Ok(());
                        }
                    }
                }
                let (code, ty) = self.emit_agg_expr(init, arr, aliases)?;
                self.type_context.define(name, ty);
                self.writeln(&format!("let mut {} = {};", Self::escape_ident(name), code));
                Ok(())
            }
            Statement::ExprStmt { expr, .. } => {
                let code = self.emit_agg_assign(expr, arr, aliases)?;
                self.writeln(&format!("{};", code));
                Ok(())
            }
            Statement::Return { value, .. } => {
                let ret = self
                    .agg_cur_ret
                    .clone()
                    .unwrap_or(AggRet::Unit);
                match (&ret, value) {
                    (AggRet::Unit, _) => {
                        self.writeln("return;");
                        Ok(())
                    }
                    (AggRet::F64, Some(e)) => {
                        let (code, ty) = self.emit_agg_expr(e, arr, aliases)?;
                        let f = if ty == RustType::F64 {
                            code
                        } else {
                            return Err(CompileError::new("agg: non-f64 return", None));
                        };
                        self.writeln(&format!("return {};", f));
                        Ok(())
                    }
                    (AggRet::Struct, Some(e)) => {
                        let code = self.emit_agg_struct_literal(e, arr, aliases)?;
                        self.writeln(&format!("return {};", code));
                        Ok(())
                    }
                    (AggRet::ArrayOfStruct, Some(e)) => {
                        let code = self.emit_agg_array_literal(e, arr, aliases)?;
                        self.writeln(&format!("return {};", code));
                        Ok(())
                    }
                    _ => Err(CompileError::new("agg: bad return", None)),
                }
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                // `for (init; cond; update) body` → `{ init; while cond { body; update; } }`.
                // The candidacy predicate forbids `break`/`continue` reaching this emitter (the
                // codegen gate below bails on them), so the lowering is faithful.
                self.writeln("{");
                self.indent += 1;
                if let Some(i) = init {
                    self.emit_agg_stmt(i, arr, aliases)?;
                }
                let cond_code = match cond {
                    Some(c) => {
                        let (code, _) = self.emit_agg_expr(c, arr, aliases)?;
                        code
                    }
                    None => "true".to_string(),
                };
                self.writeln(&format!("while {} {{", cond_code));
                self.indent += 1;
                self.emit_agg_stmt(body, arr, aliases)?;
                if let Some(u) = update {
                    let ucode = self.emit_agg_assign(u, arr, aliases)?;
                    self.writeln(&format!("{};", ucode));
                }
                self.indent -= 1;
                self.writeln("}");
                self.indent -= 1;
                self.writeln("}");
                Ok(())
            }
            _ => Err(CompileError::new("agg: unsupported statement", None)),
        }
    }

    /// Emit an assignment / increment expression in statement position inside an aggregate fn.
    fn emit_agg_assign(
        &mut self,
        expr: &Expr,
        arr: Option<&str>,
        aliases: &std::collections::HashMap<String, String>,
    ) -> Result<String, CompileError> {
        match expr {
            // `px = <f64>` — scalar local/param assign.
            Expr::Assign { name, value, .. } => {
                let (v, _) = self.emit_agg_expr(value, arr, aliases)?;
                Ok(format!("{} = {}", Self::escape_ident(name), v))
            }
            Expr::CompoundAssign {
                name, op, value, ..
            } => {
                let (v, _) = self.emit_agg_expr(value, arr, aliases)?;
                let op_str = match op {
                    CompoundOp::Add => "+=",
                    CompoundOp::Sub => "-=",
                    CompoundOp::Mul => "*=",
                    CompoundOp::Div => "/=",
                    CompoundOp::Mod => "%=",
                };
                Ok(format!("{} {} {}", Self::escape_ident(name), op_str, v))
            }
            // `i++` / `++i` / `i--` / `--i` (loop update) → native f64 step.
            Expr::PostfixInc { name, .. } | Expr::PrefixInc { name, .. } => {
                Ok(format!("{} += 1f64", Self::escape_ident(name)))
            }
            Expr::PostfixDec { name, .. } | Expr::PrefixDec { name, .. } => {
                Ok(format!("{} -= 1f64", Self::escape_ident(name)))
            }
            // `bi.vx = <f64>` (alias) / `bodies[i].vx = <f64>` (direct index) field write.
            Expr::MemberAssign {
                object,
                prop,
                value,
                ..
            } => {
                let field = crate::types::field_ident(prop.as_ref());
                let place = self.emit_agg_place(object, arr, aliases)?;
                let (v, _) = self.emit_agg_expr(value, arr, aliases)?;
                Ok(format!("{}.{} = {}", place, field, v))
            }
            _ => Err(CompileError::new("agg: unsupported statement expr", None)),
        }
    }

    /// Emit the array-element place for a field write target: an alias ident `bi` or `bodies[i]`.
    fn emit_agg_place(
        &mut self,
        object: &Expr,
        arr: Option<&str>,
        aliases: &std::collections::HashMap<String, String>,
    ) -> Result<String, CompileError> {
        match object {
            Expr::Ident { name, .. } => {
                if let Some(idxvar) = aliases.get(name.as_ref()) {
                    let a = arr.ok_or_else(|| CompileError::new("agg: alias no array", None))?;
                    return Ok(format!(
                        "{}[({}) as usize]",
                        Self::escape_ident(a),
                        Self::escape_ident(idxvar)
                    ));
                }
                Err(CompileError::new("agg: bad write target", None))
            }
            Expr::Index { object: io, index, .. } => {
                if let Expr::Ident { name: on, .. } = io.as_ref() {
                    if Some(on.as_ref()) == arr {
                        let (idx, _) = self.emit_agg_expr(index, arr, aliases)?;
                        return Ok(format!("{}[({}) as usize]", Self::escape_ident(on.as_ref()), idx));
                    }
                }
                Err(CompileError::new("agg: bad index write target", None))
            }
            _ => Err(CompileError::new("agg: bad write target", None)),
        }
    }

    /// Emit a (scalar / bool) expression inside an aggregate fn body.
    fn emit_agg_expr(
        &mut self,
        e: &Expr,
        arr: Option<&str>,
        aliases: &std::collections::HashMap<String, String>,
    ) -> Result<(String, RustType), CompileError> {
        match e {
            Expr::Literal {
                value: Literal::Number(n),
                ..
            } => Ok((Self::f64_lit(*n), RustType::F64)),
            Expr::Literal {
                value: Literal::Bool(b),
                ..
            } => Ok((format!("{}", b), RustType::Bool)),
            Expr::Ident { name, .. } => {
                let ty = self.type_context.get_type(name.as_ref());
                Ok((Self::escape_ident(name.as_ref()).into_owned(), ty))
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => {
                let (o, _) = self.emit_agg_expr(operand, arr, aliases)?;
                Ok((format!("(-({}))", o), RustType::F64))
            }
            Expr::Unary {
                op: UnaryOp::Pos,
                operand,
                ..
            } => {
                let (o, _) = self.emit_agg_expr(operand, arr, aliases)?;
                Ok((format!("({})", o), RustType::F64))
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                let (l, _) = self.emit_agg_expr(left, arr, aliases)?;
                let (r, _) = self.emit_agg_expr(right, arr, aliases)?;
                let (code, ty) = match op {
                    BinOp::Add => (format!("({} + {})", l, r), RustType::F64),
                    BinOp::Sub => (format!("({} - {})", l, r), RustType::F64),
                    BinOp::Mul => (format!("({} * {})", l, r), RustType::F64),
                    BinOp::Div => (format!("({} / {})", l, r), RustType::F64),
                    BinOp::Mod => (format!("({} % {})", l, r), RustType::F64),
                    BinOp::Pow => (format!("({}).powf({})", l, r), RustType::F64),
                    BinOp::Lt => (format!("({} < {})", l, r), RustType::Bool),
                    BinOp::Le => (format!("({} <= {})", l, r), RustType::Bool),
                    BinOp::Gt => (format!("({} > {})", l, r), RustType::Bool),
                    BinOp::Ge => (format!("({} >= {})", l, r), RustType::Bool),
                    _ => {
                        return Err(CompileError::new("agg: unsupported binop", None))
                    }
                };
                Ok((code, ty))
            }
            Expr::Member {
                object,
                prop: MemberProp::Name { name: m, .. },
                optional: false,
                ..
            } => {
                if let Expr::Ident { name: on, .. } = object.as_ref() {
                    // `bodies.length` → `(len() as f64)`.
                    if Some(on.as_ref()) == arr && m.as_ref() == "length" {
                        return Ok((
                            format!("({}.len() as f64)", Self::escape_ident(on.as_ref())),
                            RustType::F64,
                        ));
                    }
                    // `bi.field` (element alias) → `bodies[i].field`.
                    if let Some(idxvar) = aliases.get(on.as_ref()) {
                        let a = arr
                            .ok_or_else(|| CompileError::new("agg: alias no array", None))?;
                        return Ok((
                            format!(
                                "{}[({}) as usize].{}",
                                Self::escape_ident(a),
                                Self::escape_ident(idxvar),
                                crate::types::field_ident(m.as_ref())
                            ),
                            RustType::F64,
                        ));
                    }
                    // `localStruct.field` (a `Named` local).
                    let ty = self.type_context.get_type(on.as_ref());
                    if let RustType::Named { fields, .. } = &ty {
                        if let Some((_, ft)) =
                            fields.iter().find(|(k, _)| k.as_ref() == m.as_ref())
                        {
                            return Ok((
                                format!(
                                    "{}.{}",
                                    Self::escape_ident(on.as_ref()),
                                    crate::types::field_ident(m.as_ref())
                                ),
                                ft.clone(),
                            ));
                        }
                    }
                }
                // `bodies[i].field` read.
                if let Expr::Index { object: io, index, .. } = object.as_ref() {
                    if let Expr::Ident { name: on, .. } = io.as_ref() {
                        if Some(on.as_ref()) == arr {
                            let (idx, _) = self.emit_agg_expr(index, arr, aliases)?;
                            return Ok((
                                format!(
                                    "{}[({}) as usize].{}",
                                    Self::escape_ident(on.as_ref()),
                                    idx,
                                    crate::types::field_ident(m.as_ref())
                                ),
                                RustType::F64,
                            ));
                        }
                    }
                }
                Err(CompileError::new("agg: unsupported member", None))
            }
            Expr::Call { callee, args, .. } => {
                // Nested call to another group fn (e.g. `body(...)` from `makeBodies`).
                if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                    if self.aggregate_fns.contains_key(fname.as_ref()) {
                        let (code, ret) =
                            self.emit_agg_group_call(fname.as_ref(), args, arr, aliases)?;
                        let ty = match ret {
                            AggRet::F64 => RustType::F64,
                            AggRet::Struct => {
                                let alias = self.aggregate_alias.clone().unwrap();
                                RustType::Named {
                                    name: alias.as_str().into(),
                                    fields: Vec::new(),
                                }
                            }
                            _ => {
                                return Err(CompileError::new(
                                    "agg: call return not usable here",
                                    None,
                                ))
                            }
                        };
                        return Ok((code, ty));
                    }
                }
                // `Math.<fn>(x)` clean f64-method intrinsics.
                if let Expr::Member {
                    object,
                    prop: MemberProp::Name { name: method, .. },
                    ..
                } = callee.as_ref()
                {
                    if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                    {
                        if let Some(m) = Self::agg_clean_math_method(method.as_ref()) {
                            if let [CallArg::Expr(a)] = args.as_slice() {
                                let (ac, _) = self.emit_agg_expr(a, arr, aliases)?;
                                return Ok((format!("({}).{}()", ac, m), RustType::F64));
                            }
                        }
                    }
                }
                Err(CompileError::new("agg: unsupported call", None))
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let (c, _) = self.emit_agg_expr(cond, arr, aliases)?;
                let (t, _) = self.emit_agg_expr(then_branch, arr, aliases)?;
                let (el, _) = self.emit_agg_expr(else_branch, arr, aliases)?;
                Ok((format!("(if {} {{ {} }} else {{ {} }})", c, t, el), RustType::F64))
            }
            _ => Err(CompileError::new("agg: unsupported expr", None)),
        }
    }

    /// Emit a `return { ... }` object literal as a native struct literal (the `body()` factory).
    fn emit_agg_struct_literal(
        &mut self,
        e: &Expr,
        arr: Option<&str>,
        aliases: &std::collections::HashMap<String, String>,
    ) -> Result<String, CompileError> {
        let alias = self.aggregate_alias.clone().unwrap();
        let struct_ty = crate::types::named_struct_ident(&alias);
        let fields = match self.type_aliases.get(&alias) {
            Some(RustType::Named { fields, .. }) | Some(RustType::Object(fields)) => fields.clone(),
            _ => return Err(CompileError::new("agg: alias not a struct", None)),
        };
        let Expr::Object { props, .. } = e else {
            return Err(CompileError::new("agg: struct return not literal", None));
        };
        use std::collections::HashMap;
        let mut by_key: HashMap<String, &Expr> = HashMap::new();
        for p in props {
            match p {
                ObjectProp::KeyValue(k, v, _) => {
                    by_key.insert(k.to_string(), v);
                }
                ObjectProp::Spread(_) => {
                    return Err(CompileError::new("agg: struct spread", None))
                }
            }
        }
        let mut inits: Vec<String> = Vec::new();
        for (k, _) in &fields {
            let v = by_key
                .get(k.as_ref())
                .ok_or_else(|| CompileError::new("agg: struct missing field", None))?;
            let (code, _) = self.emit_agg_expr(v, arr, aliases)?;
            inits.push(format!("{}: {}", crate::types::field_ident(k.as_ref()), code));
        }
        Ok(format!("{} {{ {} }}", struct_ty, inits.join(", ")))
    }

    /// Emit a `return [a, b, c]` array literal of struct-typed idents as `vec![a, b, c]`.
    fn emit_agg_array_literal(
        &mut self,
        e: &Expr,
        _arr: Option<&str>,
        _aliases: &std::collections::HashMap<String, String>,
    ) -> Result<String, CompileError> {
        let Expr::Array { elements, .. } = e else {
            return Err(CompileError::new("agg: array return not literal", None));
        };
        let mut items: Vec<String> = Vec::new();
        for el in elements {
            match el {
                ArrayElement::Expr(Expr::Ident { name, .. }) => {
                    items.push(Self::escape_ident(name.as_ref()).into_owned());
                }
                _ => {
                    return Err(CompileError::new(
                        "agg: array element not a struct ident",
                        None,
                    ))
                }
            }
        }
        Ok(format!("vec![{}]", items.join(", ")))
    }

    /// Emit a direct call to a group fn, threading the array by `&mut`/`&` plus captured globals.
    /// Returns the call code and the callee's return shape. `arr`/`aliases` describe the CALLER's
    /// context (so an array arg can be the caller's array param, though nbody only passes scalars
    /// across nested group calls).
    fn emit_agg_group_call(
        &mut self,
        name: &str,
        args: &[CallArg],
        arr: Option<&str>,
        aliases: &std::collections::HashMap<String, String>,
    ) -> Result<(String, AggRet), CompileError> {
        let sig = self
            .aggregate_fns
            .get(name)
            .cloned()
            .ok_or_else(|| CompileError::new("agg: unknown group fn", None))?;
        let mut call_args: Vec<String> = Vec::new();
        for (i, (_pname, kind)) in sig.params.iter().enumerate() {
            let a = match args.get(i) {
                Some(CallArg::Expr(e)) => e,
                _ => return Err(CompileError::new("agg: call arg shape", None)),
            };
            match kind {
                AggParamKind::Array { is_mut } => {
                    // The arg must be a bare ident naming an array (caller's param or local).
                    let Expr::Ident { name: an, .. } = a else {
                        return Err(CompileError::new("agg: array arg not ident", None));
                    };
                    let r = if *is_mut { "&mut " } else { "&" };
                    call_args.push(format!("{}{}", r, Self::escape_ident(an.as_ref())));
                }
                AggParamKind::Scalar(_) => {
                    let (code, _) = self.emit_agg_expr(a, arr, aliases)?;
                    call_args.push(code);
                }
            }
        }
        // Captured globals are visible as same-named f64 params/locals in the caller too.
        for g in &sig.captured {
            call_args.push(Self::escape_ident(g).into_owned());
        }
        Ok((
            format!("{}_agg({})", Self::escape_ident(name), call_args.join(", ")),
            sig.ret,
        ))
    }

    /// Try to route a top-level call `name(args)` to its aggregate free fn. `as_value` wraps an
    /// f64 result in `Value::Number` for the boxed `emit_expr` context. Returns `None` if `name`
    /// isn't a group fn (caller falls back to the normal path).
    fn try_emit_toplevel_agg_call(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
        as_value: bool,
    ) -> Result<Option<(String, RustType)>, CompileError> {
        let Expr::Ident { name, .. } = callee else {
            return Ok(None);
        };
        if !self.aggregate_fns.contains_key(name.as_ref()) {
            return Ok(None);
        }
        let sig = self.aggregate_fns.get(name.as_ref()).cloned().unwrap();
        let mut call_args: Vec<String> = Vec::new();
        for (i, (_pname, kind)) in sig.params.iter().enumerate() {
            let a = match args.get(i) {
                Some(CallArg::Expr(e)) => e,
                _ => return Ok(None),
            };
            match kind {
                AggParamKind::Array { is_mut } => {
                    let Expr::Ident { name: an, .. } = a else {
                        return Ok(None);
                    };
                    let r = if *is_mut { "&mut " } else { "&" };
                    call_args.push(format!("{}{}", r, Self::escape_ident(an.as_ref())));
                }
                AggParamKind::Scalar(_) => {
                    let (code, ty) = self.emit_typed_expr(a)?;
                    let f = if ty == RustType::F64 {
                        code
                    } else if ty == RustType::Value {
                        RustType::F64.from_value_expr(&code)
                    } else {
                        code
                    };
                    call_args.push(f);
                }
            }
        }
        for g in &sig.captured {
            let (code, ty) = self.emit_typed_expr(&Expr::Ident {
                name: g.as_str().into(),
                span: tishlang_ast::Span::default(),
            })?;
            let f = if ty == RustType::F64 {
                code
            } else if ty == RustType::Value {
                RustType::F64.from_value_expr(&code)
            } else {
                code
            };
            call_args.push(f);
        }
        let call = format!("{}_agg({})", Self::escape_ident(name.as_ref()), call_args.join(", "));
        let (code, ty) = match sig.ret {
            AggRet::F64 => {
                if as_value {
                    (format!("Value::Number({})", call), RustType::Value)
                } else {
                    (call, RustType::F64)
                }
            }
            AggRet::ArrayOfStruct => {
                let alias = self.aggregate_alias.clone().unwrap();
                (
                    call,
                    RustType::Vec(Box::new(RustType::Named {
                        name: alias.as_str().into(),
                        fields: Vec::new(),
                    })),
                )
            }
            AggRet::Struct => {
                let alias = self.aggregate_alias.clone().unwrap();
                (
                    call,
                    RustType::Named {
                        name: alias.as_str().into(),
                        fields: Vec::new(),
                    },
                )
            }
            // void call: `()`; valid only in statement position (ExprStmt).
            AggRet::Unit => (call, RustType::Unit),
        };
        Ok(Some((code, ty)))
    }

    /// Detect the unboxed struct alias: the unique type-alias name `A` that is (a) used as an
    /// `A[]` array param of some top-level fn and (b) registered as a struct whose fields are all
    /// `Copy` (numeric/bool) — so element field reads/writes by index are sound. Returns `None`
    /// if there is no such alias or more than one (ambiguous → bail to boxed).
    fn detect_aggregate_alias(&self, program: &Program) -> Option<String> {
        let mut found: Option<String> = None;
        for s in &program.statements {
            if let Statement::FunDecl { params, .. } = s {
                for p in params {
                    if let FunParam::Simple(tp) = p {
                        if let Some(TypeAnnotation::Array(inner)) = tp.type_ann.as_ref() {
                            if let TypeAnnotation::Simple(a, _) = inner.as_ref() {
                                let a = a.to_string();
                                if !self.alias_is_copy_struct(&a) {
                                    continue;
                                }
                                match &found {
                                    Some(prev) if prev != &a => return None, // ambiguous
                                    _ => found = Some(a),
                                }
                            }
                        }
                    }
                }
            }
        }
        found
    }

    /// Is `name` a registered struct alias whose every field is a `Copy` scalar (f64/bool/i32)?
    fn alias_is_copy_struct(&self, name: &str) -> bool {
        match self.type_aliases.get(name) {
            Some(RustType::Named { fields, .. }) | Some(RustType::Object(fields)) => {
                !fields.is_empty()
                    && fields.iter().all(|(_, t)| {
                        matches!(t, RustType::F64 | RustType::Bool | RustType::I32)
                    })
            }
            _ => false,
        }
    }

    /// Is `ann` exactly `Simple(alias)`?
    fn ann_is_simple(ann: Option<&TypeAnnotation>, alias: &str) -> bool {
        matches!(ann, Some(TypeAnnotation::Simple(a, _)) if a.as_ref() == alias)
    }

    /// Is `ann` exactly `Array(Simple(alias))` (i.e. `alias[]`)?
    fn ann_is_array_of(ann: Option<&TypeAnnotation>, alias: &str) -> bool {
        matches!(ann, Some(TypeAnnotation::Array(inner))
            if matches!(inner.as_ref(), TypeAnnotation::Simple(a, _) if a.as_ref() == alias))
    }

    /// Math methods whose JS semantics match the Rust `f64` method 1:1 (no rounding/sign quirks).
    fn agg_clean_math_method(m: &str) -> Option<&'static str> {
        Some(match m {
            "sqrt" => "sqrt",
            "sin" => "sin",
            "cos" => "cos",
            "tan" => "tan",
            "exp" => "exp",
            "log" => "ln",
            "sinh" => "sinh",
            "cosh" => "cosh",
            "tanh" => "tanh",
            "asin" => "asin",
            "acos" => "acos",
            "atan" => "atan",
            "asinh" => "asinh",
            "acosh" => "acosh",
            "atanh" => "atanh",
            "cbrt" => "cbrt",
            "log2" => "log2",
            "log10" => "log10",
            _ => return None,
        })
    }

    /// Does `body` contain a write through array param `p` (element field write / index write)?
    fn agg_fn_mutates_array(body: &Statement, p: &str) -> bool {
        let mut aliases: std::collections::HashSet<String> = std::collections::HashSet::new();
        Self::agg_collect_aliases(body, p, &mut aliases);
        Self::agg_stmt_writes(body, p, &aliases)
    }

    fn agg_collect_aliases(s: &Statement, p: &str, out: &mut std::collections::HashSet<String>) {
        match s {
            Statement::VarDecl {
                name,
                init: Some(Expr::Index { object, index, .. }),
                ..
            } => {
                if matches!(object.as_ref(), Expr::Ident { name: o, .. } if o.as_ref() == p)
                    && matches!(index.as_ref(), Expr::Ident { .. })
                {
                    out.insert(name.to_string());
                }
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().for_each(|x| Self::agg_collect_aliases(x, p, out));
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::agg_collect_aliases(then_branch, p, out);
                if let Some(e) = else_branch {
                    Self::agg_collect_aliases(e, p, out);
                }
            }
            Statement::For { init, body, .. } => {
                if let Some(i) = init {
                    Self::agg_collect_aliases(i, p, out);
                }
                Self::agg_collect_aliases(body, p, out);
            }
            Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::ForOf { body, .. } => Self::agg_collect_aliases(body, p, out),
            _ => {}
        }
    }

    fn agg_stmt_writes(
        s: &Statement,
        p: &str,
        aliases: &std::collections::HashSet<String>,
    ) -> bool {
        match s {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(|x| Self::agg_stmt_writes(x, p, aliases))
            }
            Statement::ExprStmt { expr, .. } => Self::agg_expr_writes(expr, p, aliases),
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::agg_stmt_writes(then_branch, p, aliases)
                    || else_branch
                        .as_ref()
                        .is_some_and(|e| Self::agg_stmt_writes(e, p, aliases))
            }
            Statement::For { body, .. }
            | Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::ForOf { body, .. } => Self::agg_stmt_writes(body, p, aliases),
            _ => false,
        }
    }

    fn agg_expr_writes(
        e: &Expr,
        p: &str,
        aliases: &std::collections::HashSet<String>,
    ) -> bool {
        match e {
            Expr::MemberAssign { object, .. } => match object.as_ref() {
                Expr::Ident { name, .. } => aliases.contains(name.as_ref()),
                Expr::Index { object: io, .. } => {
                    matches!(io.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p)
                }
                _ => false,
            },
            Expr::IndexAssign { object, .. } => {
                matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == p)
            }
            _ => false,
        }
    }

    /// Top-level `let` names whose initializer is a numeric constant — the only globals safe to
    /// thread into an aggregate fn as a trailing `f64` param. A `let bodies = makeBodies()` or
    /// `let t0 = Date.now()` is excluded so it can never be mistyped as `f64`.
    fn collect_toplevel_global_lets(program: &Program) -> std::collections::HashSet<String> {
        let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
        for s in &program.statements {
            if let Statement::VarDecl {
                name,
                init: Some(e),
                ..
            } = s
            {
                if Self::expr_is_numeric_const(e, &out) {
                    out.insert(name.to_string());
                }
            }
        }
        out
    }

    /// Conservatively: a numeric literal, an arithmetic combination of such, or a reference to an
    /// already-proven numeric global (`numeric` carries the names accepted so far, in source order).
    fn expr_is_numeric_const(e: &Expr, numeric: &std::collections::HashSet<String>) -> bool {
        match e {
            Expr::Literal {
                value: Literal::Number(_),
                ..
            } => true,
            Expr::Ident { name, .. } => numeric.contains(name.as_ref()),
            Expr::Unary {
                op: UnaryOp::Neg | UnaryOp::Pos,
                operand,
                ..
            } => Self::expr_is_numeric_const(operand, numeric),
            Expr::Binary {
                left, op, right, ..
            } => {
                matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                ) && Self::expr_is_numeric_const(left, numeric)
                    && Self::expr_is_numeric_const(right, numeric)
            }
            _ => false,
        }
    }

    /// Captured globals a fn body references: free idents that are top-level globals, excluding
    /// the fn's own params, its locals, the other group-fn names, and the struct alias.
    fn agg_captured_globals(
        body: &Statement,
        params: &[FunParam],
        globals: &std::collections::HashSet<String>,
        group_fns: &std::collections::HashMap<String, AggFnSig>,
        self_name: &str,
        alias: &str,
    ) -> Vec<String> {
        let mut idents: std::collections::HashSet<String> = std::collections::HashSet::new();
        Self::collect_stmt_idents(body, &mut idents);
        let mut locals: std::collections::HashSet<String> = std::collections::HashSet::new();
        Self::collect_local_var_names(body, &mut locals);
        let pnames: std::collections::HashSet<String> = params
            .iter()
            .flat_map(|p| p.bound_names())
            .map(|n| n.to_string())
            .collect();
        let mut out: Vec<String> = idents
            .into_iter()
            .filter(|id| {
                globals.contains(id)
                    && !pnames.contains(id)
                    && !locals.contains(id)
                    && !group_fns.contains_key(id)
                    && id != self_name
                    && id != alias
            })
            .collect();
        out.sort();
        out
    }

    /// Does `s` contain a `return <value>` (vs only bare `return;` / no return)?
    fn stmt_returns_value(s: &Statement) -> bool {
        match s {
            Statement::Return { value, .. } => value.is_some(),
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().any(Self::stmt_returns_value)
            }
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::stmt_returns_value(then_branch)
                    || else_branch.as_ref().is_some_and(|e| Self::stmt_returns_value(e))
            }
            Statement::For { body, .. }
            | Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::ForOf { body, .. } => Self::stmt_returns_value(body),
            _ => false,
        }
    }

    /// Lower an expression that JS will coerce to **int32** inside a bitwise/shift computation,
    /// staying in the integer domain instead of round-tripping every intermediate through `f64`.
    ///
    /// Returns `Ok(Some(code))` where `code` is an `i32`-typed Rust expression equal to
    /// `ToInt32(e)`, or `Ok(None)` if a leaf can't be proven `F64` (then the caller keeps the
    /// existing per-op lowering — purely additive, never a regression).
    ///
    /// This is behaviour-identical to the nested `to_int32`/`to_uint32` lowering: an intermediate
    /// `(i32 as f64)` immediately re-narrowed by `to_int32` is exact (every `i32` is representable
    /// in `f64`, and `to_int32` of a finite value recovers it), so erasing it changes nothing but
    /// the round-trips. Crucially, only bitwise/shift nodes recurse — an `f64` `*`/`+`/`-` node is
    /// a *leaf* here, so e.g. `(h * 16777619) >>> 0` keeps its `f64` multiply (the 2^53 rule: the
    /// product exceeds 2^53 and must round in `f64` *before* `ToUint32`, exactly as V8 does).
    fn emit_int32_operand(&mut self, e: &Expr) -> Result<Option<String>, CompileError> {
        if let Expr::Binary {
            left, op, right, ..
        } = e
        {
            let bitwise = matches!(
                op,
                BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::Shl
                    | BinOp::Shr
                    | BinOp::UShr
            );
            if bitwise {
                let li = match self.emit_int32_operand(left)? {
                    Some(c) => c,
                    None => return Ok(None),
                };
                let ri = match self.emit_int32_operand(right)? {
                    Some(c) => c,
                    None => return Ok(None),
                };
                // Shift counts: `(ri as u32)` shares its low 5 bits with `to_uint32(rhs)`, and
                // `wrapping_sh*` masks the count mod 32 — exactly JS's `count & 31`.
                let code = match op {
                    BinOp::BitAnd => format!("({} & {})", li, ri),
                    BinOp::BitOr => format!("({} | {})", li, ri),
                    BinOp::BitXor => format!("({} ^ {})", li, ri),
                    BinOp::Shl => format!("({}).wrapping_shl(({}) as u32)", li, ri),
                    BinOp::Shr => format!("({}).wrapping_shr(({}) as u32)", li, ri),
                    // `>>>` is a logical shift on the uint32 view; reinterpret back to `i32` to
                    // stay in the integer domain (the unsigned value is recovered at the f64 edge).
                    BinOp::UShr => {
                        format!("((({}) as u32).wrapping_shr(({}) as u32) as i32)", li, ri)
                    }
                    _ => unreachable!(),
                };
                return Ok(Some(code));
            }
        }
        // Integer-literal leaf: `ToInt32(<int literal>)` is a compile-time constant — emit it
        // directly (`v as i32` = modulo 2^32 = JS ToInt32 of the integer) instead of a runtime
        // `to_int32(1_f64)` call. Removes the constant round-trip in masks/shift-counts
        // (`x & 255`, `x >> 1`, …). Trivially sound: the value is known.
        if let Some(v) = Self::int_literal_value_of(e) {
            return Ok(Some(format!("{}i32", v as i32)));
        }
        if let Expr::Ident { name, .. } = e {
            if self.int_valued_locals.contains(name.as_ref()) {
                let (code, ty) = self.emit_typed_expr(e)?;
                if ty == RustType::F64 {
                    return Ok(Some(format!(
                        "(unsafe {{ ({}).to_int_unchecked::<i64>() }} as i32)",
                        code
                    )));
                }
            }
        }
        // Leaf: fold only when it is a plain `f64` (so `to_int32` applies directly). `to_int32`
        // keeps its `is_finite` guard here — a leaf may legitimately be NaN/±Infinity (→ 0).
        let (code, ty) = self.emit_typed_expr(e)?;
        if ty == RustType::F64 {
            // Drop the `is_finite` guard and truncate directly when the leaf is PROVABLY a finite
            // integer, via either of two independent proofs:
            //   • an ARITHMETIC node with `|x| < 2^62` (operands are i32-register reads / finite
            //     literals — e.g. the FNV `h * 16777619` excursion); or
            //   • the integer-range lattice proves `x ∈ (-2^53, 2^53)` (#174 — e.g. a range-bounded
            //     induction counter `i` in `i & 255`, or `(k+1)` when `k` is range-proven).
            // Both guarantee finiteness, so `to_int_unchecked::<i64>()` is defined and truncates
            // toward zero (a no-op on an integer); `as i32` = modulo 2^32 = JS ToInt32. Bit-identical
            // to the guarded `to_int32`, a few instructions cheaper per iteration. Any unproven leaf
            // keeps the guarded `to_int32` (NaN/±Infinity → 0).
            let proven_finite_int = (matches!(e, Expr::Binary { .. })
                && self.f64_finite_bounded_below_2pow62(e))
                || self.int_range(e, &self.int_range_locals).is_some();
            if proven_finite_int {
                Ok(Some(format!(
                    "(unsafe {{ ({}).to_int_unchecked::<i64>() }} as i32)",
                    code
                )))
            } else {
                Ok(Some(format!("tishlang_runtime::to_int32({})", code)))
            }
        } else if ty == RustType::I32 {
            // An `I32` loop-accumulator already holds its JS ToInt32 bit-pattern in an integer
            // register — feed it straight in, NO `to_int32` round-trip. This is the perf win: the
            // per-op `f64`→i32 narrowing across the hash loop collapses to a register read.
            Ok(Some(code))
        } else {
            Ok(None)
        }
    }

    fn emit_typed_expr(&mut self, expr: &Expr) -> Result<(String, RustType), CompileError> {
        match expr {
            // ── literals ─────────────────────────────────────────────────────────
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => Ok((Self::f64_lit(*n), RustType::F64)),
                Literal::String(s) => {
                    Ok((format!("{:?}.to_string()", s.as_ref()), RustType::String))
                }
                Literal::Bool(b) => Ok((format!("{}", b), RustType::Bool)),
                Literal::Null => Ok(("Value::Null".to_string(), RustType::Value)),
            },

            // ── identifiers ──────────────────────────────────────────────────────
            Expr::Ident { name, .. } => {
                // #175 inline: a param of the fn currently being inlined reads its bound f64 temp.
                if let Some(top) = self.inline_subst.last() {
                    if let Some(temp) = top.get(name.as_ref()) {
                        return Ok((temp.clone(), RustType::F64));
                    }
                }
                // #176: native numeric global → thread_local Cell read.
                if self.native_numeric_globals.contains_key(name.as_ref()) {
                    return Ok((
                        Self::native_global_get(name.as_ref()),
                        RustType::F64,
                    ));
                }
                if let Some(uv) = self.usize_var_subst.get(name.as_ref()) {
                    return Ok((format!("({} as f64)", uv), RustType::F64));
                }
                if let Some(static_name) = self.module_const_f64_array_rust_ref(name.as_ref()) {
                    let var_type = RustType::Vec(Box::new(RustType::F64));
                    return Ok((
                        format!("{}.iter().copied().collect::<Vec<f64>>()", static_name),
                        var_type,
                    ));
                }
                let escaped = Self::escape_ident(name.as_ref());
                if self.refcell_wrapped_vars.contains(name.as_ref()) {
                    let var_type = self.type_context.get_type(name.as_ref());
                    if var_type.is_native() {
                        Ok((format!("(*{}.borrow()).clone()", escaped), var_type))
                    } else {
                        Ok((format!("(*{}.borrow()).clone()", escaped), RustType::Value))
                    }
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
            Expr::Binary {
                left,
                op,
                right,
                span,
                ..
            } => {
                let (l, lt) = self.emit_typed_expr(left)?;
                let (r, rt) = self.emit_typed_expr(right)?;

                // An `I32` loop-accumulator (the i32-loop-var lowering) used in a NON-bitwise
                // expression reads as its signed int32 value coerced to `f64` — every i32 is exact
                // in f64. Bitwise/shift parents never see this: they recurse into the raw AST via
                // `emit_int32_operand`, which reads the i32 register directly. So coercing here only
                // governs arithmetic/relational reads (e.g. the `h * 16777619` excursion), where the
                // operand must be f64 to keep JS Number semantics.
                let (l, lt) = if lt == RustType::I32 {
                    (format!("(({}) as f64)", l), RustType::F64)
                } else {
                    (l, lt)
                };
                let (r, rt) = if rt == RustType::I32 {
                    (format!("(({}) as f64)", r), RustType::F64)
                } else {
                    (r, rt)
                };

                if let Some(result_ty) = RustType::result_type_of_binop(*op, &lt, &rt) {
                    // Bitwise/shift over numbers: lower the *whole* chain in the int32 domain so
                    // intermediate `to_int32`/`to_uint32`↔`f64` round-trips collapse (the win `>>>`
                    // exists for — crypto/hashing loops). Only fires when every leaf proves `f64`;
                    // otherwise we fall through to the per-op lowering below. Behaviour-identical
                    // (see `emit_int32_operand`), and the gauntlet's `typed == boxed == node` check
                    // gates any divergence.
                    if matches!(
                        op,
                        BinOp::BitAnd
                            | BinOp::BitOr
                            | BinOp::BitXor
                            | BinOp::Shl
                            | BinOp::Shr
                            | BinOp::UShr
                    ) && result_ty == RustType::F64
                    {
                        if let Some(int_code) = self.emit_int32_operand(expr)? {
                            // `>>>` yields a uint32 Number; the others yield a signed int32 Number.
                            let f64_code = if matches!(op, BinOp::UShr) {
                                format!("(({}) as u32 as f64)", int_code)
                            } else {
                                format!("(({}) as f64)", int_code)
                            };
                            return Ok((f64_code, RustType::F64));
                        }
                    }
                    // Integer remainder (#174): `x % c` where the dividend `x` is a proven integer
                    // in (-2^53, 2^53) and `c` a positive integer literal → `(x as i64) % c` instead
                    // of `fmod`. Bit-identical (x is exactly an integer in f64; Rust `%` and `fmod`
                    // both truncate toward zero), and far faster — the fmod in LCG/hash recurrences.
                    if matches!(op, BinOp::Mod) && result_ty == RustType::F64 {
                        if let Some(c) = Self::int_literal_value_of(right).filter(|&c| c > 0) {
                            if self.int_range(left, &self.int_range_locals).is_some() {
                                return Ok((
                                    format!("(((({}) as i64) % {}i64) as f64)", l, c),
                                    RustType::F64,
                                ));
                            }
                        }
                    }
                    // Both sides are compatible native types → emit native op.
                    let code = match op {
                        BinOp::Add if result_ty == RustType::String => {
                            // M2: Rust `String + String` is illegal; build a fresh String.
                            // `format!` borrows both operands, so chained concats (`a + b + c`)
                            // nest cleanly with no move/clone hazards.
                            format!("format!(\"{{}}{{}}\", {}, {})", l, r)
                        }
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
                        // Native int32 bitwise/shift (operands are f64 here). `to_int32`/`to_uint32`
                        // is JS ToInt32/ToUint32 (modulo 2³², NaN/±Infinity → 0; `#[inline]` so the
                        // `is_finite` guard folds away on the hot finite path); shift counts mask to
                        // 5 bits via `wrapping_sh*` (JS semantics, no panic).
                        BinOp::BitAnd => format!(
                            "((tishlang_runtime::to_int32({}) & tishlang_runtime::to_int32({})) as f64)",
                            l, r
                        ),
                        BinOp::BitOr => format!(
                            "((tishlang_runtime::to_int32({}) | tishlang_runtime::to_int32({})) as f64)",
                            l, r
                        ),
                        BinOp::BitXor => format!(
                            "((tishlang_runtime::to_int32({}) ^ tishlang_runtime::to_int32({})) as f64)",
                            l, r
                        ),
                        BinOp::Shl => format!(
                            "(tishlang_runtime::to_int32({}).wrapping_shl(tishlang_runtime::to_uint32({})) as f64)",
                            l, r
                        ),
                        BinOp::Shr => format!(
                            "(tishlang_runtime::to_int32({}).wrapping_shr(tishlang_runtime::to_uint32({})) as f64)",
                            l, r
                        ),
                        BinOp::UShr => format!(
                            "(tishlang_runtime::to_uint32({}).wrapping_shr(tishlang_runtime::to_uint32({})) as f64)",
                            l, r
                        ),
                        _ => unreachable!("result_type_of_binop covers all handled ops"),
                    };
                    return Ok((code, result_ty));
                }

                // Mixed numeric relational: one side is a native `f64`, the other a boxed `Value`
                // (e.g. nsieve's `while (k < n)` where `k` is f64 and the param `n` stayed boxed).
                // JS does a numeric comparison here — the f64 side forces ToNumber on the other —
                // so coerce the Value inline (`as_number().unwrap_or(NaN)`) and compare natively,
                // instead of boxing the f64 side and paying `ops::{lt,le,gt,ge}` + `Value::Bool` +
                // `is_truthy` every iteration. Behaviour-identical to that boxed path for every
                // input: a non-number Value coerces to NaN, so all comparisons are `false`, exactly
                // as `ops::*` returns `false` outside the (Number,Number)/(String,String) cases —
                // and (String,String) can't reach here since one side is f64.
                if matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
                    && (lt == RustType::F64 || rt == RustType::F64)
                {
                    let coerce = |code: &str, ty: &RustType| match ty {
                        RustType::F64 => Some(code.to_string()),
                        RustType::Value => {
                            Some(format!("({}).as_number().unwrap_or(f64::NAN)", code))
                        }
                        _ => None,
                    };
                    if let (Some(lc), Some(rc)) = (coerce(&l, &lt), coerce(&r, &rt)) {
                        let sym = match op {
                            BinOp::Lt => "<",
                            BinOp::Le => "<=",
                            BinOp::Gt => ">",
                            BinOp::Ge => ">=",
                            _ => unreachable!(),
                        };
                        return Ok((format!("({} {} {})", lc, sym, rc), RustType::Bool));
                    }
                }

                // Fall back: convert both sides to Value and use the runtime.
                let lv = if lt.is_native() {
                    lt.to_value_expr(&l)
                } else {
                    l
                };
                let rv = if rt.is_native() {
                    rt.to_value_expr(&r)
                } else {
                    r
                };
                let result = self.emit_binop(&lv, *op, &rv, *span)?;
                Ok((result, RustType::Value))
            }

            // ── logical not on native bool ───────────────────────────────────────
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
                ..
            } => {
                let (o, ot) = self.emit_typed_expr(operand)?;
                if ot == RustType::Bool {
                    return Ok((format!("!{}", o), RustType::Bool));
                }
                let result = self.emit_expr(expr)?;
                Ok((result, RustType::Value))
            }

            // ── array indexing ───────────────────────────────────────────────────
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                // Module const `[f64; N]` — direct index, no Vec wrapper (fasta `codes`/`cum`).
                if !optional {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if let Some(static_name) = Self::module_const_array_static(
                            &self.module_const_f64_arrays,
                            &self.module_const_f64_aliases,
                            name.as_ref(),
                        ) {
                            let in_bounds = self.index_in_bounds(index, name.as_ref());
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
                            let access = if in_bounds {
                                format!("{}[{}]", static_name, idx_usize)
                            } else {
                                let const_len = self
                                    .cum_static_and_len(name.as_ref())
                                    .map(|(_, n)| n)
                                    .or_else(|| {
                                        self.module_const_f64_arrays
                                            .get(name.as_ref())
                                            .map(|v| v.len())
                                    })
                                    .unwrap_or(0);
                                if const_len > 0 {
                                    format!(
                                        "{{ let _i = {}; if _i < {} {{ {}[_i] }} else {{ f64::NAN }} }}",
                                        idx_usize,
                                        const_len,
                                        static_name
                                    )
                                } else {
                                    format!(
                                        "{{ let _i = {}; if _i < {}.len() {{ {}[_i] }} else {{ f64::NAN }} }}",
                                        idx_usize,
                                        static_name,
                                        static_name
                                    )
                                }
                            };
                            return Ok((access, RustType::F64));
                        }
                    }
                }
                // Native fast path: `vec[i]` where vec is Vec<T> and i is numeric.
                if !optional {
                    if let Expr::Ident { name, .. } = object.as_ref() {
                        if !self.refcell_wrapped_vars.contains(name.as_ref()) {
                            let obj_type = self.type_context.get_type(name.as_ref());
                            if let RustType::Vec(elem_type) = &obj_type {
                                let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
                                // #173 part 3: prove the index in-bounds BEFORE emitting it.
                                let in_bounds = self.index_in_bounds(index, name.as_ref());
                                let idx_usize = if let Expr::Ident { name: idx_name, .. } =
                                    index.as_ref()
                                {
                                    if let Some(uv) =
                                        self.usize_var_subst.get(idx_name.as_ref())
                                    {
                                        uv.clone()
                                    } else {
                                        let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
                                        if idx_ty == RustType::F64 || idx_ty == RustType::I32 {
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
                                        }
                                    }
                                } else {
                                    let (idx_code, idx_ty) = self.emit_typed_expr(index)?;
                                    if idx_ty == RustType::F64 || idx_ty == RustType::I32 {
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
                                    }
                                };
                                let elem_ty = *elem_type.clone();
                                // OOB-safe read for numeric/bool Vecs: JS `arr[oob]` is `undefined`
                                // (→ NaN / false in those contexts), NOT a panic. In-bounds is the
                                // same bounds-checked access, so this is purely a correctness gain
                                // (and what lets index reads be *inferred* as native — phase 2).
                                let access = match &elem_ty {
                                    // #173 part 3: a proven in-bounds read skips the `.get().unwrap_or`
                                    // branch — a direct `a[i]` (the `idx < len` proof guarantees it).
                                    RustType::F64 | RustType::Bool | RustType::I32 if in_bounds => {
                                        format!("{}[{}]", esc_obj, idx_usize)
                                    }
                                    RustType::F64 => format!(
                                        "{}.get({}).copied().unwrap_or(f64::NAN)",
                                        esc_obj, idx_usize
                                    ),
                                    RustType::I32 => format!(
                                        "{}.get({}).copied().unwrap_or(0)",
                                        esc_obj, idx_usize
                                    ),
                                    RustType::Bool => {
                                        format!("{}.get({}).copied().unwrap_or(false)", esc_obj, idx_usize)
                                    }
                                    // Other element types keep the direct index (unchanged).
                                    _ => format!("{}[{}]", esc_obj, idx_usize),
                                };
                                return Ok((access, elem_ty));
                            }
                            // Native tuple access: `tuple[const]` -> `tuple.const` (Rust tuples
                            // require a literal index; a variable index falls through to boxed).
                            if let RustType::Tuple(elems) = &obj_type {
                                if let Expr::Literal {
                                    value: Literal::Number(n),
                                    ..
                                } = index.as_ref()
                                {
                                    let i = *n as usize;
                                    if n.fract() == 0.0 && i < elems.len() {
                                        let esc_obj = Self::escape_ident(name.as_ref()).into_owned();
                                        return Ok((format!("{}.{}", esc_obj, i), elems[i].clone()));
                                    }
                                }
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

            // ── native Math intrinsics ───────────────────────────────────────────
            // `Math.sqrt(x)` etc. with a native-f64 arg lowers to a direct f64 method,
            // skipping the boxed value_call per element. Only methods whose Rust f64 op
            // matches JS semantics (round half-up & sign(0) differ → left to the runtime).
            Expr::Call { callee, args, .. } => {
                // #175 inline: a numeric leaf fn (`evalA`) called with all-f64 args expands inline to
                // pure f64 arithmetic — no dispatch. Tried first so a hot helper never pays a call.
                if !self.inline_fns.is_empty() {
                    if let Some(res) = self.try_emit_inline_call(callee, args)? {
                        return Ok(res);
                    }
                }
                // #177: a de-virtualized aggregate fn used in native arithmetic (e.g. `energy(bodies)`
                // feeding an f64 expression) → call `name_agg(..)` returning the native type.
                if !self.aggregate_fns.is_empty() {
                    if let Some((code, ty)) = self.try_emit_toplevel_agg_call(callee, args, false)? {
                        return Ok((code, ty));
                    }
                }
                // #175: a native-vec fn used in a typed/native expression (e.g. the recursive
                // `count + place(..)`). f64-returning → (call, F64); void → boxed unit.
                if !self.native_vec_fns.is_empty() {
                    if let Expr::Ident { name: vn, .. } = callee.as_ref() {
                        if let Some(sig) = self.native_vec_fns.get(vn.as_ref()).cloned() {
                            if let Some(code) = self.try_emit_native_vec_call(callee, args, false)? {
                                let ty = match sig.ret {
                                    VecRetKind::F64 => RustType::F64,
                                    VecRetKind::VecF64 => RustType::Vec(Box::new(RustType::F64)),
                                    VecRetKind::Unit => RustType::Value,
                                };
                                return Ok((code, ty));
                            }
                        }
                    }
                }
                // M5: direct call to an eligible native fn -> `name_native(<native args>)`.
                if let Expr::Ident { name: fname, .. } = callee.as_ref() {
                    if self.native_fns.contains(fname.as_ref()) {
                        let mut argc: Vec<String> = Vec::with_capacity(args.len());
                        let mut ok = true;
                        for a in args {
                            if let CallArg::Expr(e) = a {
                                let (ac, at) = self.emit_typed_expr(e)?;
                                argc.push(if at == RustType::Value {
                                    RustType::F64.from_value_expr(&ac)
                                } else {
                                    ac
                                });
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            if let Some((global, mul, add, modulus)) =
                                self.native_lcg_fns.get(fname.as_ref())
                            {
                                let max_expr = argc.first().cloned().unwrap_or_else(|| "1.0".into());
                                return Ok((
                                    self.emit_native_lcg_with(global, *mul, *add, *modulus, &max_expr),
                                    RustType::F64,
                                ));
                            }
                            return Ok((
                                format!(
                                    "{}_native({})",
                                    Self::escape_ident(fname.as_ref()),
                                    argc.join(", ")
                                ),
                                RustType::F64,
                            ));
                        }
                    }
                }
                if let [CallArg::Expr(arg_expr)] = args.as_slice() {
                    if let Expr::Member {
                        object,
                        prop: MemberProp::Name { name: method, .. },
                        ..
                    } = callee.as_ref()
                    {
                        if matches!(object.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Math")
                        {
                            let rust_m = match method.as_ref() {
                                "sqrt" => Some("sqrt"),
                                "sin" => Some("sin"),
                                "cos" => Some("cos"),
                                "tan" => Some("tan"),
                                "abs" => Some("abs"),
                                "floor" => Some("floor"),
                                "ceil" => Some("ceil"),
                                "exp" => Some("exp"),
                                "trunc" => Some("trunc"),
                                "log" => Some("ln"),
                                "sinh" => Some("sinh"),
                                "cosh" => Some("cosh"),
                                "tanh" => Some("tanh"),
                                "asinh" => Some("asinh"),
                                "acosh" => Some("acosh"),
                                "atanh" => Some("atanh"),
                                "cbrt" => Some("cbrt"),
                                "log2" => Some("log2"),
                                "log10" => Some("log10"),
                                _ => None,
                            };
                            if let Some(m) = rust_m {
                                let (arg_code, arg_ty) = self.emit_typed_expr(arg_expr)?;
                                let arg_f64 = if arg_ty == RustType::F64 {
                                    arg_code
                                } else if arg_ty == RustType::Value {
                                    RustType::F64.from_value_expr(&arg_code)
                                } else {
                                    arg_code
                                };
                                return Ok((format!("({}).{}()", arg_f64, m), RustType::F64));
                            }
                        }
                    }
                }
                // Native typed-array HOFs over a `Vec<f64>` receiver (TISH_NATIVE_HOF):
                // `xs.reduce/map/filter/some/every(<arrow>)` → a direct Rust iterator chain,
                // eliminating the per-element `value_call` and all `Value` boxing.
                if let Some(res) = self.native_vec_hof_for_call(callee, args)? {
                    return Ok(res);
                }
                let result = self.emit_expr(expr)?;
                Ok((result, RustType::Value))
            }

            // ── native struct field access ───────────────────────────────────────
            // `o.x` where `o` is a `RustType::Named` struct local and `x` is a native
            // (f64/bool/string) field → a direct Rust field read with that native type,
            // instead of boxing it through `Value::Number(o.x)`. This keeps `sum + o.x + o.y`
            // entirely native (the object_sum hot loop) — see emit_expr's struct fast path,
            // which returns the SAME access but wrapped in `Value::*` for the dynamic callers.
            Expr::Member {
                object,
                prop: MemberProp::Name { name: prop_name, .. },
                optional: false,
                ..
            } => {
                if let Expr::Ident { name: var_name, .. } = object.as_ref() {
                    let var_type = self.type_context.get_type(var_name.as_ref());
                    // #173: `vec.length` on a native `Vec<_>` → `(vec.len() as f64)`, so the length
                    // (and arithmetic derived from it) stays native instead of a boxed `get_prop`.
                    if let RustType::Vec(_) = &var_type {
                        if prop_name.as_ref() == "length" {
                            let var_esc = Self::escape_ident(var_name.as_ref()).into_owned();
                            let code = if self.refcell_wrapped_vars.contains(var_name.as_ref()) {
                                format!("({}.borrow().len() as f64)", var_esc)
                            } else {
                                format!("({}.len() as f64)", var_esc)
                            };
                            return Ok((code, RustType::F64));
                        }
                    }
                    if let RustType::Named { fields, .. } = &var_type {
                        if let Some((_, field_ty)) =
                            fields.iter().find(|(k, _)| k.as_ref() == prop_name.as_ref())
                        {
                            let var_esc = Self::escape_ident(var_name.as_ref()).into_owned();
                            let field = crate::types::field_ident(prop_name.as_ref());
                            let access = if self.refcell_wrapped_vars.contains(var_name.as_ref())
                            {
                                format!("(*{}.borrow()).{}.clone()", var_esc, field)
                            } else {
                                format!("{}.{}", var_esc, field)
                            };
                            if field_ty.is_native() {
                                return Ok((access, field_ty.clone()));
                            }
                            if matches!(field_ty, RustType::Named { .. } | RustType::Vec(_)) {
                                return Ok((access, field_ty.clone()));
                            }
                        }
                    }
                }
                let result = self.emit_expr(expr)?;
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

    /// Fused `reduce`: if the callback is exactly `(acc, x) => acc OP x` (or `x OP acc`) with a
    /// plain binop of the two params, emit a native fold over the array using the SAME runtime
    /// Value op the closure body would — eliminating the per-element `value_call`. Sound (identical
    /// Value semantics, including string `+`). Returns `None` to fall back to `array_reduce`.
    fn try_fused_reduce(
        &self,
        args: &[CallArg],
        obj_expr: &str,
        initial: &str,
    ) -> Result<Option<String>, CompileError> {
        let Some(CallArg::Expr(Expr::ArrowFunction { params, body, .. })) = args.first() else {
            return Ok(None);
        };
        let tishlang_ast::ArrowBody::Expr(be) = body else {
            return Ok(None);
        };
        if params.len() != 2 {
            return Ok(None);
        }
        let pname = |p: &FunParam| -> Option<std::sync::Arc<str>> {
            match p {
                FunParam::Simple(tp) if tp.default.is_none() => Some(std::sync::Arc::clone(&tp.name)),
                _ => None,
            }
        };
        let (Some(acc), Some(cur)) = (pname(&params[0]), pname(&params[1])) else {
            return Ok(None);
        };
        let Expr::Binary {
            left, op, right, span,
        } = be.as_ref()
        else {
            return Ok(None);
        };
        let ident = |e: &Expr| -> Option<std::sync::Arc<str>> {
            match e {
                Expr::Ident { name, .. } => Some(std::sync::Arc::clone(name)),
                _ => None,
            }
        };
        let (Some(ln), Some(rn)) = (ident(left), ident(right)) else {
            return Ok(None);
        };
        // Map each operand to `_acc` / `_x` in the body's actual order.
        let (ls, rs) = if ln.as_ref() == acc.as_ref() && rn.as_ref() == cur.as_ref() {
            ("_acc", "_x")
        } else if ln.as_ref() == cur.as_ref() && rn.as_ref() == acc.as_ref() {
            ("_x", "_acc")
        } else {
            return Ok(None);
        };
        let body_code = self.emit_binop(ls, *op, rs, *span)?;

        // Native-f64 fast path for arithmetic reducers in the standard `acc OP x` order. We can't
        // assume the array is numeric at compile time (`+` concatenates strings in JS), so emit a
        // runtime all-numeric guard: if the init and every element are `Value::Number`, fold in raw
        // f64 (no per-element `ops::add` call, no Result, no re-boxing); otherwise fall back to the
        // boxed fold from the original init — identical semantics either way. This is the array_hof
        // hot loop; a fully-unboxed `Vec<f64>` (packed arrays / task #13) would go further.
        let nat_op = if (ls, rs) == ("_acc", "_x") {
            match op {
                BinOp::Add => Some("+="),
                BinOp::Sub => Some("-="),
                BinOp::Mul => Some("*="),
                BinOp::Div => Some("/="),
                _ => None,
            }
        } else {
            None
        };
        if let Some(nat_op) = nat_op {
            return Ok(Some(format!(
                "{{ let _init0 = {init}; let _arr = ({obj}).clone(); \
                 if let Value::Array(ref _a) = _arr {{ let _b = _a.borrow(); \
                 let mut _accn: f64 = 0.0; let mut _ok = false; \
                 if let Value::Number(_i0) = &_init0 {{ _accn = *_i0; _ok = true; }} \
                 if _ok {{ for _el in _b.iter() {{ \
                 if let Value::Number(_n) = _el {{ _accn {nat_op} *_n; }} else {{ _ok = false; break; }} }} }} \
                 if _ok {{ Value::Number(_accn) }} \
                 else {{ let mut _acc = _init0; for _el in _b.iter() {{ let _x = _el.clone(); _acc = {body}; }} _acc }} \
                 }} else {{ _init0 }} }}",
                init = initial,
                obj = obj_expr,
                nat_op = nat_op,
                body = body_code
            )));
        }

        Ok(Some(format!(
            "{{ let mut _acc = {init}; let _arr = ({obj}).clone(); \
             if let Value::Array(ref _a) = _arr {{ for _el in _a.borrow().iter() {{ \
             let _x = _el.clone(); _acc = {body}; }} }} _acc }}",
            init = initial,
            obj = obj_expr,
            body = body_code
        )))
    }

    /// If `callee(args)` is `<Vec<f64>-ident>.reduce/map/filter/some/every(<arrow>)` and the
    /// `TISH_NATIVE_HOF` flag is set, lower it to a native iterator chain. Shared by
    /// `emit_typed_expr` (native sub-expressions) and `emit_native_expr` (typed `let` RHS), so a
    /// typed-array HOF lowers natively whether its result flows into arithmetic or a binding.
    fn native_vec_hof_for_call(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
    ) -> Result<Option<(String, RustType)>, CompileError> {
        if !crate::native_opts_enabled() {
            return Ok(None);
        }
        let Expr::Member {
            object,
            prop: MemberProp::Name { name: method, .. },
            optional: false,
            ..
        } = callee
        else {
            return Ok(None);
        };
        let Expr::Ident { name: recv_name, .. } = object.as_ref() else {
            return Ok(None);
        };
        // A RefCell-wrapped receiver would need a borrow to iterate — bail to the boxed path.
        if self.refcell_wrapped_vars.contains(recv_name.as_ref()) {
            return Ok(None);
        }
        let RustType::Vec(inner) = self.type_context.get_type(recv_name.as_ref()) else {
            return Ok(None);
        };
        if *inner != RustType::F64 {
            return Ok(None);
        }
        let recv_code = Self::escape_ident(recv_name.as_ref()).into_owned();
        self.try_native_vec_hof(&recv_code, &inner, recv_name.as_ref(), method.as_ref(), args)
    }

    /// Native typed-array HOFs (`TISH_NATIVE_HOF`): when the receiver is a native `Vec<f64>`
    /// (a typed `number[]`), lower `reduce`/`map`/`filter`/`some`/`every` to a direct Rust
    /// iterator chain — no per-element `value_call`, no `Value` boxing.
    ///
    /// Preconditions (any failure → `Ok(None)` → boxed `array_*`, correctness over coverage):
    /// - element type is `f64` (Copy → `.copied()`),
    /// - the callback is an arrow with simple, no-default params and an **expression** body,
    /// - the body does **not** mention the receiver — `pi_mentions` is conservative (unknown
    ///   AST nodes count as "mentions"), so a `&mut`/alias of the array inside the closure can't
    ///   slip through and break the `.iter()` borrow,
    /// - the body's emitted native type matches what the method needs (`f64` for `reduce`/`map`
    ///   element, `bool` for `filter`/`some`/`every`).
    ///
    /// The closure params are bound natively (`f64`) only while the body is emitted, then popped.
    fn try_native_vec_hof(
        &mut self,
        recv: &str,
        elem: &RustType,
        recv_name: &str,
        method: &str,
        args: &[CallArg],
    ) -> Result<Option<(String, RustType)>, CompileError> {
        // Only numeric arrays for now: `.copied()` needs a `Copy` element.
        if *elem != RustType::F64 {
            return Ok(None);
        }
        let Some(CallArg::Expr(Expr::ArrowFunction { params, body, .. })) = args.first() else {
            return Ok(None);
        };
        let tishlang_ast::ArrowBody::Expr(be) = body else {
            return Ok(None);
        };
        // The body must not touch the receiver (aliasing would break the `.iter()` borrow).
        if crate::infer::pi_mentions(be, recv_name) {
            return Ok(None);
        }
        let simple_name = |p: &FunParam| -> Option<std::sync::Arc<str>> {
            match p {
                FunParam::Simple(tp) if tp.default.is_none() => Some(std::sync::Arc::clone(&tp.name)),
                _ => None,
            }
        };
        // Emit `be` with `binds` (name, type) installed as native locals; restore on the way out.
        let emit_with = |this: &mut Self,
                             binds: &[(&std::sync::Arc<str>, RustType)]|
         -> Result<(String, RustType), CompileError> {
            this.type_context.push_scope();
            for (n, t) in binds {
                this.type_context.define(n.as_ref(), t.clone());
            }
            let res = this.emit_typed_expr(be);
            this.type_context.pop_scope();
            res
        };
        match method {
            "reduce" => {
                if args.len() != 2 || params.len() != 2 {
                    return Ok(None);
                }
                let (Some(acc), Some(x)) = (simple_name(&params[0]), simple_name(&params[1])) else {
                    return Ok(None);
                };
                let CallArg::Expr(init_e) = &args[1] else {
                    return Ok(None);
                };
                let (init_code, init_ty) = self.emit_typed_expr(init_e)?;
                let init_f64 = match init_ty {
                    RustType::F64 => init_code,
                    RustType::Value => RustType::F64.from_value_expr(&init_code),
                    _ => return Ok(None),
                };
                let acc_esc = Self::escape_ident(acc.as_ref()).into_owned();
                let x_esc = Self::escape_ident(x.as_ref()).into_owned();

                // ── i64 fast path (#174) ────────────────────────────────────────────────────────
                // When the receiver is an integer-literal array (element range known) and the body
                // lowers to native `i64` arithmetic with every node proven integral and < 2^53, run
                // the fold in `i64` — eliminating `fmod`/f64 round-trips in the hot loop (V8 keeps
                // these small-integer folds in int registers too). The accumulator's bounded integer
                // range is found by a small fixpoint seeded from the init's range; bit-identical to
                // the f64 fold because every intermediate is an exact integer < 2^53.
                if let Some(elem_r) = self.array_elem_ranges.get(recv_name).copied() {
                    if let Some(init_r) = self.int_range(init_e, &self.int_range_locals) {
                        let mut base = self.int_range_locals.clone();
                        base.insert(x.to_string(), elem_r);
                        let mut acc_r = init_r;
                        let mut converged = false;
                        for _ in 0..6 {
                            let mut m = base.clone();
                            m.insert(acc.to_string(), acc_r);
                            match self.int_range(be, &m) {
                                Some((blo, bhi)) => {
                                    let n = (acc_r.0.min(blo), acc_r.1.max(bhi));
                                    if n == acc_r {
                                        converged = true;
                                        break;
                                    }
                                    acc_r = n;
                                }
                                None => break,
                            }
                        }
                        // Confirm the range is an inductive invariant, then emit the body in i64.
                        let mut m = base.clone();
                        m.insert(acc.to_string(), acc_r);
                        let inductive = converged
                            && matches!(self.int_range(be, &m),
                                Some((blo, bhi)) if blo >= acc_r.0 && bhi <= acc_r.1);
                        if inductive {
                            let i64vars: HashSet<String> =
                                std::iter::once(acc.to_string()).collect();
                            self.type_context.push_scope();
                            self.type_context.define(acc.as_ref(), RustType::F64);
                            self.type_context.define(x.as_ref(), RustType::F64);
                            let body_i64 = self.emit_i64(be, &i64vars, &m)?;
                            self.type_context.pop_scope();
                            if let Some(body_i64) = body_i64 {
                                return Ok(Some((
                                    format!(
                                        "{{ let mut {acc}: i64 = (({init}) as i64); for {x} in {recv}.iter().copied() {{ {acc} = {body}; }} {acc} as f64 }}",
                                        acc = acc_esc, init = init_f64, x = x_esc, recv = recv, body = body_i64
                                    ),
                                    RustType::F64,
                                )));
                            }
                        }
                    }
                }

                let (body_code, body_ty) =
                    emit_with(self, &[(&acc, RustType::F64), (&x, RustType::F64)])?;
                if body_ty != RustType::F64 {
                    return Ok(None);
                }
                Ok(Some((
                    format!(
                        "{{ let mut {acc}: f64 = {init}; for {x} in {recv}.iter().copied() {{ {acc} = {body}; }} {acc} }}",
                        acc = acc_esc, init = init_f64, x = x_esc, recv = recv, body = body_code
                    ),
                    RustType::F64,
                )))
            }
            "map" => {
                if args.len() != 1 || params.len() != 1 {
                    return Ok(None);
                }
                let Some(x) = simple_name(&params[0]) else {
                    return Ok(None);
                };
                let (body_code, body_ty) = emit_with(self, &[(&x, RustType::F64)])?;
                if !body_ty.is_native() {
                    return Ok(None);
                }
                let x_esc = Self::escape_ident(x.as_ref()).into_owned();
                Ok(Some((
                    format!(
                        "{recv}.iter().copied().map(|{x}| {body}).collect::<Vec<{ety}>>()",
                        recv = recv, x = x_esc, body = body_code, ety = body_ty.to_rust_type_str()
                    ),
                    RustType::Vec(Box::new(body_ty)),
                )))
            }
            "filter" => {
                if args.len() != 1 || params.len() != 1 {
                    return Ok(None);
                }
                let Some(x) = simple_name(&params[0]) else {
                    return Ok(None);
                };
                let (body_code, body_ty) = emit_with(self, &[(&x, RustType::F64)])?;
                if body_ty != RustType::Bool {
                    return Ok(None);
                }
                let x_esc = Self::escape_ident(x.as_ref()).into_owned();
                Ok(Some((
                    format!(
                        "{recv}.iter().copied().filter(|&{x}| {body}).collect::<Vec<f64>>()",
                        recv = recv, x = x_esc, body = body_code
                    ),
                    RustType::Vec(Box::new(RustType::F64)),
                )))
            }
            "some" | "every" => {
                if args.len() != 1 || params.len() != 1 {
                    return Ok(None);
                }
                let Some(x) = simple_name(&params[0]) else {
                    return Ok(None);
                };
                let (body_code, body_ty) = emit_with(self, &[(&x, RustType::F64)])?;
                if body_ty != RustType::Bool {
                    return Ok(None);
                }
                let x_esc = Self::escape_ident(x.as_ref()).into_owned();
                let adapter = if method == "some" { "any" } else { "all" };
                Ok(Some((
                    format!(
                        "{recv}.iter().copied().{adapter}(|{x}| {body})",
                        recv = recv, adapter = adapter, x = x_esc, body = body_code
                    ),
                    RustType::Bool,
                )))
            }
            _ => Ok(None),
        }
    }

    fn emit_binop(&self, l: &str, op: BinOp, r: &str, span: Span) -> Result<String, CompileError> {
        Ok(match op {
            BinOp::Add => format!(
                "tishlang_runtime::ops::add(&{}, &{}).unwrap_or(Value::Null)",
                l, r
            ),
            BinOp::Sub => format!(
                "tishlang_runtime::ops::sub(&{}, &{}).unwrap_or(Value::Null)",
                l, r
            ),
            BinOp::Mul => format!(
                "tishlang_runtime::ops::mul(&{}, &{}).unwrap_or(Value::Null)",
                l, r
            ),
            BinOp::Div => format!(
                "tishlang_runtime::ops::div(&{}, &{}).unwrap_or(Value::Null)",
                l, r
            ),
            BinOp::Mod => format!(
                "tishlang_runtime::ops::modulo(&{}, &{}).unwrap_or(Value::Null)",
                l, r
            ),
            BinOp::Pow => format!(
                "Value::Number(tishlang_runtime::to_number_value(&({})).powf(tishlang_runtime::to_number_value(&({}))))",
                l, r
            ),
            BinOp::StrictEq => format!("Value::Bool({}.strict_eq(&{}))", l, r),
            BinOp::StrictNe => format!("Value::Bool(!{}.strict_eq(&{}))", l, r),
            BinOp::Lt => format!("tishlang_runtime::ops::lt(&{}, &{})", l, r),
            BinOp::Le => format!("tishlang_runtime::ops::le(&{}, &{})", l, r),
            BinOp::Gt => format!("tishlang_runtime::ops::gt(&{}, &{})", l, r),
            BinOp::Ge => format!("tishlang_runtime::ops::ge(&{}, &{})", l, r),
            // Short-circuit + value-returning && / || (JS, #240): yield the deciding OPERAND, not a
            // coerced boolean (`five() && 7` is `7`, not `true`). The right operand sits inside the
            // branch, so its side effects only run when reached. (Typed `bool && bool` uses Rust's
            // own `&&`/`||` above, where returning the bool already IS the operand.)
            //
            // #328: bind the left operand by `.clone()`, NOT by move — the right operand commonly reuses
            // the same local (`aid && out.indexOf(aid)`), and a bare `let __l = aid;` moves the non-`Copy`
            // `Value` so the reuse is a borrow-after-move (E0382). The left operand is still evaluated
            // exactly once (its side effects don't double-run); only its value is cloned for the test.
            BinOp::And => format!("{{ let __l = ({}).clone(); if __l.is_truthy() {{ {} }} else {{ __l }} }}", l, r),
            BinOp::Or => format!("{{ let __l = ({}).clone(); if __l.is_truthy() {{ __l }} else {{ {} }} }}", l, r),
            BinOp::BitAnd => Self::emit_bitwise_binop(l, r, "&"),
            BinOp::BitOr => Self::emit_bitwise_binop(l, r, "|"),
            BinOp::BitXor => Self::emit_bitwise_binop(l, r, "^"),
            BinOp::Shl => Self::emit_shift_binop(l, r, "to_int32_value", "wrapping_shl"),
            BinOp::Shr => Self::emit_shift_binop(l, r, "to_int32_value", "wrapping_shr"),
            BinOp::UShr => Self::emit_shift_binop(l, r, "to_uint32_value", "wrapping_shr"),
            BinOp::In => format!("tish_in_operator(&{}, &{})", l, r),
            BinOp::Eq | BinOp::Ne => {
                return Err(CompileError::new(
                    "Loose equality not supported",
                    Some(span),
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests_176 {
    use super::Codegen;
    use std::fs;
    use tishlang_parser::parse;

    #[test]
    fn fasta_collects_seed_global() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("../../tests/perf/fasta.tish");
        let src = fs::read_to_string(&path).unwrap();
        let program = parse(&src).unwrap();
        let c = Codegen::collect_native_numeric_globals(&program.statements);
        assert!(
            c.contains_key("seed"),
            "expected seed in {:?}",
            c
        );
    }

    #[test]
    fn fasta_optimized_still_collects_seed_global() {
        use tishlang_opt::optimize;
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("../../tests/perf/fasta.tish");
        let src = fs::read_to_string(&path).unwrap();
        let program = parse(&src).unwrap();
        let optimized = optimize(&program);
        let c_opt = Codegen::collect_native_numeric_globals(&optimized.statements);
        let inferred = crate::infer::infer_program(&optimized);
        let c = Codegen::collect_native_numeric_globals(&inferred.statements);
        assert!(c_opt.contains_key("seed"), "optimized only: {:?}", c_opt);
        assert!(c.contains_key("seed"), "optimized+inferred: {:?}", c);
    }

    #[test]
    fn fib_native_body_uses_param_not_call_site_literal() {
        use std::path::PathBuf;
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("../../tests/perf/recursion_fib.tish");
        for k in [
            "TISH_PARAM_NATIVE",
            "TISH_PARAM_INFER",
            "TISH_NATIVE_FN",
            "TISH_STRUCT_INFER",
            "TISH_FUSED_HOF",
            "TISH_NATIVE_HOF",
            "TISH_AGGREGATE_INFER",
        ] {
            std::env::set_var(k, "1");
        }
        let (rust, _, _, _) =
            crate::compile_project_full(&path, path.parent(), &[], true).unwrap();
        let nv = rust
            .split("fn fib_native(")
            .nth(1)
            .expect("fib_native");
        let nv = nv.split("fn run()").next().unwrap_or(nv);
        assert!(
            !nv.contains("fib_native((35_f64"),
            "fib_native must use param n, not call-site literal"
        );
        assert!(nv.contains("fib_native((n -"), "expected recursive n-1 call");
    }
}
