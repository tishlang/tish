//! Shared JSX lowering: emit `h(tag, props, children)` as JavaScript or Rust (`Value`) source.

use std::collections::HashSet;

use tishlang_ast::{
    ArrayElement, ArrowBody, CallArg, ExportDeclaration, Expr, JsxAttrValue, JsxChild, JsxProp,
    Literal, MemberProp, ObjectProp, Program, Statement,
};

/// Escape a Tish identifier for Rust output (matches `tishlang_compile` conventions).
pub fn escape_ident_rust(s: &str) -> String {
    if s == "await" || s == "default" {
        format!("_{}", s)
    } else {
        s.to_string()
    }
}

/// Emit JSX expression as JavaScript (same rules as legacy `tishlang_compile_js`).
pub fn emit_jsx_js<F, E>(expr: &Expr, emit_expr: &mut F) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
    E: From<String>,
{
    match expr {
        Expr::JsxElement {
            tag,
            props,
            children,
            ..
        } => {
            let tag_str = if tag
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                tag.as_ref().to_string()
            } else {
                format!("{:?}", tag.as_ref())
            };
            let props_str = emit_jsx_props_js(props, emit_expr)?;
            let children_strs: Result<Vec<_>, _> = children
                .iter()
                .map(|c| emit_jsx_child_js(c, emit_expr))
                .collect();
            let children_str = children_strs?.join(", ");
            Ok(format!("h({}, {}, [{}])", tag_str, props_str, children_str))
        }
        Expr::JsxFragment { children, .. } => {
            let children_strs: Result<Vec<_>, _> = children
                .iter()
                .map(|c| emit_jsx_child_js(c, emit_expr))
                .collect();
            let children_str = children_strs?.join(", ");
            Ok(format!("h(Fragment, null, [{}])", children_str))
        }
        _ => Err(emit_err("emit_jsx_js: not a JSX expression")),
    }
}

fn emit_err<E>(msg: &str) -> E
where
    E: From<String>,
{
    E::from(msg.to_string())
}

fn emit_jsx_props_js<F, E>(props: &[JsxProp], emit_expr: &mut F) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
{
    if props.is_empty() {
        return Ok("null".to_string());
    }
    let parts: Result<Vec<_>, _> = props
        .iter()
        .map(|p| match p {
            JsxProp::Attr { name, value } => {
                let val = match value {
                    JsxAttrValue::String(s) => format!("{:?}", s.as_ref()),
                    JsxAttrValue::Expr(e) => emit_expr(e)?,
                    JsxAttrValue::ImplicitTrue => "true".to_string(),
                };
                let key = name.as_ref();
                Ok(if key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    format!("{}: {}", key, val)
                } else {
                    format!("{:?}: {}", key, val)
                })
            }
            JsxProp::Spread(e) => Ok(format!("...{}", emit_expr(e)?)),
        })
        .collect();
    Ok(format!("{{ {} }}", parts?.join(", ")))
}

fn emit_jsx_child_js<F, E>(child: &JsxChild, emit_expr: &mut F) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
{
    match child {
        JsxChild::Text(s) => Ok(format!("{:?}", s.as_ref())),
        JsxChild::Expr(e) => {
            let inner = emit_expr(e)?;
            let needs_string = matches!(
                e,
                Expr::Literal {
                    value: Literal::Number(_) | Literal::Bool(_) | Literal::Null,
                    ..
                }
            );
            Ok(if needs_string {
                format!("String({})", inner)
            } else {
                inner
            })
        }
    }
}

/// Every `fn Foo` name in the program (including nested bodies), for Rust JSX tag lowering.
///
/// PascalCase JSX tags that match a name here are emitted as a Rust identifier (component
/// `Value::Function`). Other PascalCase tags become `Value::String("Tag")` (native intrinsics).
pub fn collect_fun_decl_names(program: &Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for s in &program.statements {
        collect_fun_decl_names_stmt(s, &mut names);
    }
    names
}

