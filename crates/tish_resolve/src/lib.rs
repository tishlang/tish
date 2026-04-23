//! Lexical name resolution for Tish (go-to-definition, hover, references).
//!
//! Coordinates: LSP uses 0-based lines and UTF-16 columns; [`tishlang_ast::Span`] uses 1-based
//! lines and 1-based **Unicode scalar** columns from the lexer. Conversion goes through byte
//! offsets in the original source string.

mod pos;

pub use pos::{lsp_position_for_span_start, span_contains_lsp_position, span_to_lsp_range_exclusive};

use std::collections::HashMap;
use std::sync::Arc;

use tishlang_ast::{
    ArrowBody, CallArg, DestructElement, DestructPattern, Expr, ExportDeclaration, FunParam,
    ImportSpecifier, MemberProp, Program, Statement, TypedParam,
};

/// Smallest source span covering the LSP cursor (definition site or reference).
#[derive(Debug, Clone)]
pub struct NameUse {
    pub name: Arc<str>,
    pub span: tishlang_ast::Span,
}

/// Find the tightest name under the cursor (identifier reference or binding).
pub fn name_at_cursor(program: &Program, source: &str, lsp_line: u32, lsp_character: u32) -> Option<NameUse> {
    let mut best: Option<(u64, NameUse)> = None;
    for s in &program.statements {
        collect_stmt(s, source, lsp_line, lsp_character, &mut best);
    }
    best.map(|(_, u)| u)
}

fn span_size(source: &str, span: &tishlang_ast::Span) -> u64 {
    pos::lex_span_byte_range(source, span)
        .map(|(a, b)| b.saturating_sub(a) as u64)
        .unwrap_or(u64::MAX)
}

fn consider(
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    span: &tishlang_ast::Span,
    name: Arc<str>,
    best: &mut Option<(u64, NameUse)>,
) {
    if !pos::span_contains_lsp_position(source, span, lsp_line, lsp_char) {
        return;
    }
    let sz = span_size(source, span);
    let nu = NameUse {
        name,
        span: *span,
    };
    match best {
        None => *best = Some((sz, nu)),
        Some((osz, _)) if sz < *osz => *best = Some((sz, nu)),
        _ => {}
    }
}

fn synthetic_name_span(start: (usize, usize), name: &str) -> tishlang_ast::Span {
    tishlang_ast::Span {
        start,
        end: (start.0, start.1.saturating_add(name.chars().count())),
    }
}