fn collect_fun_decl_names_stmt(stmt: &Statement, names: &mut HashSet<String>) {
    match stmt {
        Statement::FunDecl { name, body, .. } => {
            names.insert(name.to_string());
            collect_fun_decl_names_stmt(body, names);
        }
        Statement::Block { statements, .. } => {
            for s in statements {
                collect_fun_decl_names_stmt(s, names);
            }
        }
        Statement::VarDecl { init, .. } => {
            if let Some(e) = init {
                collect_fun_decl_names_expr(e, names);
            }
        }
        Statement::VarDeclDestructure { init, .. } => collect_fun_decl_names_expr(init, names),
        Statement::ExprStmt { expr, .. } => collect_fun_decl_names_expr(expr, names),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_fun_decl_names_expr(cond, names);
            collect_fun_decl_names_stmt(then_branch, names);
            if let Some(e) = else_branch {
                collect_fun_decl_names_stmt(e, names);
            }
        }
        Statement::While { cond, body, .. } => {
            collect_fun_decl_names_expr(cond, names);
            collect_fun_decl_names_stmt(body, names);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                collect_fun_decl_names_stmt(i, names);
            }
            if let Some(c) = cond {
                collect_fun_decl_names_expr(c, names);
            }
            if let Some(u) = update {
                collect_fun_decl_names_expr(u, names);
            }
            collect_fun_decl_names_stmt(body, names);
        }
        Statement::ForOf { iterable, body, .. } => {
            collect_fun_decl_names_expr(iterable, names);
            collect_fun_decl_names_stmt(body, names);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                collect_fun_decl_names_expr(e, names);
            }
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            collect_fun_decl_names_expr(expr, names);
            for (ce, ss) in cases {
                if let Some(e) = ce {
                    collect_fun_decl_names_expr(e, names);
                }
                for s in ss {
                    collect_fun_decl_names_stmt(s, names);
                }
            }
            if let Some(ss) = default_body {
                for s in ss {
                    collect_fun_decl_names_stmt(s, names);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            collect_fun_decl_names_stmt(body, names);
            collect_fun_decl_names_expr(cond, names);
        }
        Statement::Throw { value, .. } => collect_fun_decl_names_expr(value, names),
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            collect_fun_decl_names_stmt(body, names);
            if let Some(c) = catch_body {
                collect_fun_decl_names_stmt(c, names);
            }
            if let Some(f) = finally_body {
                collect_fun_decl_names_stmt(f, names);
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => collect_fun_decl_names_stmt(inner, names),
            ExportDeclaration::Default(e) => collect_fun_decl_names_expr(e, names),
        },
        Statement::Import { .. } | Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

fn collect_fun_decl_names_expr(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::ArrowFunction { body, .. } => match body {
            ArrowBody::Expr(e) => collect_fun_decl_names_expr(e, names),
            ArrowBody::Block(s) => collect_fun_decl_names_stmt(s, names),
        },
        Expr::Binary { left, right, .. } => {
            collect_fun_decl_names_expr(left, names);
            collect_fun_decl_names_expr(right, names);
        }
        Expr::Unary { operand, .. } => collect_fun_decl_names_expr(operand, names),
        Expr::Assign { value, .. } => collect_fun_decl_names_expr(value, names),
        Expr::Call { callee, args, .. } => {
            collect_fun_decl_names_expr(callee, names);
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => collect_fun_decl_names_expr(e, names),
                }
            }
        }
        Expr::Member { object, prop, .. } => {
            collect_fun_decl_names_expr(object, names);
            if let MemberProp::Expr(e) = prop {
                collect_fun_decl_names_expr(e, names);
            }
        }
        Expr::Index { object, index, .. } => {
            collect_fun_decl_names_expr(object, names);
            collect_fun_decl_names_expr(index, names);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_fun_decl_names_expr(cond, names);
            collect_fun_decl_names_expr(then_branch, names);
            collect_fun_decl_names_expr(else_branch, names);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    ArrayElement::Expr(e) | ArrayElement::Spread(e) => {
                        collect_fun_decl_names_expr(e, names);
                    }
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => {
                        collect_fun_decl_names_expr(e, names);
                    }
                }
            }
        }
        Expr::NullishCoalesce { left, right, .. } => {
            collect_fun_decl_names_expr(left, names);
            collect_fun_decl_names_expr(right, names);
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                collect_fun_decl_names_expr(e, names);
            }
        }
        Expr::Await { operand, .. } | Expr::TypeOf { operand, .. } => {
            collect_fun_decl_names_expr(operand, names);
        }
        Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. } => {
            collect_fun_decl_names_expr(value, names);
        }
        Expr::MemberAssign { object, value, .. } => {
            collect_fun_decl_names_expr(object, names);
            collect_fun_decl_names_expr(value, names);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            collect_fun_decl_names_expr(object, names);
            collect_fun_decl_names_expr(index, names);
            collect_fun_decl_names_expr(value, names);
        }
        Expr::New { callee, args, .. } => {
            collect_fun_decl_names_expr(callee, names);
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => collect_fun_decl_names_expr(e, names),
                }
            }
        }
        Expr::PostfixInc { .. }
        | Expr::PrefixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixDec { .. } => {}
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    JsxProp::Attr { value, .. } => {
                        if let JsxAttrValue::Expr(e) = value {
                            collect_fun_decl_names_expr(e, names);
                        }
                    }
                    JsxProp::Spread(e) => collect_fun_decl_names_expr(e, names),
                }
            }
            for c in children {
                if let JsxChild::Expr(e) = c {
                    collect_fun_decl_names_expr(e, names);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for c in children {
                if let JsxChild::Expr(e) = c {
                    collect_fun_decl_names_expr(e, names);
                }
            }
        }
        Expr::Literal { .. } | Expr::Ident { .. } | Expr::NativeModuleLoad { .. } => {}
    }
}

/// Emit JSX as Rust `Value` by calling `tishlang_ui::ui_h` directly (no closure capture of a local `h` binding).
pub fn emit_jsx_rust<F, E>(
    expr: &Expr,
    emit_expr: &mut F,
    fun_decls: &HashSet<String>,
) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
    E: From<String>,
{
    match expr {
        Expr::JsxElement {
            tag,
            props,
            children,
            ..
        } => {
            let is_component = tag
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false);
            let tag_rust = if is_component {
                if fun_decls.contains(tag.as_ref()) {
                    escape_ident_rust(tag.as_ref())
                } else {
                    format!("Value::String({:?}.into())", tag.as_ref())
                }
            } else {
                format!("Value::String({:?}.into())", tag.as_ref())
            };
            let props_rust = emit_jsx_props_rust(props, emit_expr)?;
            let child_parts: Result<Vec<_>, _> = children
                .iter()
                .map(|c| emit_jsx_child_rust(c, emit_expr))
                .collect();
            let children_rust = format!(
                "Value::Array(Rc::new(RefCell::new(vec![{}])))",
                child_parts?.join(", ")
            );
            Ok(wrap_h_call_rust(&tag_rust, &props_rust, &children_rust))
        }
        Expr::JsxFragment { children, .. } => {
            let child_parts: Result<Vec<_>, _> = children
                .iter()
                .map(|c| emit_jsx_child_rust(c, emit_expr))
                .collect();
            let children_rust = format!(
                "Value::Array(Rc::new(RefCell::new(vec![{}])))",
                child_parts?.join(", ")
            );
            Ok(wrap_h_call_rust("Fragment", "Value::Null", &children_rust))
        }
        _ => Err(E::from("emit_jsx_rust: not a JSX expression".to_string())),
    }
}

fn wrap_h_call_rust(tag: &str, props: &str, children: &str) -> String {
    format!(
        "tishlang_ui::ui_h(&[({}).clone(), ({}).clone(), ({}).clone()])",
        tag, props, children
    )
}