fn collect_stmt(
    stmt: &Statement,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, NameUse)>,
) {
    match stmt {
        Statement::VarDecl {
            name,
            name_span,
            init,
            ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            if let Some(e) = init {
                collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::VarDeclDestructure { pattern, init, .. } => {
            collect_destruct_pattern(pattern, source, lsp_line, lsp_char, best);
            collect_expr(init, source, lsp_line, lsp_char, best);
        }
        Statement::ExprStmt { expr, .. } => collect_expr(expr, source, lsp_line, lsp_char, best),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_expr(cond, source, lsp_line, lsp_char, best);
            collect_stmt(then_branch, source, lsp_line, lsp_char, best);
            if let Some(e) = else_branch {
                collect_stmt(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::While { cond, body, .. } => {
            collect_expr(cond, source, lsp_line, lsp_char, best);
            collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                collect_stmt(i, source, lsp_line, lsp_char, best);
            }
            if let Some(e) = cond {
                collect_expr(e, source, lsp_line, lsp_char, best);
            }
            if let Some(e) = update {
                collect_expr(e, source, lsp_line, lsp_char, best);
            }
            collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            collect_expr(iterable, source, lsp_line, lsp_char, best);
            collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::Block { statements, .. } => {
            for s in statements {
                collect_stmt(s, source, lsp_line, lsp_char, best);
            }
        }
        Statement::FunDecl {
            name,
            name_span,
            params,
            body,
            ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            for p in params {
                collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            collect_expr(expr, source, lsp_line, lsp_char, best);
            for (_ce, stmts) in cases {
                for s in stmts {
                    collect_stmt(s, source, lsp_line, lsp_char, best);
                }
            }
            if let Some(stmts) = default_body {
                for s in stmts {
                    collect_stmt(s, source, lsp_line, lsp_char, best);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            collect_stmt(body, source, lsp_line, lsp_char, best);
            collect_expr(cond, source, lsp_line, lsp_char, best);
        }
        Statement::Throw { value, .. } => collect_expr(value, source, lsp_line, lsp_char, best),
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            ..
        } => {
            collect_stmt(body, source, lsp_line, lsp_char, best);
            if let (Some(n), Some(sp)) = (catch_param, catch_param_span) {
                consider(source, lsp_line, lsp_char, sp, n.clone(), best);
            }
            if let Some(cb) = catch_body {
                collect_stmt(cb, source, lsp_line, lsp_char, best);
            }
            if let Some(fb) = finally_body {
                collect_stmt(fb, source, lsp_line, lsp_char, best);
            }
        }
        Statement::Import { specifiers, .. } => {
            for sp in specifiers {
                match sp {
                    ImportSpecifier::Named {
                        name,
                        name_span,
                        alias,
                        alias_span,
                    } => {
                        let local = alias.as_ref().map(|a| a.clone()).unwrap_or_else(|| name.clone());
                        let spn = alias_span.as_ref().unwrap_or(name_span);
                        consider(source, lsp_line, lsp_char, spn, local, best);
                    }
                    ImportSpecifier::Namespace { name, name_span } => {
                        consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
                    }
                    ImportSpecifier::Default { name, name_span } => {
                        consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
                    }
                }
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => collect_stmt(inner, source, lsp_line, lsp_char, best),
            ExportDeclaration::Default(e) => collect_expr(e, source, lsp_line, lsp_char, best),
        },
        Statement::TypeAlias { name, name_span, .. } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
        }
        Statement::DeclareVar { name, name_span, .. } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            for p in params {
                collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

fn collect_fun_param(
    p: &FunParam,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, NameUse)>,
) {
    match p {
        FunParam::Simple(tp) => collect_typed_param(tp, source, lsp_line, lsp_char, best),
        FunParam::Destructure { pattern, .. } => {
            collect_destruct_pattern(pattern, source, lsp_line, lsp_char, best);
        }
    }
}

fn collect_typed_param(
    tp: &TypedParam,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, NameUse)>,
) {
    consider(
        source,
        lsp_line,
        lsp_char,
        &tp.name_span,
        tp.name.clone(),
        best,
    );
    if let Some(e) = &tp.default {
        collect_expr(e, source, lsp_line, lsp_char, best);
    }
}

fn collect_destruct_pattern(
    p: &DestructPattern,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, NameUse)>,
) {
    match p {
        DestructPattern::Array(elements) => {
            for el in elements {
                if let Some(el) = el {
                    match el {
                        DestructElement::Ident(n, sp) => {
                            consider(source, lsp_line, lsp_char, sp, n.clone(), best);
                        }
                        DestructElement::Pattern(inner) => {
                            collect_destruct_pattern(inner, source, lsp_line, lsp_char, best);
                        }
                        DestructElement::Rest(n, sp) => {
                            consider(source, lsp_line, lsp_char, sp, n.clone(), best);
                        }
                    }
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(n, sp) => {
                        consider(source, lsp_line, lsp_char, sp, n.clone(), best);
                    }
                    DestructElement::Pattern(inner) => {
                        collect_destruct_pattern(inner, source, lsp_line, lsp_char, best);
                    }
                    DestructElement::Rest(n, sp) => {
                        consider(source, lsp_line, lsp_char, sp, n.clone(), best);
                    }
                }
            }
        }
    }
}

fn collect_expr(
    expr: &Expr,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, NameUse)>,
) {
    match expr {
        Expr::Ident { name, span } => consider(source, lsp_line, lsp_char, span, name.clone(), best),
        Expr::Literal { .. } => {}
        Expr::Binary { left, right, .. } => {
            collect_expr(left, source, lsp_line, lsp_char, best);
            collect_expr(right, source, lsp_line, lsp_char, best);
        }
        Expr::Unary { operand, .. } => collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::Call { callee, args, .. } => {
            collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => collect_expr(e, source, lsp_line, lsp_char, best),
                    CallArg::Spread(e) => collect_expr(e, source, lsp_line, lsp_char, best),
                }
            }
        }
        Expr::New { callee, args, .. } => {
            collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => collect_expr(e, source, lsp_line, lsp_char, best),
                    CallArg::Spread(e) => collect_expr(e, source, lsp_line, lsp_char, best),
                }
            }
        }
        Expr::Member { object, prop, .. } => {
            collect_expr(object, source, lsp_line, lsp_char, best);
            match prop {
                MemberProp::Name { name, span } => {
                    consider(source, lsp_line, lsp_char, span, name.clone(), best);
                }
                MemberProp::Expr(ix) => collect_expr(ix, source, lsp_line, lsp_char, best),
            }
        }
        Expr::Index { object, index, .. } => {
            collect_expr(object, source, lsp_line, lsp_char, best);
            collect_expr(index, source, lsp_line, lsp_char, best);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_expr(cond, source, lsp_line, lsp_char, best);
            collect_expr(then_branch, source, lsp_line, lsp_char, best);
            collect_expr(else_branch, source, lsp_line, lsp_char, best);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            collect_expr(left, source, lsp_line, lsp_char, best);
            collect_expr(right, source, lsp_line, lsp_char, best);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::ArrayElement::Spread(e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::Assign { name, span, value } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
            collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::TypeOf { operand, .. } => collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::PostfixInc { name, span } | Expr::PostfixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
        }
        Expr::PrefixInc { name, span } | Expr::PrefixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
        }
        Expr::CompoundAssign { name, span, value, .. }
        | Expr::LogicalAssign { name, span, value, .. } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
            collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::MemberAssign {
            object,
            value,
            ..
        } => {
            collect_expr(object, source, lsp_line, lsp_char, best);
            collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            collect_expr(object, source, lsp_line, lsp_char, best);
            collect_expr(index, source, lsp_line, lsp_char, best);
            collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            match body {
                ArrowBody::Expr(e) => collect_expr(e, source, lsp_line, lsp_char, best),
                ArrowBody::Block(b) => collect_stmt(b, source, lsp_line, lsp_char, best),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Expr::Await { operand, .. } => collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => match value {
                        tishlang_ast::JsxAttrValue::Expr(e) => {
                            collect_expr(e, source, lsp_line, lsp_char, best)
                        }
                        _ => {}
                    },
                    tishlang_ast::JsxProp::Spread(e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
            for ch in children {
                match ch {
                    tishlang_ast::JsxChild::Expr(e) => {
                        collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::JsxChild::Text(_) => {}
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    collect_expr(e, source, lsp_line, lsp_char, best);
                }
            }
        }
        Expr::NativeModuleLoad { .. } => {}
    }
}

/// For `a.b.c` with the cursor on property `c`, the root binding local name and member path `[b, c]`.
#[derive(Debug, Clone)]
pub struct MemberAccessChain {
    pub root_local: Arc<str>,
    pub members: Vec<Arc<str>>,
}

/// When the cursor sits on a static member name, resolve `root.local` plus `members` left-to-right after the root.
pub fn member_access_chain_at_cursor(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_character: u32,
) -> Option<MemberAccessChain> {
    let mut best: Option<(u64, MemberAccessChain)> = None;
    for s in &program.statements {
        member_chain_collect_stmt(s, source, lsp_line, lsp_character, &mut best);
    }
    best.map(|(_, c)| c)
}

fn chain_from_member_object(object: &Expr, rightmost: Arc<str>) -> Option<MemberAccessChain> {
    let mut members = vec![rightmost];
    let mut cur = object;
    loop {
        match cur {
            Expr::Member {
                object: o,
                prop: MemberProp::Name { name, .. },
                ..
            } => {
                members.push(name.clone());
                cur = o.as_ref();
            }
            Expr::Ident { name, .. } => {
                members.reverse();
                return Some(MemberAccessChain {
                    root_local: name.clone(),
                    members,
                });
            }
            _ => return None,
        }
    }
}

fn member_chain_try_update(
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    prop_span: &tishlang_ast::Span,
    object: &Expr,
    name: Arc<str>,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    if !pos::span_contains_lsp_position(source, prop_span, lsp_line, lsp_char) {
        return;
    }
    let Some(chain) = chain_from_member_object(object, name) else {
        return;
    };
    let sz = span_size(source, prop_span);
    match best {
        None => *best = Some((sz, chain)),
        Some((osz, _)) if sz < *osz => *best = Some((sz, chain)),
        _ => {}
    }
}

fn member_chain_collect_expr(
    expr: &Expr,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    match expr {
        Expr::Ident { .. } | Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => {}
        Expr::Binary { left, right, .. } => {
            member_chain_collect_expr(left, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(right, source, lsp_line, lsp_char, best);
        }
        Expr::Unary { operand, .. } => member_chain_collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::Call { callee, args, .. } => {
            member_chain_collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
                    CallArg::Spread(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
                }
            }
        }
        Expr::New { callee, args, .. } => {
            member_chain_collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
                    CallArg::Spread(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
                }
            }
        }
        Expr::Member { object, prop, .. } => {
            member_chain_collect_expr(object.as_ref(), source, lsp_line, lsp_char, best);
            match prop {
                MemberProp::Name { name, span } => {
                    member_chain_try_update(
                        source,
                        lsp_line,
                        lsp_char,
                        span,
                        object.as_ref(),
                        name.clone(),
                        best,
                    );
                }
                MemberProp::Expr(ix) => {
                    member_chain_collect_expr(ix, source, lsp_line, lsp_char, best);
                }
            }
        }
        Expr::Index { object, index, .. } => {
            member_chain_collect_expr(object, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(index, source, lsp_line, lsp_char, best);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            member_chain_collect_expr(cond, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(then_branch, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(else_branch, source, lsp_line, lsp_char, best);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            member_chain_collect_expr(left, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(right, source, lsp_line, lsp_char, best);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::ArrayElement::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::Assign { value, .. } | Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. } => {
            member_chain_collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::TypeOf { operand, .. } => member_chain_collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. } => {}
        Expr::MemberAssign { object, value, .. } => {
            member_chain_collect_expr(object, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            member_chain_collect_expr(object, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(index, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                member_chain_collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            match body {
                ArrowBody::Expr(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
                ArrowBody::Block(b) => member_chain_collect_stmt(b, source, lsp_line, lsp_char, best),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Expr::Await { operand, .. } => member_chain_collect_expr(operand, source, lsp_line, lsp_char, best),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
                }
            }
        }
    }
}

fn member_chain_collect_fun_param(
    p: &FunParam,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    match p {
        FunParam::Simple(tp) => {
            if let Some(e) = &tp.default {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        FunParam::Destructure { pattern, .. } => {
            member_chain_collect_destruct_pattern(pattern, source, lsp_line, lsp_char, best);
        }
    }
}

fn member_chain_collect_destruct_pattern(
    pattern: &DestructPattern,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements {
                if let Some(el) = el {
                    match el {
                        DestructElement::Ident(_, _) => {}
                        DestructElement::Pattern(inner) => {
                            member_chain_collect_destruct_pattern(inner, source, lsp_line, lsp_char, best)
                        }
                        DestructElement::Rest(_, _) => {}
                    }
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(_, _) => {}
                    DestructElement::Pattern(inner) => {
                        member_chain_collect_destruct_pattern(inner, source, lsp_line, lsp_char, best)
                    }
                    DestructElement::Rest(_, _) => {}
                }
            }
        }
    }
}

fn member_chain_collect_stmt(
    stmt: &Statement,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    match stmt {
        Statement::VarDecl { init, .. } => {
            if let Some(e) = init {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::VarDeclDestructure { pattern, init, .. } => {
            member_chain_collect_destruct_pattern(pattern, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(init, source, lsp_line, lsp_char, best);
        }
        Statement::ExprStmt { expr, .. } => member_chain_collect_expr(expr, source, lsp_line, lsp_char, best),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            member_chain_collect_expr(cond, source, lsp_line, lsp_char, best);
            member_chain_collect_stmt(then_branch, source, lsp_line, lsp_char, best);
            if let Some(e) = else_branch {
                member_chain_collect_stmt(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::While { cond, body, .. } => {
            member_chain_collect_expr(cond, source, lsp_line, lsp_char, best);
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                member_chain_collect_stmt(i, source, lsp_line, lsp_char, best);
            }
            if let Some(e) = cond {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
            if let Some(e) = update {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::ForOf { iterable, body, .. } => {
            member_chain_collect_expr(iterable, source, lsp_line, lsp_char, best);
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Statement::Block { statements, .. } => {
            for s in statements {
                member_chain_collect_stmt(s, source, lsp_line, lsp_char, best);
            }
        }
        Statement::FunDecl { params, body, .. } => {
            for p in params {
                member_chain_collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            member_chain_collect_expr(expr, source, lsp_line, lsp_char, best);
            for (_ce, stmts) in cases {
                for s in stmts {
                    member_chain_collect_stmt(s, source, lsp_line, lsp_char, best);
                }
            }
            if let Some(stmts) = default_body {
                for s in stmts {
                    member_chain_collect_stmt(s, source, lsp_line, lsp_char, best);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
            member_chain_collect_expr(cond, source, lsp_line, lsp_char, best);
        }
        Statement::Throw { value, .. } => member_chain_collect_expr(value, source, lsp_line, lsp_char, best),
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            member_chain_collect_stmt(body, source, lsp_line, lsp_char, best);
            if let Some(cb) = catch_body {
                member_chain_collect_stmt(cb, source, lsp_line, lsp_char, best);
            }
            if let Some(fb) = finally_body {
                member_chain_collect_stmt(fb, source, lsp_line, lsp_char, best);
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => member_chain_collect_stmt(inner, source, lsp_line, lsp_char, best),
            ExportDeclaration::Default(e) => member_chain_collect_expr(e, source, lsp_line, lsp_char, best),
        },
        Statement::Import { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => {}
    }
}

// --- resolve pass ---

struct ScopeStack(Vec<HashMap<String, tishlang_ast::Span>>);

impl ScopeStack {
    fn new() -> Self {
        Self(vec![HashMap::new()])
    }
    fn fork(&self) -> Self {
        Self(self.0.clone())
    }
    fn push(&mut self) {
        self.0.push(HashMap::new());
    }
    fn pop(&mut self) {
        let _ = self.0.pop();
    }
    fn define(&mut self, name: &str, span: tishlang_ast::Span) {
        if let Some(m) = self.0.last_mut() {
            m.insert(name.to_string(), span);
        }
    }
    fn resolve(&self, name: &str) -> Option<tishlang_ast::Span> {
        for m in self.0.iter().rev() {
            if let Some(s) = m.get(name) {
                return Some(*s);
            }
        }
        None
    }
}

fn define_fun_param_stack(p: &FunParam, scopes: &mut ScopeStack) {
    match p {
        FunParam::Simple(tp) => {
            scopes.define(tp.name.as_ref(), tp.name_span);
        }
        FunParam::Destructure { pattern, .. } => define_pattern_stack(pattern, scopes),
    }
}

fn define_pattern_stack(pattern: &DestructPattern, scopes: &mut ScopeStack) {
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements {
                if let Some(el) = el {
                    match el {
                        DestructElement::Ident(n, sp) => scopes.define(n.as_ref(), *sp),
                        DestructElement::Pattern(inner) => define_pattern_stack(inner, scopes),
                        DestructElement::Rest(n, sp) => scopes.define(n.as_ref(), *sp),
                    }
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(n, sp) => scopes.define(n.as_ref(), *sp),
                    DestructElement::Pattern(inner) => define_pattern_stack(inner, scopes),
                    DestructElement::Rest(n, sp) => scopes.define(n.as_ref(), *sp),
                }
            }
        }
    }
}

fn walk_expr_resolve(
    expr: &Expr,
    scopes: &ScopeStack,
    target: &NameUse,
) -> Option<tishlang_ast::Span> {
    let tgt = target.span;
    match expr {
        Expr::Ident { name, span } if *span == tgt && name.as_ref() == target.name.as_ref() => {
            scopes.resolve(name.as_ref())
        }
        Expr::Assign { name, span, value } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            if sp == tgt && name.as_ref() == target.name.as_ref() {
                return scopes.resolve(name.as_ref());
            }
            walk_expr_resolve(value, scopes, target)
        }
        Expr::CompoundAssign { name, span, value, .. } | Expr::LogicalAssign { name, span, value, .. } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            if sp == tgt && name.as_ref() == target.name.as_ref() {
                return scopes.resolve(name.as_ref());
            }
            walk_expr_resolve(value, scopes, target)
        }
        Expr::PostfixInc { name, span } | Expr::PostfixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            if sp == tgt && name.as_ref() == target.name.as_ref() {
                return scopes.resolve(name.as_ref());
            }
            None
        }
        Expr::PrefixInc { name, span } | Expr::PrefixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            if sp == tgt && name.as_ref() == target.name.as_ref() {
                return scopes.resolve(name.as_ref());
            }
            None
        }
        Expr::Binary { left, right, .. } => walk_expr_resolve(left, scopes, target)
            .or_else(|| walk_expr_resolve(right, scopes, target)),
        Expr::Unary { operand, .. } => walk_expr_resolve(operand, scopes, target),
        Expr::Call { callee, args, .. } => {
            walk_expr_resolve(callee, scopes, target).or_else(|| {
                for a in args {
                    let e = match a {
                        CallArg::Expr(e) => e,
                        CallArg::Spread(e) => e,
                    };
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
                None
            })
        }
        Expr::New { callee, args, .. } => {
            walk_expr_resolve(callee, scopes, target).or_else(|| {
                for a in args {
                    let e = match a {
                        CallArg::Expr(e) => e,
                        CallArg::Spread(e) => e,
                    };
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
                None
            })
        }
        Expr::Member { object, .. } => walk_expr_resolve(object, scopes, target),
        Expr::Index { object, index, .. } => walk_expr_resolve(object, scopes, target)
            .or_else(|| walk_expr_resolve(index, scopes, target)),
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => walk_expr_resolve(cond, scopes, target)
            .or_else(|| walk_expr_resolve(then_branch, scopes, target))
            .or_else(|| walk_expr_resolve(else_branch, scopes, target)),
        Expr::NullishCoalesce { left, right, .. } => walk_expr_resolve(left, scopes, target)
            .or_else(|| walk_expr_resolve(right, scopes, target)),
        Expr::Array { elements, .. } => {
            for el in elements {
                let e = match el {
                    tishlang_ast::ArrayElement::Expr(e) => e,
                    tishlang_ast::ArrayElement::Spread(e) => e,
                };
                if let Some(s) = walk_expr_resolve(e, scopes, target) {
                    return Some(s);
                }
            }
            None
        }
        Expr::Object { props, .. } => {
            for p in props {
                let e = match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                if let Some(s) = walk_expr_resolve(e, scopes, target) {
                    return Some(s);
                }
            }
            None
        }
        Expr::TypeOf { operand, .. } => walk_expr_resolve(operand, scopes, target),
        Expr::MemberAssign { object, value, .. } => walk_expr_resolve(object, scopes, target)
            .or_else(|| walk_expr_resolve(value, scopes, target)),
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => walk_expr_resolve(object, scopes, target)
            .or_else(|| walk_expr_resolve(index, scopes, target))
            .or_else(|| walk_expr_resolve(value, scopes, target)),
        Expr::ArrowFunction { params, body, .. } => {
            let mut inner = scopes.fork();
            inner.push();
            for p in params {
                define_fun_param_stack(p, &mut inner);
            }
            match body {
                ArrowBody::Expr(e) => walk_expr_resolve(e, &inner, target),
                ArrowBody::Block(b) => walk_stmt_resolve(b, &mut inner, target),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                if let Some(s) = walk_expr_resolve(e, scopes, target) {
                    return Some(s);
                }
            }
            None
        }
        Expr::Await { operand, .. } => walk_expr_resolve(operand, scopes, target),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            if let Some(s) = walk_expr_resolve(e, scopes, target) {
                                return Some(s);
                            }
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => {
                        if let Some(s) = walk_expr_resolve(e, scopes, target) {
                            return Some(s);
                        }
                    }
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
            }
            None
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
            }
            None
        }
        Expr::Ident { .. } => None,
        Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => None,
    }
}

fn walk_stmt_resolve(
    stmt: &Statement,
    scopes: &mut ScopeStack,
    target: &NameUse,
) -> Option<tishlang_ast::Span> {
    let tgt_span = target.span;
    match stmt {
        Statement::VarDecl {
            name,
            name_span,
            mutable: _,
            type_ann: _,
            init,
            ..
        } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            if let Some(e) = init {
                if let Some(s) = walk_expr_resolve(e, scopes, target) {
                    return Some(s);
                }
            }
            scopes.define(name.as_ref(), *name_span);
            None
        }
        Statement::VarDeclDestructure {
            pattern,
            init,
            ..
        } => {
            if let Some(s) = walk_expr_resolve(init, scopes, target) {
                return Some(s);
            }
            define_pattern_stack(pattern, scopes);
            None
        }
        Statement::ExprStmt { expr, .. } => walk_expr_resolve(expr, scopes, target),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            walk_expr_resolve(cond, scopes, target)
                .or_else(|| walk_stmt_implicit(then_branch, scopes, target))
                .or_else(|| else_branch.as_ref().and_then(|b| walk_stmt_implicit(b, scopes, target)))
        }
        Statement::While { cond, body, .. } => {
            walk_expr_resolve(cond, scopes, target).or_else(|| walk_stmt_implicit(body, scopes, target))
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            scopes.push();
            if let Some(i) = init {
                if let Some(s) = walk_stmt_resolve(i, scopes, target) {
                    scopes.pop();
                    return Some(s);
                }
            }
            let r = (|| {
                if let Some(e) = cond {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
                if let Some(e) = update {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        return Some(s);
                    }
                }
                walk_stmt_implicit(body, scopes, target)
            })();
            scopes.pop();
            r
        }
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            ..
        } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            if let Some(s) = walk_expr_resolve(iterable, scopes, target) {
                return Some(s);
            }
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            let r = walk_stmt_implicit(body, scopes, target);
            scopes.pop();
            r
        }
        Statement::Return { value, .. } => {
            value.as_ref().and_then(|e| walk_expr_resolve(e, scopes, target))
        }
        Statement::Block { statements, .. } => {
            scopes.push();
            let mut out = None;
            for s in statements {
                if let Some(x) = walk_stmt_resolve(s, scopes, target) {
                    out = Some(x);
                    break;
                }
            }
            scopes.pop();
            out
        }
        Statement::FunDecl {
            name,
            name_span,
            params,
            body,
            ..
        } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            let r = walk_stmt_resolve(body, scopes, target);
            scopes.pop();
            if r.is_some() {
                return r;
            }
            scopes.define(name.as_ref(), *name_span);
            None
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            if let Some(s) = walk_expr_resolve(expr, scopes, target) {
                return Some(s);
            }
            scopes.push();
            let mut out = None;
            for (_ce, stmts) in cases {
                for st in stmts {
                    if let Some(x) = walk_stmt_resolve(st, scopes, target) {
                        out = Some(x);
                        break;
                    }
                }
                if out.is_some() {
                    break;
                }
            }
            if out.is_none() {
                if let Some(stmts) = default_body {
                    for st in stmts {
                        if let Some(x) = walk_stmt_resolve(st, scopes, target) {
                            out = Some(x);
                            break;
                        }
                    }
                }
            }
            scopes.pop();
            out
        }
        Statement::DoWhile { body, cond, .. } => walk_stmt_implicit(body, scopes, target)
            .or_else(|| walk_expr_resolve(cond, scopes, target)),
        Statement::Throw { value, .. } => walk_expr_resolve(value, scopes, target),
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            ..
        } => {
            if let Some(s) = walk_stmt_resolve(body, scopes, target) {
                return Some(s);
            }
            if let (Some(n), Some(sp)) = (catch_param, catch_param_span) {
                if *sp == tgt_span && n.as_ref() == target.name.as_ref() {
                    return Some(*sp);
                }
            }
            if let Some(cb) = catch_body {
                scopes.push();
                if let (Some(n), Some(sp)) = (catch_param, catch_param_span) {
                    scopes.define(n.as_ref(), *sp);
                }
                let r = walk_stmt_resolve(cb, scopes, target);
                scopes.pop();
                if r.is_some() {
                    return r;
                }
            }
            if let Some(fb) = finally_body {
                return walk_stmt_resolve(fb, scopes, target);
            }
            None
        }
        Statement::Import { specifiers, .. } => {
            for sp in specifiers {
                match sp {
                    ImportSpecifier::Named {
                        name,
                        name_span,
                        alias,
                        alias_span,
                    } => {
                        let local = alias.as_ref().map(|a| a.clone()).unwrap_or_else(|| name.clone());
                        let spn = alias_span.as_ref().unwrap_or(name_span);
                        if *spn == tgt_span && local.as_ref() == target.name.as_ref() {
                            return Some(*spn);
                        }
                        scopes.define(local.as_ref(), *spn);
                    }
                    ImportSpecifier::Namespace { name, name_span } => {
                        if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                            return Some(*name_span);
                        }
                        scopes.define(name.as_ref(), *name_span);
                    }
                    ImportSpecifier::Default { name, name_span } => {
                        if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                            return Some(*name_span);
                        }
                        scopes.define(name.as_ref(), *name_span);
                    }
                }
            }
            None
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => walk_stmt_resolve(inner, scopes, target),
            ExportDeclaration::Default(e) => walk_expr_resolve(e, scopes, target),
        },
        Statement::TypeAlias { name, name_span, .. } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            scopes.define(name.as_ref(), *name_span);
            None
        }
        Statement::DeclareVar { name, name_span, .. } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            scopes.define(name.as_ref(), *name_span);
            None
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            ..
        } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            scopes.pop();
            None
        }
        Statement::Break { .. } | Statement::Continue { .. } => None,
    }
}

fn walk_stmt_implicit(
    stmt: &Statement,
    scopes: &mut ScopeStack,
    target: &NameUse,
) -> Option<tishlang_ast::Span> {
    if matches!(stmt, Statement::Block { .. }) {
        walk_stmt_resolve(stmt, scopes, target)
    } else {
        scopes.push();
        let r = walk_stmt_resolve(stmt, scopes, target);
        scopes.pop();
        r
    }
}

/// Identifier reference with no binding in lexical scope (same rules as [`definition_span`]).
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedIdentifier {
    pub name: Arc<str>,
    pub span: tishlang_ast::Span,
}

/// Names always present on the interpreter root scope (see `tishlang_eval`) and listed in
/// `stdlib/builtins.d.tish`. They must not produce "unresolved identifier" diagnostics when
/// used without a `let`/`import`.
pub fn is_runtime_global_ident(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "parseInt"
            | "parseFloat"
            | "decodeURI"
            | "encodeURI"
            | "Boolean"
            | "isFinite"
            | "isNaN"
            | "Infinity"
            | "NaN"
            | "Math"
            | "JSON"
            | "Object"
            | "Array"
            | "String"
            | "Date"
            | "Uint8Array"
            | "AudioContext"
            | "RegExp"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
    )
}

fn record_unresolved(scopes: &ScopeStack, name: &Arc<str>, span: tishlang_ast::Span, out: &mut Vec<UnresolvedIdentifier>) {
    if is_runtime_global_ident(name.as_ref()) {
        return;
    }
    if scopes.resolve(name.as_ref()).is_none() {
        out.push(UnresolvedIdentifier {
            name: name.clone(),
            span,
        });
    }
}

fn check_unresolved_expr(expr: &Expr, scopes: &ScopeStack, out: &mut Vec<UnresolvedIdentifier>) {
    match expr {
        Expr::Ident { name, span } => record_unresolved(scopes, name, *span, out),
        Expr::Assign { name, span, value } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_unresolved(scopes, name, sp, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::CompoundAssign { name, span, value, .. } | Expr::LogicalAssign { name, span, value, .. } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_unresolved(scopes, name, sp, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::PostfixInc { name, span } | Expr::PostfixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_unresolved(scopes, name, sp, out);
        }
        Expr::PrefixInc { name, span } | Expr::PrefixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_unresolved(scopes, name, sp, out);
        }
        Expr::Binary { left, right, .. } => {
            check_unresolved_expr(left, scopes, out);
            check_unresolved_expr(right, scopes, out);
        }
        Expr::Unary { operand, .. } => check_unresolved_expr(operand, scopes, out),
        Expr::Call { callee, args, .. } => {
            check_unresolved_expr(callee, scopes, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::New { callee, args, .. } => {
            check_unresolved_expr(callee, scopes, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::Member { object, prop, .. } => {
            check_unresolved_expr(object, scopes, out);
            if let tishlang_ast::MemberProp::Expr(ix) = prop {
                check_unresolved_expr(ix, scopes, out);
            }
        }
        Expr::Index { object, index, .. } => {
            check_unresolved_expr(object, scopes, out);
            check_unresolved_expr(index, scopes, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            check_unresolved_expr(cond, scopes, out);
            check_unresolved_expr(then_branch, scopes, out);
            check_unresolved_expr(else_branch, scopes, out);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            check_unresolved_expr(left, scopes, out);
            check_unresolved_expr(right, scopes, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                let e = match el {
                    tishlang_ast::ArrayElement::Expr(e) => e,
                    tishlang_ast::ArrayElement::Spread(e) => e,
                };
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                let e = match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::TypeOf { operand, .. } => check_unresolved_expr(operand, scopes, out),
        Expr::MemberAssign { object, value, .. } => {
            check_unresolved_expr(object, scopes, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            check_unresolved_expr(object, scopes, out);
            check_unresolved_expr(index, scopes, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::ArrowFunction { params, body, .. } => {
            let mut inner = scopes.fork();
            inner.push();
            for p in params {
                define_fun_param_stack(p, &mut inner);
            }
            match body {
                ArrowBody::Expr(e) => check_unresolved_expr(e, &inner, out),
                ArrowBody::Block(b) => check_unresolved_stmt(b, &mut inner, out),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::Await { operand, .. } => check_unresolved_expr(operand, scopes, out),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            check_unresolved_expr(e, scopes, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => check_unresolved_expr(e, scopes, out),
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    check_unresolved_expr(e, scopes, out);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    check_unresolved_expr(e, scopes, out);
                }
            }
        }
        Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => {}
    }
}

fn check_stmt_implicit_unresolved(stmt: &Statement, scopes: &mut ScopeStack, out: &mut Vec<UnresolvedIdentifier>) {
    if matches!(stmt, Statement::Block { .. }) {
        check_unresolved_stmt(stmt, scopes, out);
    } else {
        scopes.push();
        check_unresolved_stmt(stmt, scopes, out);
        scopes.pop();
    }
}

fn check_unresolved_stmt(stmt: &Statement, scopes: &mut ScopeStack, out: &mut Vec<UnresolvedIdentifier>) {
    match stmt {
        Statement::VarDecl {
            name,
            name_span,
            init,
            ..
        } => {
            if let Some(e) = init {
                check_unresolved_expr(e, scopes, out);
            }
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::VarDeclDestructure { pattern, init, .. } => {
            check_unresolved_expr(init, scopes, out);
            define_pattern_stack(pattern, scopes);
        }
        Statement::ExprStmt { expr, .. } => check_unresolved_expr(expr, scopes, out),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            check_unresolved_expr(cond, scopes, out);
            check_stmt_implicit_unresolved(then_branch, scopes, out);
            if let Some(b) = else_branch {
                check_stmt_implicit_unresolved(b, scopes, out);
            }
        }
        Statement::While { cond, body, .. } => {
            check_unresolved_expr(cond, scopes, out);
            check_stmt_implicit_unresolved(body, scopes, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            scopes.push();
            if let Some(i) = init {
                check_unresolved_stmt(i, scopes, out);
            }
            if let Some(e) = cond {
                check_unresolved_expr(e, scopes, out);
            }
            if let Some(e) = update {
                check_unresolved_expr(e, scopes, out);
            }
            check_stmt_implicit_unresolved(body, scopes, out);
            scopes.pop();
        }
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            ..
        } => {
            check_unresolved_expr(iterable, scopes, out);
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            check_stmt_implicit_unresolved(body, scopes, out);
            scopes.pop();
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                check_unresolved_expr(e, scopes, out);
            }
        }
        Statement::Block { statements, .. } => {
            scopes.push();
            for s in statements {
                check_unresolved_stmt(s, scopes, out);
            }
            scopes.pop();
        }
        Statement::FunDecl {
            name,
            name_span,
            params,
            body,
            ..
        } => {
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            check_unresolved_stmt(body, scopes, out);
            scopes.pop();
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            check_unresolved_expr(expr, scopes, out);
            scopes.push();
            for (_ce, stmts) in cases {
                for st in stmts {
                    check_unresolved_stmt(st, scopes, out);
                }
            }
            if let Some(stmts) = default_body {
                for st in stmts {
                    check_unresolved_stmt(st, scopes, out);
                }
            }
            scopes.pop();
        }
        Statement::DoWhile { body, cond, .. } => {
            check_stmt_implicit_unresolved(body, scopes, out);
            check_unresolved_expr(cond, scopes, out);
        }
        Statement::Throw { value, .. } => check_unresolved_expr(value, scopes, out),
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            ..
        } => {
            check_unresolved_stmt(body, scopes, out);
            if let Some(cb) = catch_body {
                scopes.push();
                if let (Some(n), Some(sp)) = (catch_param, catch_param_span) {
                    scopes.define(n.as_ref(), *sp);
                }
                check_unresolved_stmt(cb, scopes, out);
                scopes.pop();
            }
            if let Some(fb) = finally_body {
                check_unresolved_stmt(fb, scopes, out);
            }
        }
        Statement::Import { specifiers, .. } => {
            for sp in specifiers {
                match sp {
                    ImportSpecifier::Named {
                        name,
                        name_span,
                        alias,
                        alias_span,
                    } => {
                        let local = alias.as_ref().map(|a| a.clone()).unwrap_or_else(|| name.clone());
                        let spn = alias_span.as_ref().unwrap_or(name_span);
                        scopes.define(local.as_ref(), *spn);
                    }
                    ImportSpecifier::Namespace { name, name_span } => {
                        scopes.define(name.as_ref(), *name_span);
                    }
                    ImportSpecifier::Default { name, name_span } => {
                        scopes.define(name.as_ref(), *name_span);
                    }
                }
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => check_unresolved_stmt(inner, scopes, out),
            ExportDeclaration::Default(e) => check_unresolved_expr(e, scopes, out),
        },
        Statement::TypeAlias { name, name_span, .. } => {
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::DeclareVar { name, name_span, .. } => {
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            ..
        } => {
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            scopes.pop();
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Collect unresolved simple-name references across `program` (top-level scope accumulates like `definition_span`).
pub fn collect_unresolved_identifiers(program: &Program) -> Vec<UnresolvedIdentifier> {
    let mut out = Vec::new();
    let mut scopes = ScopeStack::new();
    for stmt in &program.statements {
        check_unresolved_stmt(stmt, &mut scopes, &mut out);
    }
    out
}

/// Classify an unused binding for editor messaging (mirrors common TS/ESLint groupings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnusedBindingKind {
    Import,
    Variable,
    Parameter,
}

/// A declared name that is never read (same resolution rules as [`reference_spans_for_def`]).
#[derive(Debug, Clone, PartialEq)]
pub struct UnusedBinding {
    pub name: Arc<str>,
    pub span: tishlang_ast::Span,
    pub kind: UnusedBindingKind,
}

#[derive(Debug, Clone)]
struct BindingSite {
    name: Arc<str>,
    span: tishlang_ast::Span,
    kind: UnusedBindingKind,
    exported: bool,
}

fn enumerate_pattern_bindings(
    pattern: &DestructPattern,
    kind: UnusedBindingKind,
    exported: bool,
    out: &mut Vec<BindingSite>,
) {
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements {
                if let Some(el) = el {
                    match el {
                        DestructElement::Ident(n, sp) => out.push(BindingSite {
                            name: n.clone(),
                            span: *sp,
                            kind,
                            exported,
                        }),
                        DestructElement::Pattern(inner) => {
                            enumerate_pattern_bindings(inner, kind, exported, out);
                        }
                        DestructElement::Rest(n, sp) => out.push(BindingSite {
                            name: n.clone(),
                            span: *sp,
                            kind,
                            exported,
                        }),
                    }
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(n, sp) => out.push(BindingSite {
                        name: n.clone(),
                        span: *sp,
                        kind,
                        exported,
                    }),
                    DestructElement::Pattern(inner) => {
                        enumerate_pattern_bindings(inner, kind, exported, out);
                    }
                    DestructElement::Rest(n, sp) => out.push(BindingSite {
                        name: n.clone(),
                        span: *sp,
                        kind,
                        exported,
                    }),
                }
            }
        }
    }
}

fn enumerate_fun_param(p: &FunParam, exported: bool, out: &mut Vec<BindingSite>) {
    match p {
        FunParam::Simple(tp) => {
            out.push(BindingSite {
                name: tp.name.clone(),
                span: tp.name_span,
                kind: UnusedBindingKind::Parameter,
                exported,
            });
            if let Some(e) = &tp.default {
                enumerate_expr(e, exported, out);
            }
        }
        FunParam::Destructure {
            pattern,
            default,
            ..
        } => {
            enumerate_pattern_bindings(pattern, UnusedBindingKind::Parameter, exported, out);
            if let Some(e) = default {
                enumerate_expr(e, exported, out);
            }
        }
    }
}

fn enumerate_expr(expr: &Expr, exported: bool, out: &mut Vec<BindingSite>) {
    match expr {
        Expr::Literal { .. } | Expr::Ident { .. } | Expr::NativeModuleLoad { .. } => {}
        Expr::Binary { left, right, .. } => {
            enumerate_expr(left, exported, out);
            enumerate_expr(right, exported, out);
        }
        Expr::Unary { operand, .. } => enumerate_expr(operand, exported, out),
        Expr::Call { callee, args, .. } => {
            enumerate_expr(callee, exported, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                enumerate_expr(e, exported, out);
            }
        }
        Expr::New { callee, args, .. } => {
            enumerate_expr(callee, exported, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                enumerate_expr(e, exported, out);
            }
        }
        Expr::Member { object, prop, .. } => {
            enumerate_expr(object, exported, out);
            if let MemberProp::Expr(ix) = prop {
                enumerate_expr(ix, exported, out);
            }
        }
        Expr::Index { object, index, .. } => {
            enumerate_expr(object, exported, out);
            enumerate_expr(index, exported, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            enumerate_expr(cond, exported, out);
            enumerate_expr(then_branch, exported, out);
            enumerate_expr(else_branch, exported, out);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            enumerate_expr(left, exported, out);
            enumerate_expr(right, exported, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                let e = match el {
                    tishlang_ast::ArrayElement::Expr(e) => e,
                    tishlang_ast::ArrayElement::Spread(e) => e,
                };
                enumerate_expr(e, exported, out);
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                let e = match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                enumerate_expr(e, exported, out);
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => {
            enumerate_expr(value, exported, out);
        }
        Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. } => {}
        Expr::TypeOf { operand, .. } => enumerate_expr(operand, exported, out),
        Expr::MemberAssign { object, value, .. } => {
            enumerate_expr(object, exported, out);
            enumerate_expr(value, exported, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            enumerate_expr(object, exported, out);
            enumerate_expr(index, exported, out);
            enumerate_expr(value, exported, out);
        }
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                enumerate_fun_param(p, exported, out);
            }
            match body {
                ArrowBody::Expr(e) => enumerate_expr(e, exported, out),
                ArrowBody::Block(b) => enumerate_stmt(b, exported, out),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                enumerate_expr(e, exported, out);
            }
        }
        Expr::Await { operand, .. } => enumerate_expr(operand, exported, out),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            enumerate_expr(e, exported, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => enumerate_expr(e, exported, out),
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    enumerate_expr(e, exported, out);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    enumerate_expr(e, exported, out);
                }
            }
        }
    }
}

fn enumerate_stmt(stmt: &Statement, exported: bool, out: &mut Vec<BindingSite>) {
    match stmt {
        Statement::Import { specifiers, .. } => {
            for sp in specifiers {
                match sp {
                    ImportSpecifier::Named {
                        name,
                        name_span,
                        alias,
                        alias_span,
                    } => {
                        let local = alias.as_ref().map(|a| a.clone()).unwrap_or_else(|| name.clone());
                        let spn = alias_span.as_ref().unwrap_or(name_span);
                        out.push(BindingSite {
                            name: local,
                            span: *spn,
                            kind: UnusedBindingKind::Import,
                            exported: false,
                        });
                    }
                    ImportSpecifier::Namespace { name, name_span } => {
                        out.push(BindingSite {
                            name: name.clone(),
                            span: *name_span,
                            kind: UnusedBindingKind::Import,
                            exported: false,
                        });
                    }
                    ImportSpecifier::Default { name, name_span } => {
                        out.push(BindingSite {
                            name: name.clone(),
                            span: *name_span,
                            kind: UnusedBindingKind::Import,
                            exported: false,
                        });
                    }
                }
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => enumerate_stmt(inner, true, out),
            ExportDeclaration::Default(e) => enumerate_expr(e, exported, out),
        },
        Statement::VarDecl {
            name,
            name_span,
            init,
            ..
        } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
            if let Some(e) = init {
                enumerate_expr(e, exported, out);
            }
        }
        Statement::VarDeclDestructure { pattern, init, .. } => {
            enumerate_pattern_bindings(pattern, UnusedBindingKind::Variable, exported, out);
            enumerate_expr(init, exported, out);
        }
        Statement::ExprStmt { expr, .. } => enumerate_expr(expr, exported, out),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            enumerate_expr(cond, exported, out);
            enumerate_stmt(then_branch, exported, out);
            if let Some(b) = else_branch {
                enumerate_stmt(b, exported, out);
            }
        }
        Statement::While { cond, body, .. } => {
            enumerate_expr(cond, exported, out);
            enumerate_stmt(body, exported, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                enumerate_stmt(i, exported, out);
            }
            if let Some(e) = cond {
                enumerate_expr(e, exported, out);
            }
            if let Some(e) = update {
                enumerate_expr(e, exported, out);
            }
            enumerate_stmt(body, exported, out);
        }
        Statement::ForOf {
            name,
            name_span,
            iterable,
            body,
            ..
        } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
            enumerate_expr(iterable, exported, out);
            enumerate_stmt(body, exported, out);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                enumerate_expr(e, exported, out);
            }
        }
        Statement::Block { statements, .. } => {
            for s in statements {
                enumerate_stmt(s, exported, out);
            }
        }
        Statement::FunDecl {
            name,
            name_span,
            params,
            rest_param,
            body,
            ..
        } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
            for p in params {
                enumerate_fun_param(p, exported, out);
            }
            if let Some(r) = rest_param {
                out.push(BindingSite {
                    name: r.name.clone(),
                    span: r.name_span,
                    kind: UnusedBindingKind::Parameter,
                    exported,
                });
            }
            enumerate_stmt(body, exported, out);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            enumerate_expr(expr, exported, out);
            for (ce, stmts) in cases {
                if let Some(e) = ce {
                    enumerate_expr(e, exported, out);
                }
                for st in stmts {
                    enumerate_stmt(st, exported, out);
                }
            }
            if let Some(stmts) = default_body {
                for st in stmts {
                    enumerate_stmt(st, exported, out);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            enumerate_stmt(body, exported, out);
            enumerate_expr(cond, exported, out);
        }
        Statement::Throw { value, .. } => enumerate_expr(value, exported, out),
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            ..
        } => {
            enumerate_stmt(body, exported, out);
            if let (Some(n), Some(sp), Some(cb)) = (catch_param, catch_param_span, catch_body) {
                out.push(BindingSite {
                    name: n.clone(),
                    span: *sp,
                    kind: UnusedBindingKind::Variable,
                    exported,
                });
                enumerate_stmt(cb, exported, out);
            }
            if let Some(fb) = finally_body {
                enumerate_stmt(fb, exported, out);
            }
        }
        Statement::TypeAlias { name, name_span, .. } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
        }
        Statement::DeclareVar { name, name_span, .. } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            rest_param,
            ..
        } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
            for p in params {
                enumerate_fun_param(p, exported, out);
            }
            if let Some(r) = rest_param {
                out.push(BindingSite {
                    name: r.name.clone(),
                    span: r.name_span,
                    kind: UnusedBindingKind::Parameter,
                    exported,
                });
            }
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Declarations whose values are never read (imports, locals, parameters). Skips `exported` module
/// bindings and names starting with `_` (common intentional-unused convention).
pub fn collect_unused_bindings(program: &Program, source: &str) -> Vec<UnusedBinding> {
    let mut sites = Vec::new();
    for s in &program.statements {
        enumerate_stmt(s, false, &mut sites);
    }
    let mut out = Vec::new();
    for site in sites {
        if site.exported || site.name.as_ref().starts_with('_') {
            continue;
        }
        let refs = reference_spans_for_def(program, source, site.name.as_ref(), site.span);
        if refs.len() == 1 {
            out.push(UnusedBinding {
                name: site.name,
                span: site.span,
                kind: site.kind,
            });
        }
    }
    out
}

/// Resolve go-to-definition for cursor position. Returns the defining [`tishlang_ast::Span`].
pub fn definition_span(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_character: u32,
) -> Option<tishlang_ast::Span> {
    let use_site = name_at_cursor(program, source, lsp_line, lsp_character)?;
    let mut scopes = ScopeStack::new();
    for stmt in &program.statements {
        if let Some(s) = walk_stmt_resolve(stmt, &mut scopes, &use_site) {
            return Some(s);
        }
    }
    None
}

/// Locals declared one level inside the innermost `Block` that contains the cursor (best-effort completion).
pub fn block_locals_containing_cursor(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_character: u32,
) -> Vec<(Arc<str>, tishlang_ast::Span)> {
    let mut out = Vec::new();
    for s in &program.statements {
        collect_block_locals(s, source, lsp_line, lsp_character, &mut out);
    }
    out
}

fn collect_block_locals(
    stmt: &Statement,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    out: &mut Vec<(Arc<str>, tishlang_ast::Span)>,
) {
    match stmt {
        Statement::Block { statements, span } => {
            if pos::span_contains_lsp_position(source, span, lsp_line, lsp_char) {
                for st in statements {
                    match st {
                        Statement::VarDecl {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::FunDecl {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::TypeAlias {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::DeclareVar {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::DeclareFun {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        _ => {}
                    }
                }
            }
            for st in statements {
                collect_block_locals(st, source, lsp_line, lsp_char, out);
            }
        }
        Statement::FunDecl { body, .. } | Statement::While { body, .. } => {
            collect_block_locals(body, source, lsp_line, lsp_char, out);
        }
        Statement::ForOf {
            name,
            name_span,
            body,
            span,
            ..
        } => {
            if pos::span_contains_lsp_position(source, span, lsp_line, lsp_char)
                && pos::span_contains_lsp_position(source, &body.as_ref().span(), lsp_line, lsp_char)
            {
                out.push((name.clone(), *name_span));
            }
            collect_block_locals(body, source, lsp_line, lsp_char, out);
        }
        Statement::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_block_locals(then_branch, source, lsp_line, lsp_char, out);
            if let Some(b) = else_branch {
                collect_block_locals(b, source, lsp_line, lsp_char, out);
            }
        }
        Statement::For { init, body, .. } => {
            if let Some(i) = init {
                collect_block_locals(i, source, lsp_line, lsp_char, out);
            }
            collect_block_locals(body, source, lsp_line, lsp_char, out);
        }
        Statement::DoWhile { body, .. } => collect_block_locals(body, source, lsp_line, lsp_char, out),
        Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            ..
        } => {
            collect_block_locals(body, source, lsp_line, lsp_char, out);
            if let (Some(n), Some(ps), Some(cb)) = (catch_param, catch_param_span, catch_body) {
                if pos::span_contains_lsp_position(source, &cb.as_ref().span(), lsp_line, lsp_char) {
                    out.push((n.clone(), *ps));
                }
                collect_block_locals(cb, source, lsp_line, lsp_char, out);
            }
            if let Some(fb) = finally_body {
                collect_block_locals(fb, source, lsp_line, lsp_char, out);
            }
        }
        Statement::Switch { cases, default_body, .. } => {
            for (_ce, stmts) in cases {
                for st in stmts {
                    collect_block_locals(st, source, lsp_line, lsp_char, out);
                }
            }
            if let Some(stmts) = default_body {
                for st in stmts {
                    collect_block_locals(st, source, lsp_line, lsp_char, out);
                }
            }
        }
        Statement::Export { declaration, .. } => {
            if let ExportDeclaration::Named(inner) = declaration.as_ref() {
                collect_block_locals(inner, source, lsp_line, lsp_char, out);
            }
        }
        _ => {}
    }
}

fn param_layer_names(params: &[FunParam], rest_param: &Option<TypedParam>) -> Vec<Arc<str>> {
    let mut v = Vec::new();
    for p in params {
        match p {
            FunParam::Simple(tp) => v.push(tp.name.clone()),
            FunParam::Destructure { pattern, .. } => collect_pattern_binding_names(pattern, &mut v),
        }
    }
    if let Some(r) = rest_param {
        v.push(r.name.clone());
    }
    v
}

fn collect_pattern_binding_names(pattern: &DestructPattern, out: &mut Vec<Arc<str>>) {
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements {
                if let Some(el) = el {
                    match el {
                        DestructElement::Ident(n, _) => out.push(n.clone()),
                        DestructElement::Pattern(inner) => collect_pattern_binding_names(inner, out),
                        DestructElement::Rest(n, _) => out.push(n.clone()),
                    }
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(n, _) => out.push(n.clone()),
                    DestructElement::Pattern(inner) => collect_pattern_binding_names(inner, out),
                    DestructElement::Rest(n, _) => out.push(n.clone()),
                }
            }
        }
    }
}

fn record_callable_stack(
    stack: &[Vec<Arc<str>>],
    best: &mut Option<(usize, Vec<Arc<str>>)>,
) {
    let depth = stack.len();
    let flat: Vec<Arc<str>> = stack
        .iter()
        .rev()
        .flat_map(|layer| layer.iter().cloned())
        .collect();
    match best {
        None => *best = Some((depth, flat)),
        Some((bd, _)) if depth > *bd => *best = Some((depth, flat)),
        _ => {}
    }
}

fn walk_expr_completion(
    expr: &Expr,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    stack: &mut Vec<Vec<Arc<str>>>,
    best: &mut Option<(usize, Vec<Arc<str>>)>,
) {
    let sp = expr.span();
    if !pos::span_contains_lsp_position(source, &sp, lsp_line, lsp_char) {
        return;
    }
    match expr {
        Expr::ArrowFunction { params, body, .. } => {
            let layer = param_layer_names(params, &None);
            stack.push(layer);
            record_callable_stack(stack, best);
            for p in params {
                match p {
                    FunParam::Simple(tp) => {
                        if let Some(e) = &tp.default {
                            walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                        }
                    }
                    FunParam::Destructure { default: Some(e), .. } => {
                        walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                    }
                    _ => {}
                }
            }
            match body {
                ArrowBody::Expr(e) => walk_expr_completion(e, source, lsp_line, lsp_char, stack, best),
                ArrowBody::Block(b) => {
                    walk_stmt_completion(b, source, lsp_line, lsp_char, stack, best);
                }
            }
            stack.pop();
        }
        Expr::Binary { left, right, .. } => {
            walk_expr_completion(left, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(right, source, lsp_line, lsp_char, stack, best);
        }
        Expr::Unary { operand, .. } => walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best),
        Expr::Call { callee, args, .. } => {
            walk_expr_completion(callee, source, lsp_line, lsp_char, stack, best);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        }
        Expr::New { callee, args, .. } => {
            walk_expr_completion(callee, source, lsp_line, lsp_char, stack, best);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        }
        Expr::Member { object, .. } => walk_expr_completion(object, source, lsp_line, lsp_char, stack, best),
        Expr::Index { object, index, .. } => {
            walk_expr_completion(object, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(index, source, lsp_line, lsp_char, stack, best);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            walk_expr_completion(cond, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(then_branch, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(else_branch, source, lsp_line, lsp_char, stack, best);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            walk_expr_completion(left, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(right, source, lsp_line, lsp_char, stack, best);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                let e = match el {
                    tishlang_ast::ArrayElement::Expr(e) => e,
                    tishlang_ast::ArrayElement::Spread(e) => e,
                };
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                let e = match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => {
            walk_expr_completion(value, source, lsp_line, lsp_char, stack, best);
        }
        Expr::TypeOf { operand, .. } => walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best),
        Expr::MemberAssign { object, value, .. } => {
            walk_expr_completion(object, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(value, source, lsp_line, lsp_char, stack, best);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            walk_expr_completion(object, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(index, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(value, source, lsp_line, lsp_char, stack, best);
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        }
        Expr::Await { operand, .. } => walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => {
                        walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                    }
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                }
            }
        }
        Expr::Ident { .. }
        | Expr::Literal { .. }
        | Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. }
        | Expr::NativeModuleLoad { .. } => {}
    }
}

fn walk_stmt_completion(
    stmt: &Statement,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    stack: &mut Vec<Vec<Arc<str>>>,
    best: &mut Option<(usize, Vec<Arc<str>>)>,
) {
    let st_span = stmt.span();
    if !pos::span_contains_lsp_position(source, &st_span, lsp_line, lsp_char) {
        return;
    }
    match stmt {
        Statement::VarDecl { init: Some(e), .. } => {
            walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
        }
        Statement::VarDecl { .. } => {}
        Statement::VarDeclDestructure { init, .. } => {
            walk_expr_completion(init, source, lsp_line, lsp_char, stack, best);
        }
        Statement::ExprStmt { expr, .. } => {
            walk_expr_completion(expr, source, lsp_line, lsp_char, stack, best);
        }
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            walk_expr_completion(cond, source, lsp_line, lsp_char, stack, best);
            walk_stmt_completion(then_branch, source, lsp_line, lsp_char, stack, best);
            if let Some(b) = else_branch {
                walk_stmt_completion(b, source, lsp_line, lsp_char, stack, best);
            }
        }
        Statement::While { cond, body, .. } => {
            walk_expr_completion(cond, source, lsp_line, lsp_char, stack, best);
            walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                walk_stmt_completion(i, source, lsp_line, lsp_char, stack, best);
            }
            if let Some(e) = cond {
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
            if let Some(e) = update {
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
            walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
        }
        Statement::ForOf {
            name,
            iterable,
            body,
            ..
        } => {
            walk_expr_completion(iterable, source, lsp_line, lsp_char, stack, best);
            let body_sp = body.as_ref().span();
            if pos::span_contains_lsp_position(source, &body_sp, lsp_line, lsp_char) {
                stack.push(vec![name.clone()]);
                record_callable_stack(stack, best);
                walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
                stack.pop();
            }
        }
        Statement::Return { value: Some(e), .. } => {
            walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
        }
        Statement::Return { .. } => {}
        Statement::Block { statements, .. } => {
            for s in statements {
                walk_stmt_completion(s, source, lsp_line, lsp_char, stack, best);
            }
        }
        Statement::FunDecl {
            params,
            rest_param,
            body,
            ..
        } => {
            let layer = param_layer_names(params, rest_param);
            stack.push(layer);
            record_callable_stack(stack, best);
            for p in params {
                match p {
                    FunParam::Simple(tp) => {
                        if let Some(e) = &tp.default {
                            walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                        }
                    }
                    FunParam::Destructure { default: Some(e), .. } => {
                        walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                    }
                    _ => {}
                }
            }
            walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
            stack.pop();
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            walk_expr_completion(expr, source, lsp_line, lsp_char, stack, best);
            for (_ce, stmts) in cases {
                for s in stmts {
                    walk_stmt_completion(s, source, lsp_line, lsp_char, stack, best);
                }
            }
            if let Some(stmts) = default_body {
                for s in stmts {
                    walk_stmt_completion(s, source, lsp_line, lsp_char, stack, best);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
            walk_expr_completion(cond, source, lsp_line, lsp_char, stack, best);
        }
        Statement::Throw { value, .. } => {
            walk_expr_completion(value, source, lsp_line, lsp_char, stack, best);
        }
        Statement::Try {
            body,
            catch_param,
            catch_body,
            finally_body,
            ..
        } => {
            walk_stmt_completion(body, source, lsp_line, lsp_char, stack, best);
            if let (Some(n), Some(cb)) = (catch_param, catch_body) {
                let csp = cb.as_ref().span();
                if pos::span_contains_lsp_position(source, &csp, lsp_line, lsp_char) {
                    stack.push(vec![n.clone()]);
                    record_callable_stack(stack, best);
                    walk_stmt_completion(cb, source, lsp_line, lsp_char, stack, best);
                    stack.pop();
                }
            }
            if let Some(fb) = finally_body {
                walk_stmt_completion(fb, source, lsp_line, lsp_char, stack, best);
            }
        }
        Statement::Import { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => {}
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => {
                walk_stmt_completion(inner, source, lsp_line, lsp_char, stack, best);
            }
            ExportDeclaration::Default(e) => {
                walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
            }
        },
    }
}

/// Callable parameter names visible at the cursor (innermost layer first in the returned `Vec`).
pub fn callable_param_names_at_cursor(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
) -> Vec<Arc<str>> {
    let mut best: Option<(usize, Vec<Arc<str>>)> = None;
    let mut stack: Vec<Vec<Arc<str>>> = Vec::new();
    for s in &program.statements {
        walk_stmt_completion(s, source, lsp_line, lsp_char, &mut stack, &mut best);
    }
    best.map(|(_, n)| n).unwrap_or_default()
}

/// Value names for completion: inner callable parameters, then block-local `let`/`fn`, then module bindings.
pub fn completion_value_names_at_cursor(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
) -> Vec<Arc<str>> {
    use std::collections::HashSet;
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    let push = |n: Arc<str>, out: &mut Vec<Arc<str>>, seen: &mut HashSet<String>| {
        if seen.insert(n.to_string()) {
            out.push(n);
        }
    };
    for n in callable_param_names_at_cursor(program, source, lsp_line, lsp_char) {
        push(n, &mut out, &mut seen);
    }
    for (n, _) in block_locals_containing_cursor(program, source, lsp_line, lsp_char) {
        push(n, &mut out, &mut seen);
    }
    for (n, _) in shallow_module_bindings(program) {
        push(n, &mut out, &mut seen);
    }
    out
}

/// Top-level value bindings (completion baseline). Inner scopes handled in LSP via [`definition_span`].
pub fn shallow_module_bindings(program: &Program) -> Vec<(Arc<str>, tishlang_ast::Span)> {
    let mut out = Vec::new();
    for s in &program.statements {
        match s {
            Statement::VarDecl {
                name,
                name_span,
                ..
            } => out.push((name.clone(), *name_span)),
            Statement::FunDecl {
                name,
                name_span,
                ..
            } => out.push((name.clone(), *name_span)),
            Statement::Import { specifiers, .. } => {
                for sp in specifiers {
                    match sp {
                        ImportSpecifier::Named {
                            name,
                            name_span,
                            alias,
                            alias_span,
                        } => {
                            let local = alias.as_ref().map(|a| a.clone()).unwrap_or_else(|| name.clone());
                            let spn = alias_span.as_ref().unwrap_or(name_span);
                            out.push((local, *spn));
                        }
                        ImportSpecifier::Namespace { name, name_span } => {
                            out.push((name.clone(), *name_span));
                        }
                        ImportSpecifier::Default { name, name_span } => {
                            out.push((name.clone(), *name_span));
                        }
                    }
                }
            }
            Statement::Export { declaration, .. } => {
                if let ExportDeclaration::Named(inner) = declaration.as_ref() {
                    match inner.as_ref() {
                        Statement::VarDecl {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::FunDecl {
                            name,
                            name_span,
                            ..
                        } => out.push((name.clone(), *name_span)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// All spans (definition + uses) that resolve to `def_span` for `name`.
pub fn reference_spans_for_def(
    program: &Program,
    source: &str,
    name: &str,
    def_span: tishlang_ast::Span,
) -> Vec<tishlang_ast::Span> {
    let mut out = vec![def_span];
    for s in &program.statements {
        refs_stmt(s, program, source, name, def_span, &mut out);
    }
    out.sort_by_key(|s| (s.start.0, s.start.1, s.end.0, s.end.1));
    out.dedup_by(|a, b| a == b);
    out
}

fn refs_stmt(
    stmt: &Statement,
    program: &Program,
    source: &str,
    name: &str,
    def_span: tishlang_ast::Span,
    out: &mut Vec<tishlang_ast::Span>,
) {
    match stmt {
        Statement::VarDecl { init, .. } => {
            if let Some(e) = init {
                refs_expr(e, program, source, name, def_span, out);
            }
        }
        Statement::VarDeclDestructure { init, .. } => refs_expr(init, program, source, name, def_span, out),
        Statement::ExprStmt { expr, .. } => refs_expr(expr, program, source, name, def_span, out),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            refs_expr(cond, program, source, name, def_span, out);
            refs_stmt(then_branch, program, source, name, def_span, out);
            if let Some(b) = else_branch {
                refs_stmt(b, program, source, name, def_span, out);
            }
        }
        Statement::While { cond, body, .. } => {
            refs_expr(cond, program, source, name, def_span, out);
            refs_stmt(body, program, source, name, def_span, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                refs_stmt(i, program, source, name, def_span, out);
            }
            if let Some(e) = cond {
                refs_expr(e, program, source, name, def_span, out);
            }
            if let Some(e) = update {
                refs_expr(e, program, source, name, def_span, out);
            }
            refs_stmt(body, program, source, name, def_span, out);
        }
        Statement::ForOf {
            iterable,
            body,
            ..
        } => {
            refs_expr(iterable, program, source, name, def_span, out);
            refs_stmt(body, program, source, name, def_span, out);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                refs_expr(e, program, source, name, def_span, out);
            }
        }
        Statement::Block { statements, .. } => {
            for s in statements {
                refs_stmt(s, program, source, name, def_span, out);
            }
        }
        Statement::FunDecl { params, body, .. } => {
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if let Some(e) = &tp.default {
                        refs_expr(e, program, source, name, def_span, out);
                    }
                } else if let FunParam::Destructure { default: Some(e), .. } = p {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
            refs_stmt(body, program, source, name, def_span, out);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            refs_expr(expr, program, source, name, def_span, out);
            for (_ce, stmts) in cases {
                for s in stmts {
                    refs_stmt(s, program, source, name, def_span, out);
                }
            }
            if let Some(stmts) = default_body {
                for s in stmts {
                    refs_stmt(s, program, source, name, def_span, out);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            refs_stmt(body, program, source, name, def_span, out);
            refs_expr(cond, program, source, name, def_span, out);
        }
        Statement::Throw { value, .. } => refs_expr(value, program, source, name, def_span, out),
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            refs_stmt(body, program, source, name, def_span, out);
            if let Some(cb) = catch_body {
                refs_stmt(cb, program, source, name, def_span, out);
            }
            if let Some(fb) = finally_body {
                refs_stmt(fb, program, source, name, def_span, out);
            }
        }
        Statement::Import { .. }
        | Statement::Export { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => {}
    }
}

fn refs_expr(
    expr: &Expr,
    program: &Program,
    source: &str,
    name: &str,
    def_span: tishlang_ast::Span,
    out: &mut Vec<tishlang_ast::Span>,
) {
    let maybe_push = |span: &tishlang_ast::Span, n: &Arc<str>, out: &mut Vec<tishlang_ast::Span>| {
        if n.as_ref() != name {
            return;
        }
        let Some((l, c)) = pos::lsp_position_for_span_start(source, span) else {
            return;
        };
        if definition_span(program, source, l, c) == Some(def_span) {
            out.push(*span);
        }
    };

    match expr {
        Expr::Ident { name: n, span } => maybe_push(span, n, out),
        Expr::Binary { left, right, .. } => {
            refs_expr(left, program, source, name, def_span, out);
            refs_expr(right, program, source, name, def_span, out);
        }
        Expr::Unary { operand, .. } => refs_expr(operand, program, source, name, def_span, out),
        Expr::Call { callee, args, .. } => {
            refs_expr(callee, program, source, name, def_span, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                refs_expr(e, program, source, name, def_span, out);
            }
        }
        Expr::New { callee, args, .. } => {
            refs_expr(callee, program, source, name, def_span, out);
            for a in args {
                let e = match a {
                    CallArg::Expr(e) => e,
                    CallArg::Spread(e) => e,
                };
                refs_expr(e, program, source, name, def_span, out);
            }
        }
        Expr::Member { object, .. } => refs_expr(object, program, source, name, def_span, out),
        Expr::Index { object, index, .. } => {
            refs_expr(object, program, source, name, def_span, out);
            refs_expr(index, program, source, name, def_span, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            refs_expr(cond, program, source, name, def_span, out);
            refs_expr(then_branch, program, source, name, def_span, out);
            refs_expr(else_branch, program, source, name, def_span, out);
        }
        Expr::NullishCoalesce { left, right, .. } => {
            refs_expr(left, program, source, name, def_span, out);
            refs_expr(right, program, source, name, def_span, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                    tishlang_ast::ArrayElement::Spread(e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                }
            }
        }
        Expr::Assign { name: n, span, value } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
            refs_expr(value, program, source, name, def_span, out);
        }
        Expr::TypeOf { operand, .. } => refs_expr(operand, program, source, name, def_span, out),
        Expr::PostfixInc { name: n, span } | Expr::PostfixDec { name: n, span } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
        }
        Expr::PrefixInc { name: n, span } | Expr::PrefixDec { name: n, span } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
        }
        Expr::CompoundAssign { name: n, span, value, .. }
        | Expr::LogicalAssign { name: n, span, value, .. } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
            refs_expr(value, program, source, name, def_span, out);
        }
        Expr::MemberAssign { object, value, .. } => {
            refs_expr(object, program, source, name, def_span, out);
            refs_expr(value, program, source, name, def_span, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            refs_expr(object, program, source, name, def_span, out);
            refs_expr(index, program, source, name, def_span, out);
            refs_expr(value, program, source, name, def_span, out);
        }
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if let Some(e) = &tp.default {
                        refs_expr(e, program, source, name, def_span, out);
                    }
                } else if let FunParam::Destructure { default: Some(e), .. } = p {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
            match body {
                ArrowBody::Expr(e) => refs_expr(e, program, source, name, def_span, out),
                ArrowBody::Block(b) => refs_stmt(b, program, source, name, def_span, out),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                refs_expr(e, program, source, name, def_span, out);
            }
        }
        Expr::Await { operand, .. } => refs_expr(operand, program, source, name, def_span, out),
        Expr::JsxElement { props, children, .. } => {
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            refs_expr(e, program, source, name, def_span, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => refs_expr(e, program, source, name, def_span, out),
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
        }
        Expr::Literal { .. } | Expr::NativeModuleLoad { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_parser::parse;

    #[test]
    fn resolves_second_line_reference() {
        let src = "let a = 1\nlet b = a\n";
        let program = parse(src).expect("parse");
        let ds = definition_span(&program, src, 1, 8).expect("def");
        assert_eq!(ds.start.0, 1);
    }

    #[test]
    fn inner_scope_shadow() {
        let src = "let x = 1\n{\n  let x = 2\n  x\n}\n";
        let program = parse(src).expect("parse");
        let inner = definition_span(&program, src, 3, 2).expect("inner x def");
        assert_eq!(inner.start.0, 3);
    }

    #[test]
    fn completion_includes_fn_params() {
        let src = "fn f(foo, bar) {\n  bar\n}\n";
        let program = parse(src).expect("parse");
        let names = completion_value_names_at_cursor(&program, src, 1, 2);
        assert!(names.iter().any(|n| n.as_ref() == "foo"), "names={names:?}");
        assert!(names.iter().any(|n| n.as_ref() == "bar"));
    }

    #[test]
    fn unresolved_unknown_ident() {
        let src = "let x = nope\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name.as_ref(), "nope");
    }

    #[test]
    fn console_global_not_unresolved() {
        let src = "console.log(1)\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert!(u.is_empty(), "u={u:?}");
    }

    #[test]
    fn resolved_ident_not_unresolved() {
        let src = "let x = 1\nlet y = x\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert!(u.is_empty(), "u={u:?}");
    }

    #[test]
    fn unused_import_named() {
        let src = "import { a, b } from \"./m\"\nlet x = a\nx\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name.as_ref(), "b");
        assert_eq!(u[0].kind, UnusedBindingKind::Import);
    }

    #[test]
    fn unused_let_underscore_ignored() {
        let src = "let _x = 1\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "u={u:?}");
    }

    #[test]
    fn unused_local_let() {
        let src = "let dead = 1\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].name.as_ref(), "dead");
        assert_eq!(u[0].kind, UnusedBindingKind::Variable);
    }

    #[test]
    fn export_not_reported_unused() {
        let src = "export let x = 1\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "u={u:?}");
    }

    #[test]
    fn member_access_chain_three_deep() {
        let src = "let x = a.b.c\n";
        let program = parse(src).expect("parse");
        let ch = member_access_chain_at_cursor(&program, src, 0, 12).expect("chain");
        assert_eq!(ch.root_local.as_ref(), "a");
        assert_eq!(ch.members.len(), 2);
        assert_eq!(ch.members[0].as_ref(), "b");
        assert_eq!(ch.members[1].as_ref(), "c");
    }

    #[test]
    fn name_at_cursor_prefers_member_prop() {
        let src = "let x = a.b.c\n";
        let program = parse(src).expect("parse");
        let nu = name_at_cursor(&program, src, 0, 12).expect("name use");
        assert_eq!(nu.name.as_ref(), "c");
    }

    #[test]
    fn member_access_chain_across_lines() {
        let src = "fn main()\n  let x = a.b\n    .c\n";
        let program = parse(src).expect("parse");
        let ch = member_access_chain_at_cursor(&program, src, 2, 5).expect("chain");
        assert_eq!(ch.root_local.as_ref(), "a");
        assert_eq!(ch.members.len(), 2);
        assert_eq!(ch.members[0].as_ref(), "b");
        assert_eq!(ch.members[1].as_ref(), "c");
    }
}