fn emit_jsx_props_rust<F, E>(props: &[JsxProp], emit_expr: &mut F) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
    E: From<String>,
{
    if props.is_empty() {
        return Ok("Value::Null".to_string());
    }
    let has_spread = props.iter().any(|p| matches!(p, JsxProp::Spread(_)));
    if has_spread {
        let mut parts = Vec::new();
        for prop in props {
            match prop {
                JsxProp::Attr { name, value } => {
                    let val = match value {
                        JsxAttrValue::String(s) => {
                            format!("Value::String({:?}.into())", s.as_ref())
                        }
                        JsxAttrValue::Expr(e) => emit_expr(e)?,
                        JsxAttrValue::ImplicitTrue => "Value::Bool(true)".to_string(),
                    };
                    parts.push(format!(
                        "_obj.insert(Arc::from({:?}), ({}).clone());",
                        name.as_ref(),
                        val
                    ));
                }
                JsxProp::Spread(e) => {
                    let val = emit_expr(e)?;
                    parts.push(format!(
                        "if let Value::Object(ref _spread) = {} {{ for (k, v) in _spread.borrow().iter() {{ _obj.insert(Arc::clone(k), v.clone()); }} }}",
                        val
                    ));
                }
            }
        }
        Ok(format!(
            "{{ let mut _obj: ObjectMap = ObjectMap::default(); {} Value::Object(Rc::new(RefCell::new(_obj))) }}",
            parts.join(" ")
        ))
    } else {
        let mut kv = Vec::new();
        for prop in props {
            if let JsxProp::Attr { name, value } = prop {
                let val = match value {
                    JsxAttrValue::String(s) => {
                        format!("Value::String({:?}.into())", s.as_ref())
                    }
                    JsxAttrValue::Expr(e) => emit_expr(e)?,
                    JsxAttrValue::ImplicitTrue => "Value::Bool(true)".to_string(),
                };
                kv.push(format!(
                    "(Arc::from({:?}), ({}).clone())",
                    name.as_ref(),
                    val
                ));
            }
        }
        Ok(format!(
            "Value::Object(Rc::new(RefCell::new(ObjectMap::from([{}]))))",
            kv.join(", ")
        ))
    }
}

fn emit_jsx_child_rust<F, E>(child: &JsxChild, emit_expr: &mut F) -> Result<String, E>
where
    F: FnMut(&Expr) -> Result<String, E>,
    E: From<String>,
{
    match child {
        JsxChild::Text(s) => Ok(format!("Value::String({:?}.into())", s.as_ref())),
        JsxChild::Expr(e) => {
            let inner = emit_expr(e)?;
            let needs_string = matches!(
                e,
                Expr::Literal {
                    value: Literal::Number(_) | Literal::Bool(_) | Literal::Null,
                    ..
                }
            );
            Ok(if needs_string {
                format!("Value::String(({}).to_display_string().into())", inner)
            } else {
                format!("({}).clone()", inner)
            })
        }
    }
}

/// Whether the program contains any JSX syntax (for conditional native UI globals).
pub fn program_contains_jsx(program: &tishlang_ast::Program) -> bool {
    program.statements.iter().any(stmt_contains_jsx)
}

fn stmt_contains_jsx(stmt: &tishlang_ast::Statement) -> bool {
    use tishlang_ast::{ExportDeclaration, Statement};
    match stmt {
        Statement::Block { statements, .. } => statements.iter().any(stmt_contains_jsx),
        Statement::VarDecl { init, .. } => init.as_ref().is_some_and(expr_contains_jsx),
        Statement::VarDeclDestructure { init, .. } => expr_contains_jsx(init),
        Statement::ExprStmt { expr, .. } => expr_contains_jsx(expr),
        Statement::Return { value, .. } => value.as_ref().is_some_and(expr_contains_jsx),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_contains_jsx(cond)
                || stmt_contains_jsx(then_branch)
                || else_branch.as_ref().is_some_and(|s| stmt_contains_jsx(s))
        }
        Statement::While { cond, body, .. } | Statement::DoWhile { body, cond, .. } => {
            expr_contains_jsx(cond) || stmt_contains_jsx(body)
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|s| stmt_contains_jsx(s))
                || cond.as_ref().is_some_and(expr_contains_jsx)
                || update.as_ref().is_some_and(expr_contains_jsx)
                || stmt_contains_jsx(body)
        }
        Statement::ForOf { iterable, body, .. } => {
            expr_contains_jsx(iterable) || stmt_contains_jsx(body)
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            expr_contains_jsx(expr)
                || cases.iter().any(|(e, ss)| {
                    e.as_ref().is_some_and(expr_contains_jsx) || ss.iter().any(stmt_contains_jsx)
                })
                || default_body
                    .as_ref()
                    .is_some_and(|ss| ss.iter().any(stmt_contains_jsx))
        }
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            stmt_contains_jsx(body)
                || catch_body.as_ref().is_some_and(|s| stmt_contains_jsx(s))
                || finally_body.as_ref().is_some_and(|s| stmt_contains_jsx(s))
        }
        Statement::FunDecl { body, .. } => stmt_contains_jsx(body),
        Statement::Throw { value, .. } => expr_contains_jsx(value),
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => stmt_contains_jsx(inner),
            ExportDeclaration::Default(e) => expr_contains_jsx(e),
        },
        Statement::Import { .. } | Statement::Break { .. } | Statement::Continue { .. } => false,
    }
}

fn expr_contains_jsx(expr: &Expr) -> bool {
    match expr {
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => true,
        Expr::Binary { left, right, .. } => expr_contains_jsx(left) || expr_contains_jsx(right),
        Expr::Unary { operand, .. } => expr_contains_jsx(operand),
        Expr::Assign { value, .. } => expr_contains_jsx(value),
        Expr::Call { callee, args, .. } => {
            expr_contains_jsx(callee)
                || args.iter().any(|a| match a {
                    tishlang_ast::CallArg::Expr(e) | tishlang_ast::CallArg::Spread(e) => {
                        expr_contains_jsx(e)
                    }
                })
        }
        Expr::Member { object, prop, .. } => {
            expr_contains_jsx(object)
                || matches!(prop, tishlang_ast::MemberProp::Expr(e) if expr_contains_jsx(e))
        }
        Expr::Index { object, index, .. } => expr_contains_jsx(object) || expr_contains_jsx(index),
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            expr_contains_jsx(cond)
                || expr_contains_jsx(then_branch)
                || expr_contains_jsx(else_branch)
        }
        Expr::Array { elements, .. } => elements.iter().any(|el| match el {
            ArrayElement::Expr(e) | ArrayElement::Spread(e) => expr_contains_jsx(e),
        }),
        Expr::Object { props, .. } => props.iter().any(|p| match p {
            ObjectProp::KeyValue(_, e) | ObjectProp::Spread(e) => expr_contains_jsx(e),
        }),
        Expr::ArrowFunction { body, .. } => match body {
            tishlang_ast::ArrowBody::Expr(e) => expr_contains_jsx(e),
            tishlang_ast::ArrowBody::Block(s) => stmt_contains_jsx(s),
        },
        Expr::NullishCoalesce { left, right, .. } => {
            expr_contains_jsx(left) || expr_contains_jsx(right)
        }
        Expr::TemplateLiteral { exprs, .. } => exprs.iter().any(expr_contains_jsx),
        Expr::Await { operand, .. } => expr_contains_jsx(operand),
        Expr::TypeOf { operand, .. } => expr_contains_jsx(operand),
        Expr::PostfixInc { .. }
        | Expr::PrefixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixDec { .. } => false,
        Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. } => {
            expr_contains_jsx(value)
        }
        Expr::MemberAssign { object, value, .. } => {
            expr_contains_jsx(object) || expr_contains_jsx(value)
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => expr_contains_jsx(object) || expr_contains_jsx(index) || expr_contains_jsx(value),
        Expr::New { callee, args, .. } => {
            expr_contains_jsx(callee)
                || args.iter().any(|a| match a {
                    tishlang_ast::CallArg::Expr(e) | tishlang_ast::CallArg::Spread(e) => {
                        expr_contains_jsx(e)
                    }
                })
        }
        Expr::Literal { .. } | Expr::Ident { .. } | Expr::NativeModuleLoad { .. } => false,
    }
}
