//! Lexical name resolution for Tish (go-to-definition, hover, references).
//!
//! Coordinates: LSP uses 0-based lines and UTF-16 columns; [`tishlang_ast::Span`] uses 1-based
//! lines and 1-based **Unicode scalar** columns from the lexer. Conversion goes through byte
//! offsets in the original source string.

mod pos;

pub use pos::{
    lsp_position_for_span_start, span_contains_lsp_position, span_to_lsp_range_exclusive,
};

use std::collections::HashMap;
use std::sync::Arc;

use tishlang_ast::{
    ArrowBody, CallArg, DestructElement, DestructPattern, ExportDeclaration, Expr, FunParam,
    ImportSpecifier, MemberProp, Program, Statement, TypedParam,
};

/// Smallest source span covering the LSP cursor (definition site or reference).
#[derive(Debug, Clone)]
pub struct NameUse {
    pub name: Arc<str>,
    pub span: tishlang_ast::Span,
}

/// Find the tightest name under the cursor (identifier reference or binding).
pub fn name_at_cursor(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_character: u32,
) -> Option<NameUse> {
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
    let nu = NameUse { name, span: *span };
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

thread_local! {
    /// When active, accumulates the definition spans that identifier uses resolve to during one
    /// `collect_unresolved_identifiers` scope-aware walk (`record_unresolved` already resolves every
    /// use). `collect_unused_bindings` activates it to learn which declarations are referenced in a
    /// single O(N) pass, instead of re-scanning the whole program per binding (which was ~O(N³) and
    /// froze large files). Inactive for normal callers (the unresolved-name diagnostic), which see
    /// no behavior change.
    static REFERENCED_DEFS: std::cell::RefCell<Option<std::collections::HashSet<(usize, usize)>>> =
        const { std::cell::RefCell::new(None) };

    /// Stack of definition-span starts of the functions whose bodies we are currently inside. A use
    /// that resolves to one of these is a SELF-reference (e.g. `fn a() { return a() }`) — recursion,
    /// not an external use — so it must not mark the binding "referenced" for the unused check, or a
    /// function reachable only via its own recursion would never be flagged dead (#150). Only
    /// consulted while `REFERENCED_DEFS` is active; find-references / go-to-definition (which want the
    /// recursive call) use a different path and are unaffected.
    static CURRENT_FN_DEFS: std::cell::RefCell<Vec<(usize, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// One scope-aware resolution pass: the set of definition span *starts* that at least one identifier
/// use resolves to (a name's declaration position is unique, so the start identifies it). Reuses the
/// exact walk and scope rules of [`collect_unresolved_identifiers`], so it stays consistent with the
/// unresolved-name diagnostic. O(N) (single walk; scope lookups O(1)).
fn collect_referenced_def_spans(program: &Program) -> std::collections::HashSet<(usize, usize)> {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            REFERENCED_DEFS.with(|r| *r.borrow_mut() = None);
            // Defensive: the FunDecl walk push/pops this in balanced pairs, but clear it here too so
            // a panic mid-walk can't leave a stale enclosing-fn span for the next collection.
            CURRENT_FN_DEFS.with(|s| s.borrow_mut().clear());
        }
    }
    let _g = Guard;
    REFERENCED_DEFS.with(|r| *r.borrow_mut() = Some(std::collections::HashSet::new()));
    // Side effect: record_unresolved inserts each resolved def span into REFERENCED_DEFS.
    let _ = collect_unresolved_identifiers(program);
    REFERENCED_DEFS.with(|r| r.borrow_mut().take().unwrap_or_default())
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
        Statement::Multi { statements, .. } => {
            for s in statements {
                collect_stmt(s, source, lsp_line, lsp_char, best);
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
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            for p in params {
                collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            if let Some(rp) = rest_param {
                consider(source, lsp_line, lsp_char, &rp.name_span, rp.name.clone(), best);
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
                        let local = alias
                            .as_ref()
                            .map(|a| a.clone())
                            .unwrap_or_else(|| name.clone());
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
            ExportDeclaration::Named(inner) => {
                collect_stmt(inner, source, lsp_line, lsp_char, best)
            }
            ExportDeclaration::Default(e) => collect_expr(e, source, lsp_line, lsp_char, best),
            ExportDeclaration::ReExport { .. } => {}
        },
        Statement::TypeAlias {
            name, name_span, ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
        }
        Statement::DeclareVar {
            name, name_span, ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            rest_param,
            ..
        } => {
            consider(source, lsp_line, lsp_char, name_span, name.clone(), best);
            for p in params {
                collect_fun_param(p, source, lsp_line, lsp_char, best);
            }
            if let Some(rp) = rest_param {
                consider(source, lsp_line, lsp_char, &rp.name_span, rp.name.clone(), best);
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
            for el in elements.iter().flatten() {
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
        Expr::Ident { name, span } => {
            consider(source, lsp_line, lsp_char, span, name.clone(), best)
        }
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => {
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
        Expr::Delete { target: del, .. } => collect_expr(del, source, lsp_line, lsp_char, best),
        Expr::PostfixInc { name, span } | Expr::PostfixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
        }
        Expr::PrefixInc { name, span } | Expr::PrefixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
        }
        Expr::CompoundAssign {
            name, span, value, ..
        }
        | Expr::LogicalAssign {
            name, span, value, ..
        } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            consider(source, lsp_line, lsp_char, &sp, name.clone(), best);
            collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::MemberAssign { object, value, .. } => {
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
        Expr::JsxElement {
            tag,
            tag_span,
            close_tag_span,
            props,
            children,
            ..
        } => {
            // A PascalCase tag references a component binding (`<Foo/>` lowers to `h(Foo, …)`); let
            // the cursor land on it (opening or closing tag) for hover / go-to-def / rename.
            if tag.chars().next().is_some_and(|c| c.is_uppercase()) {
                consider(source, lsp_line, lsp_char, tag_span, tag.clone(), best);
                if let Some(cs) = close_tag_span {
                    consider(source, lsp_line, lsp_char, cs, tag.clone(), best);
                }
            }
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                        collect_expr(e, source, lsp_line, lsp_char, best)
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
        Expr::Unary { operand, .. } => {
            member_chain_collect_expr(operand, source, lsp_line, lsp_char, best)
        }
        Expr::Call { callee, args, .. } => {
            member_chain_collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    CallArg::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::New { callee, args, .. } => {
            member_chain_collect_expr(callee, source, lsp_line, lsp_char, best);
            for a in args {
                match a {
                    CallArg::Expr(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    CallArg::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                    }
                }
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => {
            member_chain_collect_expr(value, source, lsp_line, lsp_char, best);
        }
        Expr::TypeOf { operand, .. } => {
            member_chain_collect_expr(operand, source, lsp_line, lsp_char, best)
        }
        Expr::Delete { target: del, .. } => {
            member_chain_collect_expr(del, source, lsp_line, lsp_char, best)
        }
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
                ArrowBody::Expr(e) => {
                    member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
                }
                ArrowBody::Block(b) => {
                    member_chain_collect_stmt(b, source, lsp_line, lsp_char, best)
                }
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best);
            }
        }
        Expr::Await { operand, .. } => {
            member_chain_collect_expr(operand, source, lsp_line, lsp_char, best)
        }
        Expr::JsxElement {
            props, children, ..
        } => {
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

#[allow(clippy::only_used_in_recursion)] // params threaded to recurse the pattern tree (mirrors sibling collectors)
fn member_chain_collect_destruct_pattern(
    pattern: &DestructPattern,
    source: &str,
    lsp_line: u32,
    lsp_char: u32,
    best: &mut Option<(u64, MemberAccessChain)>,
) {
    match pattern {
        DestructPattern::Array(elements) => {
            for el in elements.iter().flatten() {
                match el {
                    DestructElement::Ident(_, _) => {}
                    DestructElement::Pattern(inner) => member_chain_collect_destruct_pattern(
                        inner, source, lsp_line, lsp_char, best,
                    ),
                    DestructElement::Rest(_, _) => {}
                }
            }
        }
        DestructPattern::Object(props) => {
            for pr in props {
                match &pr.value {
                    DestructElement::Ident(_, _) => {}
                    DestructElement::Pattern(inner) => member_chain_collect_destruct_pattern(
                        inner, source, lsp_line, lsp_char, best,
                    ),
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
        Statement::ExprStmt { expr, .. } => {
            member_chain_collect_expr(expr, source, lsp_line, lsp_char, best)
        }
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
        Statement::Multi { statements, .. } => {
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
        Statement::Throw { value, .. } => {
            member_chain_collect_expr(value, source, lsp_line, lsp_char, best)
        }
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
            ExportDeclaration::Named(inner) => {
                member_chain_collect_stmt(inner, source, lsp_line, lsp_char, best)
            }
            ExportDeclaration::Default(e) => {
                member_chain_collect_expr(e, source, lsp_line, lsp_char, best)
            }
            ExportDeclaration::ReExport { .. } => {}
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
            for el in elements.iter().flatten() {
                match el {
                    DestructElement::Ident(n, sp) => scopes.define(n.as_ref(), *sp),
                    DestructElement::Pattern(inner) => define_pattern_stack(inner, scopes),
                    DestructElement::Rest(n, sp) => scopes.define(n.as_ref(), *sp),
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

/// Pre-define hoisted function-declaration names (incl. `export fn` and ambient `declare fn`) for a
/// statement list, so a reference to a function declared later in the same scope resolves. Function
/// values capture the live scope, so calling a sibling function declared further down succeeds at
/// runtime (e.g. mutual recursion, or a helper defined below its caller); the resolver mirrors that
/// by defining the names before walking any sibling body. Only function names are hoisted — `let`
/// is not, matching the interpreter.
fn hoist_fn_names(stmts: &[Statement], scopes: &mut ScopeStack) {
    for s in stmts {
        match s {
            Statement::FunDecl {
                name, name_span, ..
            }
            | Statement::DeclareFun {
                name, name_span, ..
            } => {
                scopes.define(name.as_ref(), *name_span);
            }
            Statement::Export { declaration, .. } => {
                if let ExportDeclaration::Named(inner) = declaration.as_ref() {
                    if let Statement::FunDecl {
                        name, name_span, ..
                    } = inner.as_ref()
                    {
                        scopes.define(name.as_ref(), *name_span);
                    }
                }
            }
            Statement::Multi { statements, .. } => hoist_fn_names(statements, scopes),
            _ => {}
        }
    }
}

/// The default-value expression of a parameter, if any (`fn f(a = EXPR)`). Both simple and
/// destructuring parameters can carry a default; rest parameters never do (no default syntax).
fn param_default(p: &FunParam) -> Option<&Expr> {
    match p {
        FunParam::Simple(tp) => tp.default.as_ref(),
        FunParam::Destructure { default, .. } => default.as_ref(),
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
        Expr::CompoundAssign {
            name, span, value, ..
        }
        | Expr::LogicalAssign {
            name, span, value, ..
        } => {
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
        Expr::New { callee, args, .. } => walk_expr_resolve(callee, scopes, target).or_else(|| {
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
        }),
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                if let Some(s) = walk_expr_resolve(e, scopes, target) {
                    return Some(s);
                }
            }
            None
        }
        Expr::TypeOf { operand, .. } => walk_expr_resolve(operand, scopes, target),
        Expr::Delete { target: del, .. } => walk_expr_resolve(del, scopes, target),
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
            for p in params {
                if let Some(e) = param_default(p) {
                    if let Some(s) = walk_expr_resolve(e, &inner, target) {
                        return Some(s);
                    }
                }
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
        Expr::JsxElement {
            tag,
            tag_span,
            close_tag_span,
            props,
            children,
            ..
        } => {
            // Go-to-def on a PascalCase component tag (opening or closing) resolves to its binding.
            if tag.chars().next().is_some_and(|c| c.is_uppercase())
                && (*tag_span == tgt || *close_tag_span == Some(tgt))
                && tag.as_ref() == target.name.as_ref()
            {
                return scopes.resolve(tag.as_ref());
            }
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
        Statement::VarDeclDestructure { pattern, init, .. } => {
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
        } => walk_expr_resolve(cond, scopes, target)
            .or_else(|| walk_stmt_implicit(then_branch, scopes, target))
            .or_else(|| {
                else_branch
                    .as_ref()
                    .and_then(|b| walk_stmt_implicit(b, scopes, target))
            }),
        Statement::While { cond, body, .. } => walk_expr_resolve(cond, scopes, target)
            .or_else(|| walk_stmt_implicit(body, scopes, target)),
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
        Statement::Return { value, .. } => value
            .as_ref()
            .and_then(|e| walk_expr_resolve(e, scopes, target)),
        Statement::Block { statements, .. } => {
            scopes.push();
            hoist_fn_names(statements, scopes);
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
        // Transparent group (comma-declarators): same scope, no push/pop.
        Statement::Multi { statements, .. } => {
            let mut out = None;
            for s in statements {
                if let Some(x) = walk_stmt_resolve(s, scopes, target) {
                    out = Some(x);
                    break;
                }
            }
            out
        }
        Statement::FunDecl {
            name,
            name_span,
            params,
            rest_param,
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
            if let Some(rp) = rest_param {
                scopes.define(rp.name.as_ref(), rp.name_span);
            }
            let mut r = None;
            for p in params {
                if let Some(e) = param_default(p) {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        r = Some(s);
                        break;
                    }
                }
            }
            if r.is_none() {
                r = walk_stmt_resolve(body, scopes, target);
            }
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
                        let local = alias
                            .as_ref()
                            .map(|a| a.clone())
                            .unwrap_or_else(|| name.clone());
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
            ExportDeclaration::ReExport { .. } => None,
        },
        Statement::TypeAlias {
            name, name_span, ..
        } => {
            if *name_span == tgt_span && name.as_ref() == target.name.as_ref() {
                return Some(*name_span);
            }
            scopes.define(name.as_ref(), *name_span);
            None
        }
        Statement::DeclareVar {
            name, name_span, ..
        } => {
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
            rest_param,
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
            if let Some(rp) = rest_param {
                scopes.define(rp.name.as_ref(), rp.name_span);
            }
            let mut r = None;
            for p in params {
                if let Some(e) = param_default(p) {
                    if let Some(s) = walk_expr_resolve(e, scopes, target) {
                        r = Some(s);
                        break;
                    }
                }
            }
            scopes.pop();
            r
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
///
/// NOTE: maintained by hand to track the globals the interpreter registers on its root scope
/// (`tish_eval`). It is the only suppression the LSP has — the resolver does not load
/// `builtins.d.tish` — so any global missing here surfaces as a false `tish-unresolved-name`
/// error in the editor. Keep it in sync (see the `runtime_globals_not_unresolved` test).
pub fn is_runtime_global_ident(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "parseInt"
            | "parseFloat"
            | "decodeURI"
            | "encodeURI"
            | "Boolean"
            | "Number"
            | "isFinite"
            | "isNaN"
            | "Infinity"
            | "NaN"
            | "Math"
            | "JSON"
            | "Object"
            | "Array"
            | "String"
            | "Symbol"
            | "Date"
            | "Set"
            | "Map"
            | "Float64Array"
            | "Float32Array"
            | "Int8Array"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "AudioContext"
            | "RegExp"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "Promise"
            | "fetch"
            | "fetchAll"
            | "serve"
            | "htmlEscape"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
    )
}

fn record_unresolved(
    scopes: &ScopeStack,
    name: &Arc<str>,
    span: tishlang_ast::Span,
    out: &mut Vec<UnresolvedIdentifier>,
) {
    match scopes.resolve(name.as_ref()) {
        // Resolved to an in-scope declaration. Record it (when collection is active) so
        // collect_unused_bindings learns this def is referenced — done before the global check so a
        // local that shadows a global name still counts as a use. Unresolved-name output unchanged.
        Some(def) => {
            // A use that resolves to a function we're currently inside is a self-recursive call,
            // not an external use — skip it so a function reachable only via its own recursion is
            // still reported unused (#150).
            let is_self_ref = CURRENT_FN_DEFS.with(|s| s.borrow().contains(&def.start));
            if !is_self_ref {
                REFERENCED_DEFS.with(|r| {
                    if let Some(set) = r.borrow_mut().as_mut() {
                        set.insert(def.start);
                    }
                });
            }
        }
        None => {
            if !is_runtime_global_ident(name.as_ref()) {
                out.push(UnresolvedIdentifier {
                    name: name.clone(),
                    span,
                });
            }
        }
    }
}

/// Like [`record_unresolved`] for an assignment TARGET: still flags an unresolved name (writing to
/// an undeclared variable is an error), but does NOT mark a resolved def as *referenced*. A pure
/// write — and, like ESLint's `no-unused-vars`, a read-modify-write (`+=`/`++`/`||=`) — is not a
/// "use", so a binding that is only ever written gets reported as unused. Reads elsewhere (incl. the
/// RHS, e.g. `n = n + 1`) still go through `record_unresolved` and keep the binding live. (#149)
fn record_write(
    scopes: &ScopeStack,
    name: &Arc<str>,
    span: tishlang_ast::Span,
    out: &mut Vec<UnresolvedIdentifier>,
) {
    if scopes.resolve(name.as_ref()).is_none() && !is_runtime_global_ident(name.as_ref()) {
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
            // Target is a WRITE (not a read) for unused detection; the RHS is read normally.
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_write(scopes, name, sp, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::CompoundAssign {
            name, span, value, ..
        }
        | Expr::LogicalAssign {
            name, span, value, ..
        } => {
            // Read-modify-write: ESLint counts the target as a write, not a use. RHS is read.
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_write(scopes, name, sp, out);
            check_unresolved_expr(value, scopes, out);
        }
        Expr::PostfixInc { name, span } | Expr::PostfixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_write(scopes, name, sp, out);
        }
        Expr::PrefixInc { name, span } | Expr::PrefixDec { name, span } => {
            let sp = synthetic_name_span(span.start, name.as_ref());
            record_write(scopes, name, sp, out);
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => e,
                    tishlang_ast::ObjectProp::Spread(e) => e,
                };
                check_unresolved_expr(e, scopes, out);
            }
        }
        Expr::TypeOf { operand, .. } => check_unresolved_expr(operand, scopes, out),
        Expr::Delete { target: del, .. } => check_unresolved_expr(del, scopes, out),
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
            for p in params {
                if let Some(e) = param_default(p) {
                    check_unresolved_expr(e, &inner, out);
                }
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
        Expr::JsxElement {
            props, children, ..
        } => {
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

fn check_stmt_implicit_unresolved(
    stmt: &Statement,
    scopes: &mut ScopeStack,
    out: &mut Vec<UnresolvedIdentifier>,
) {
    if matches!(stmt, Statement::Block { .. }) {
        check_unresolved_stmt(stmt, scopes, out);
    } else {
        scopes.push();
        check_unresolved_stmt(stmt, scopes, out);
        scopes.pop();
    }
}

fn check_unresolved_stmt(
    stmt: &Statement,
    scopes: &mut ScopeStack,
    out: &mut Vec<UnresolvedIdentifier>,
) {
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
            hoist_fn_names(statements, scopes);
            for s in statements {
                check_unresolved_stmt(s, scopes, out);
            }
            scopes.pop();
        }
        // Transparent group (comma-declarators): same scope, no push/pop.
        Statement::Multi { statements, .. } => {
            for s in statements {
                check_unresolved_stmt(s, scopes, out);
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
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            if let Some(rp) = rest_param {
                scopes.define(rp.name.as_ref(), rp.name_span);
            }
            for p in params {
                if let Some(e) = param_default(p) {
                    check_unresolved_expr(e, scopes, out);
                }
            }
            // Mark this function as "currently inside" so a recursive self-call in the body isn't
            // counted as an external use for the unused-binding check (#150).
            CURRENT_FN_DEFS.with(|s| s.borrow_mut().push(name_span.start));
            check_unresolved_stmt(body, scopes, out);
            CURRENT_FN_DEFS.with(|s| {
                s.borrow_mut().pop();
            });
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
                        let local = alias
                            .as_ref()
                            .map(|a| a.clone())
                            .unwrap_or_else(|| name.clone());
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
            ExportDeclaration::ReExport { .. } => {}
        },
        Statement::TypeAlias {
            name, name_span, ..
        } => {
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::DeclareVar {
            name, name_span, ..
        } => {
            scopes.define(name.as_ref(), *name_span);
        }
        Statement::DeclareFun {
            name,
            name_span,
            params,
            rest_param,
            ..
        } => {
            scopes.push();
            scopes.define(name.as_ref(), *name_span);
            for p in params {
                define_fun_param_stack(p, scopes);
            }
            if let Some(rp) = rest_param {
                scopes.define(rp.name.as_ref(), rp.name_span);
            }
            for p in params {
                if let Some(e) = param_default(p) {
                    check_unresolved_expr(e, scopes, out);
                }
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
    hoist_fn_names(&program.statements, &mut scopes);
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
            for el in elements.iter().flatten() {
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
            pattern, default, ..
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => e,
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
        Expr::Delete { target: del, .. } => enumerate_expr(del, exported, out),
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
        Expr::JsxElement {
            props, children, ..
        } => {
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
                        let local = alias
                            .as_ref()
                            .map(|a| a.clone())
                            .unwrap_or_else(|| name.clone());
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
            ExportDeclaration::ReExport { .. } => {}
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
        Statement::Multi { statements, .. } => {
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
        Statement::TypeAlias {
            name, name_span, ..
        } => {
            out.push(BindingSite {
                name: name.clone(),
                span: *name_span,
                kind: UnusedBindingKind::Variable,
                exported,
            });
        }
        // Ambient `declare` declarations describe symbols defined elsewhere and have no body, so
        // nothing here is ever "unused" within this file (params can't be read; the name is API
        // surface for other modules). They contribute no unused-binding candidates.
        Statement::DeclareVar { .. } | Statement::DeclareFun { .. } => {}
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Collect PascalCase JSX component tag names used anywhere in `program`. A `<Foo/>` tag lowers to
/// `h(Foo, …)` — a real reference to the `Foo` binding — but the tag is not a span-validated
/// identifier use, so the reference walker misses it. Used by [`collect_unused_bindings`] to avoid
/// flagging a component import/binding used only as a tag. (Lowercase tags lower to string
/// literals, so they are not references.) Precise rename/find-references for tags still needs a
/// real tag span in the AST and is handled separately.
fn collect_jsx_component_tags(program: &Program) -> std::collections::HashSet<Arc<str>> {
    let mut out = std::collections::HashSet::new();
    for s in &program.statements {
        jsx_tags_stmt(s, &mut out);
    }
    out
}

fn jsx_tags_stmt(s: &Statement, out: &mut std::collections::HashSet<Arc<str>>) {
    match s {
        Statement::VarDecl { init, .. } => {
            if let Some(e) = init {
                jsx_tags_expr(e, out);
            }
        }
        Statement::VarDeclDestructure { init, .. } => jsx_tags_expr(init, out),
        Statement::ExprStmt { expr, .. } => jsx_tags_expr(expr, out),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            jsx_tags_expr(cond, out);
            jsx_tags_stmt(then_branch, out);
            if let Some(b) = else_branch {
                jsx_tags_stmt(b, out);
            }
        }
        Statement::While { cond, body, .. } => {
            jsx_tags_expr(cond, out);
            jsx_tags_stmt(body, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                jsx_tags_stmt(i, out);
            }
            if let Some(e) = cond {
                jsx_tags_expr(e, out);
            }
            if let Some(e) = update {
                jsx_tags_expr(e, out);
            }
            jsx_tags_stmt(body, out);
        }
        Statement::ForOf { iterable, body, .. } => {
            jsx_tags_expr(iterable, out);
            jsx_tags_stmt(body, out);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                jsx_tags_expr(e, out);
            }
        }
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            for s in statements {
                jsx_tags_stmt(s, out);
            }
        }
        Statement::FunDecl { params, body, .. } => {
            for p in params {
                if let Some(e) = param_default(p) {
                    jsx_tags_expr(e, out);
                }
            }
            jsx_tags_stmt(body, out);
        }
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            jsx_tags_expr(expr, out);
            for (_, stmts) in cases {
                for s in stmts {
                    jsx_tags_stmt(s, out);
                }
            }
            if let Some(stmts) = default_body {
                for s in stmts {
                    jsx_tags_stmt(s, out);
                }
            }
        }
        Statement::DoWhile { body, cond, .. } => {
            jsx_tags_stmt(body, out);
            jsx_tags_expr(cond, out);
        }
        Statement::Throw { value, .. } => jsx_tags_expr(value, out),
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            jsx_tags_stmt(body, out);
            if let Some(cb) = catch_body {
                jsx_tags_stmt(cb, out);
            }
            if let Some(fb) = finally_body {
                jsx_tags_stmt(fb, out);
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => jsx_tags_stmt(inner, out),
            ExportDeclaration::Default(e) => jsx_tags_expr(e, out),
            ExportDeclaration::ReExport { .. } => {}
        },
        Statement::Import { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. }
        | Statement::DeclareFun { .. } => {}
    }
}

fn jsx_tags_expr(e: &Expr, out: &mut std::collections::HashSet<Arc<str>>) {
    match e {
        Expr::JsxElement {
            tag,
            props,
            children,
            ..
        } => {
            if tag.chars().next().is_some_and(|c| c.is_uppercase()) {
                out.insert(tag.clone());
            }
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            jsx_tags_expr(e, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => jsx_tags_expr(e, out),
                }
            }
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    jsx_tags_expr(e, out);
                }
            }
        }
        Expr::JsxFragment { children, .. } => {
            for ch in children {
                if let tishlang_ast::JsxChild::Expr(e) = ch {
                    jsx_tags_expr(e, out);
                }
            }
        }
        Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
            jsx_tags_expr(left, out);
            jsx_tags_expr(right, out);
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf { operand, .. }
        | Expr::Await { operand, .. } => jsx_tags_expr(operand, out),
        Expr::Delete { target, .. } => jsx_tags_expr(target, out),
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            jsx_tags_expr(callee, out);
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => jsx_tags_expr(e, out),
                }
            }
        }
        Expr::Member { object, .. } => jsx_tags_expr(object, out),
        Expr::Index { object, index, .. } => {
            jsx_tags_expr(object, out);
            jsx_tags_expr(index, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            jsx_tags_expr(cond, out);
            jsx_tags_expr(then_branch, out);
            jsx_tags_expr(else_branch, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(e)
                    | tishlang_ast::ArrayElement::Spread(e) => jsx_tags_expr(e, out),
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e, _)
                    | tishlang_ast::ObjectProp::Spread(e) => jsx_tags_expr(e, out),
                }
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => jsx_tags_expr(value, out),
        Expr::MemberAssign { object, value, .. } => {
            jsx_tags_expr(object, out);
            jsx_tags_expr(value, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            jsx_tags_expr(object, out);
            jsx_tags_expr(index, out);
            jsx_tags_expr(value, out);
        }
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                if let Some(e) = param_default(p) {
                    jsx_tags_expr(e, out);
                }
            }
            match body {
                ArrowBody::Expr(e) => jsx_tags_expr(e, out),
                ArrowBody::Block(b) => jsx_tags_stmt(b, out),
            }
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                jsx_tags_expr(e, out);
            }
        }
        Expr::Ident { .. }
        | Expr::Literal { .. }
        | Expr::NativeModuleLoad { .. }
        | Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. } => {}
    }
}

/// Names referenced in a TYPE position — every `Simple(name)` use in a type annotation (param /
/// var / return types, alias bodies, nested object/array/union/fn types). A binding (a type-only
/// import or a type alias) used *only* as a type would otherwise be reported unused (#139), since
/// the value-resolution walk never sees annotations. Name-based and intentionally conservative,
/// mirroring the JSX-tag set.
fn type_reference_names(program: &Program) -> std::collections::HashSet<Arc<str>> {
    let mut occ: Vec<TypeOcc> = Vec::new();
    for s in &program.statements {
        tref_stmt(s, &mut occ);
    }
    occ.into_iter().filter(|o| !o.is_decl).map(|o| o.name).collect()
}

/// Declarations whose values are never read (imports, locals, parameters). Skips `exported` module
/// bindings and names starting with `_` (common intentional-unused convention).
pub fn collect_unused_bindings(program: &Program, _source: &str) -> Vec<UnusedBinding> {
    let mut sites = Vec::new();
    for s in &program.statements {
        enumerate_stmt(s, false, &mut sites);
    }
    // A binding used only as a PascalCase JSX component tag (`<Foo/>`) is genuinely used — the
    // resolution walk can't see the tag, so consult the tag set to avoid a false "unused" report
    // (deleting such an import would break the build).
    let jsx_tags = collect_jsx_component_tags(program);
    // A binding referenced only in a type annotation (`: Foo`) is used; the value walk can't see
    // type positions, so consult the type-reference names to avoid a false "unused" report (#139).
    let type_refs = type_reference_names(program);
    // Which declarations are referenced, learned in ONE scope-aware pass. The previous approach
    // re-scanned the whole program per binding (re-resolving each use), which was ~O(N³) and froze
    // large files; this is O(N). A binding is unused iff nothing resolves to its definition span.
    let referenced = collect_referenced_def_spans(program);
    let mut out = Vec::new();
    for site in sites {
        if site.exported || site.name.as_ref().starts_with('_') {
            continue;
        }
        if jsx_tags.contains(&site.name) {
            continue;
        }
        if type_refs.contains(&site.name) {
            continue;
        }
        if !referenced.contains(&site.span.start) {
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
    hoist_fn_names(&program.statements, &mut scopes);
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
                            name, name_span, ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::FunDecl {
                            name, name_span, ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::TypeAlias {
                            name, name_span, ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::DeclareVar {
                            name, name_span, ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::DeclareFun {
                            name, name_span, ..
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
                && pos::span_contains_lsp_position(
                    source,
                    &body.as_ref().span(),
                    lsp_line,
                    lsp_char,
                )
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
        Statement::DoWhile { body, .. } => {
            collect_block_locals(body, source, lsp_line, lsp_char, out)
        }
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
                if pos::span_contains_lsp_position(source, &cb.as_ref().span(), lsp_line, lsp_char)
                {
                    out.push((n.clone(), *ps));
                }
                collect_block_locals(cb, source, lsp_line, lsp_char, out);
            }
            if let Some(fb) = finally_body {
                collect_block_locals(fb, source, lsp_line, lsp_char, out);
            }
        }
        Statement::Switch {
            cases,
            default_body,
            ..
        } => {
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
            for el in elements.iter().flatten() {
                match el {
                    DestructElement::Ident(n, _) => out.push(n.clone()),
                    DestructElement::Pattern(inner) => {
                        collect_pattern_binding_names(inner, out)
                    }
                    DestructElement::Rest(n, _) => out.push(n.clone()),
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

fn record_callable_stack(stack: &[Vec<Arc<str>>], best: &mut Option<(usize, Vec<Arc<str>>)>) {
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
                    FunParam::Destructure {
                        default: Some(e), ..
                    } => {
                        walk_expr_completion(e, source, lsp_line, lsp_char, stack, best);
                    }
                    _ => {}
                }
            }
            match body {
                ArrowBody::Expr(e) => {
                    walk_expr_completion(e, source, lsp_line, lsp_char, stack, best)
                }
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
        Expr::Unary { operand, .. } => {
            walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best)
        }
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
        Expr::Member { object, .. } => {
            walk_expr_completion(object, source, lsp_line, lsp_char, stack, best)
        }
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => e,
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
        Expr::TypeOf { operand, .. } => {
            walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best)
        }
        Expr::Delete { target: del, .. } => {
            walk_expr_completion(del, source, lsp_line, lsp_char, stack, best)
        }
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
        Expr::Await { operand, .. } => {
            walk_expr_completion(operand, source, lsp_line, lsp_char, stack, best)
        }
        Expr::JsxElement {
            props, children, ..
        } => {
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
        Statement::Multi { statements, .. } => {
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
                    FunParam::Destructure {
                        default: Some(e), ..
                    } => {
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
            ExportDeclaration::ReExport { .. } => {}
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
                name, name_span, ..
            } => out.push((name.clone(), *name_span)),
            Statement::FunDecl {
                name, name_span, ..
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
                            let local = alias
                                .as_ref()
                                .map(|a| a.clone())
                                .unwrap_or_else(|| name.clone());
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
                            name, name_span, ..
                        } => out.push((name.clone(), *name_span)),
                        Statement::FunDecl {
                            name, name_span, ..
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
// ---- Type-alias rename (#131) ---------------------------------------------------------------
// Type names live in a namespace separate from value bindings, so the value resolver
// (`definition_span` / `reference_spans_for_def`) never sees `: T` annotation uses. These walkers
// gather every type-name occurrence — the alias declaration plus each span-carrying
// `TypeAnnotation::Simple` reference in any annotation — so renaming a type alias edits the
// declaration and all its uses together (previously only the declaration was edited, silently
// breaking the program).

struct TypeOcc {
    name: Arc<str>,
    span: tishlang_ast::Span,
    is_decl: bool,
}

fn tref_ann(ann: &tishlang_ast::TypeAnnotation, out: &mut Vec<TypeOcc>) {
    use tishlang_ast::TypeAnnotation as T;
    match ann {
        T::Simple(name, span) => out.push(TypeOcc {
            name: name.clone(),
            span: *span,
            is_decl: false,
        }),
        T::Array(inner) => tref_ann(inner, out),
        T::Object(fields) => {
            for (_, t) in fields {
                tref_ann(t, out);
            }
        }
        T::Function { params, returns } => {
            for p in params {
                tref_ann(p, out);
            }
            tref_ann(returns, out);
        }
        T::Union(ts) | T::Tuple(ts) | T::Intersection(ts) => {
            for t in ts {
                tref_ann(t, out);
            }
        }
        T::Literal(_) => {}
    }
}

fn tref_typed_param(tp: &TypedParam, out: &mut Vec<TypeOcc>) {
    if let Some(t) = &tp.type_ann {
        tref_ann(t, out);
    }
    if let Some(d) = &tp.default {
        tref_expr(d, out);
    }
}

fn tref_param(p: &FunParam, out: &mut Vec<TypeOcc>) {
    match p {
        FunParam::Simple(tp) => tref_typed_param(tp, out),
        FunParam::Destructure {
            type_ann, default, ..
        } => {
            if let Some(t) = type_ann {
                tref_ann(t, out);
            }
            if let Some(d) = default {
                tref_expr(d, out);
            }
        }
    }
}

fn tref_arrow_body(body: &ArrowBody, out: &mut Vec<TypeOcc>) {
    match body {
        ArrowBody::Expr(e) => tref_expr(e, out),
        ArrowBody::Block(s) => tref_stmt(s, out),
    }
}

fn tref_expr(expr: &Expr, out: &mut Vec<TypeOcc>) {
    match expr {
        Expr::ArrowFunction { params, body, .. } => {
            for p in params {
                tref_param(p, out);
            }
            tref_arrow_body(body, out);
        }
        Expr::Binary { left, right, .. } | Expr::NullishCoalesce { left, right, .. } => {
            tref_expr(left, out);
            tref_expr(right, out);
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf { operand, .. }
        | Expr::Await { operand, .. } => tref_expr(operand, out),
        Expr::Delete { target, .. } => tref_expr(target, out),
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            tref_expr(callee, out);
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => tref_expr(e, out),
                }
            }
        }
        Expr::Member { object, .. } => tref_expr(object, out),
        Expr::Index { object, index, .. } => {
            tref_expr(object, out);
            tref_expr(index, out);
        }
        Expr::Conditional {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            tref_expr(cond, out);
            tref_expr(then_branch, out);
            tref_expr(else_branch, out);
        }
        Expr::Array { elements, .. } => {
            for el in elements {
                match el {
                    tishlang_ast::ArrayElement::Expr(e)
                    | tishlang_ast::ArrayElement::Spread(e) => tref_expr(e, out),
                }
            }
        }
        Expr::Object { props, .. } => {
            for p in props {
                match p {
                    tishlang_ast::ObjectProp::KeyValue(_, e, _)
                    | tishlang_ast::ObjectProp::Spread(e) => tref_expr(e, out),
                }
            }
        }
        Expr::Assign { value, .. }
        | Expr::CompoundAssign { value, .. }
        | Expr::LogicalAssign { value, .. } => tref_expr(value, out),
        Expr::MemberAssign { object, value, .. } => {
            tref_expr(object, out);
            tref_expr(value, out);
        }
        Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            tref_expr(object, out);
            tref_expr(index, out);
            tref_expr(value, out);
        }
        Expr::TemplateLiteral { exprs, .. } => {
            for e in exprs {
                tref_expr(e, out);
            }
        }
        _ => {}
    }
}

fn tref_stmt(stmt: &Statement, out: &mut Vec<TypeOcc>) {
    match stmt {
        Statement::TypeAlias {
            name,
            name_span,
            ty,
            ..
        } => {
            out.push(TypeOcc {
                name: name.clone(),
                span: *name_span,
                is_decl: true,
            });
            tref_ann(ty, out);
        }
        Statement::VarDecl { type_ann, init, .. } => {
            if let Some(t) = type_ann {
                tref_ann(t, out);
            }
            if let Some(e) = init {
                tref_expr(e, out);
            }
        }
        Statement::VarDeclDestructure { init, .. } => tref_expr(init, out),
        Statement::DeclareVar { type_ann, .. } => {
            if let Some(t) = type_ann {
                tref_ann(t, out);
            }
        }
        Statement::FunDecl {
            params,
            rest_param,
            return_type,
            body,
            ..
        } => {
            for p in params {
                tref_param(p, out);
            }
            if let Some(rp) = rest_param {
                tref_typed_param(rp, out);
            }
            if let Some(t) = return_type {
                tref_ann(t, out);
            }
            tref_stmt(body, out);
        }
        Statement::DeclareFun {
            params,
            rest_param,
            return_type,
            ..
        } => {
            for p in params {
                tref_param(p, out);
            }
            if let Some(rp) = rest_param {
                tref_typed_param(rp, out);
            }
            if let Some(t) = return_type {
                tref_ann(t, out);
            }
        }
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            for s in statements {
                tref_stmt(s, out);
            }
        }
        Statement::ExprStmt { expr, .. } => tref_expr(expr, out),
        Statement::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            tref_expr(cond, out);
            tref_stmt(then_branch, out);
            if let Some(e) = else_branch {
                tref_stmt(e, out);
            }
        }
        Statement::While { cond, body, .. } => {
            tref_expr(cond, out);
            tref_stmt(body, out);
        }
        Statement::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                tref_stmt(i, out);
            }
            if let Some(c) = cond {
                tref_expr(c, out);
            }
            if let Some(u) = update {
                tref_expr(u, out);
            }
            tref_stmt(body, out);
        }
        Statement::ForOf { iterable, body, .. } => {
            tref_expr(iterable, out);
            tref_stmt(body, out);
        }
        Statement::DoWhile { body, cond, .. } => {
            tref_stmt(body, out);
            tref_expr(cond, out);
        }
        Statement::Return { value, .. } => {
            if let Some(e) = value {
                tref_expr(e, out);
            }
        }
        Statement::Throw { value, .. } => tref_expr(value, out),
        Statement::Switch {
            expr,
            cases,
            default_body,
            ..
        } => {
            tref_expr(expr, out);
            for (c, body) in cases {
                if let Some(c) = c {
                    tref_expr(c, out);
                }
                for s in body {
                    tref_stmt(s, out);
                }
            }
            if let Some(body) = default_body {
                for s in body {
                    tref_stmt(s, out);
                }
            }
        }
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
            tref_stmt(body, out);
            if let Some(c) = catch_body {
                tref_stmt(c, out);
            }
            if let Some(f) = finally_body {
                tref_stmt(f, out);
            }
        }
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => tref_stmt(inner, out),
            ExportDeclaration::Default(e) => tref_expr(e, out),
            ExportDeclaration::ReExport { .. } => {}
        },
        _ => {}
    }
}

/// If the cursor sits on a type-alias declaration name or on a `: T` type-reference use, returns
/// every source span that must be renamed together (the declaration plus all annotation uses).
/// Returns `None` when the cursor is not on a user-declared type alias, so callers fall back to
/// value-binding rename. Fixes the silent breakage where renaming a `type T` left every `: T`
/// annotation stale (type names are in a namespace the value resolver does not see).
pub fn type_alias_rename_spans(
    program: &Program,
    source: &str,
    lsp_line: u32,
    lsp_character: u32,
) -> Option<Vec<tishlang_ast::Span>> {
    let mut occ: Vec<TypeOcc> = Vec::new();
    for s in &program.statements {
        tref_stmt(s, &mut occ);
    }
    // The type name whose declaration or use span contains the cursor.
    let target = occ
        .iter()
        .find(|o| pos::span_contains_lsp_position(source, &o.span, lsp_line, lsp_character))?
        .name
        .clone();
    // Only rename user-declared aliases — never a builtin (`number`, `string`, …) used in an
    // annotation, which has no declaration to keep in sync.
    if !occ.iter().any(|o| o.is_decl && o.name == target) {
        return None;
    }
    let mut spans: Vec<tishlang_ast::Span> = occ
        .iter()
        .filter(|o| o.name == target)
        .map(|o| o.span)
        .collect();
    spans.sort_by_key(|s| (s.start.0, s.start.1, s.end.0, s.end.1));
    spans.dedup();
    Some(spans)
}

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
        Statement::VarDeclDestructure { init, .. } => {
            refs_expr(init, program, source, name, def_span, out)
        }
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
        Statement::ForOf { iterable, body, .. } => {
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
        Statement::Multi { statements, .. } => {
            for s in statements {
                refs_stmt(s, program, source, name, def_span, out);
            }
        }
        Statement::FunDecl { params, body, .. } => {
            for p in params {
                if let Some(e) = param_default(p) {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
            refs_stmt(body, program, source, name, def_span, out);
        }
        // Ambient `declare fn` params can carry value-level defaults too.
        Statement::DeclareFun { params, .. } => {
            for p in params {
                if let Some(e) = param_default(p) {
                    refs_expr(e, program, source, name, def_span, out);
                }
            }
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
        // Descend into exported declarations so references inside `export fn`/`export let`/
        // `export default` bodies are counted (otherwise a binding used only there looks unused).
        Statement::Export { declaration, .. } => match declaration.as_ref() {
            ExportDeclaration::Named(inner) => {
                refs_stmt(inner, program, source, name, def_span, out)
            }
            ExportDeclaration::Default(e) => refs_expr(e, program, source, name, def_span, out),
            ExportDeclaration::ReExport { .. } => {}
        },
        Statement::Import { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. }
        | Statement::TypeAlias { .. }
        | Statement::DeclareVar { .. } => {}
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
    let maybe_push =
        |span: &tishlang_ast::Span, n: &Arc<str>, out: &mut Vec<tishlang_ast::Span>| {
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
                    tishlang_ast::ObjectProp::KeyValue(_, e, _) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                    tishlang_ast::ObjectProp::Spread(e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
                }
            }
        }
        Expr::Assign {
            name: n,
            span,
            value,
        } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
            refs_expr(value, program, source, name, def_span, out);
        }
        Expr::TypeOf { operand, .. } => refs_expr(operand, program, source, name, def_span, out),
        Expr::Delete { target: del, .. } => refs_expr(del, program, source, name, def_span, out),
        Expr::PostfixInc { name: n, span } | Expr::PostfixDec { name: n, span } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
        }
        Expr::PrefixInc { name: n, span } | Expr::PrefixDec { name: n, span } => {
            let sp = synthetic_name_span(span.start, n.as_ref());
            maybe_push(&sp, n, out);
        }
        Expr::CompoundAssign {
            name: n,
            span,
            value,
            ..
        }
        | Expr::LogicalAssign {
            name: n,
            span,
            value,
            ..
        } => {
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
                } else if let FunParam::Destructure {
                    default: Some(e), ..
                } = p
                {
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
        Expr::JsxElement {
            tag,
            tag_span,
            close_tag_span,
            props,
            children,
            ..
        } => {
            // A PascalCase tag is a reference to its component binding. Count BOTH the opening and
            // closing tag so find-references lists them and rename rewrites both (renaming only the
            // open tag would leave `<Bar></Foo>`). maybe_push validates each via definition_span.
            if tag.chars().next().is_some_and(|c| c.is_uppercase()) {
                maybe_push(tag_span, tag, out);
                if let Some(cs) = close_tag_span {
                    maybe_push(cs, tag, out);
                }
            }
            for p in props {
                match p {
                    tishlang_ast::JsxProp::Attr { value, .. } => {
                        if let tishlang_ast::JsxAttrValue::Expr(e) = value {
                            refs_expr(e, program, source, name, def_span, out);
                        }
                    }
                    tishlang_ast::JsxProp::Spread(e) => {
                        refs_expr(e, program, source, name, def_span, out)
                    }
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

    // #158: on the line AFTER a function's closing brace, completion must not leak the function's
    // params or inner locals (the body Block span used to overrun onto that line). The top-level
    // function name is still in scope.
    #[test]
    fn completion_does_not_leak_past_function_brace() {
        let src = "fn f(paramA, paramB) {\n  let secret = 1\n}\nlet after = 0\n";
        let program = parse(src).expect("parse");
        let names = completion_value_names_at_cursor(&program, src, 3, 0); // start of `let after`
        let has = |n: &str| names.iter().any(|x| x.as_ref() == n);
        assert!(!has("paramA") && !has("paramB"), "params leaked past `}}`: {names:?}");
        assert!(!has("secret"), "inner local leaked past `}}`: {names:?}");
        assert!(has("f"), "top-level fn must stay in scope: {names:?}");
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
    fn import_used_in_exported_fn_body_not_unused() {
        // Regression: a binding used only inside an `export fn` body was wrongly flagged unused
        // because refs_stmt did not descend into Statement::Export.
        let src = "import { publicUser } from \"./m\"\nexport fn toUserPublic(store, u) {\n  let pub = publicUser(u)\n  return pub\n}\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "publicUser is used in the exported fn; u={u:?}");
    }

    #[test]
    fn import_used_in_export_default_not_unused() {
        let src = "import { p } from \"./m\"\nexport default p(1)\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "p is used in export default; u={u:?}");
    }

    #[test]
    fn import_used_in_exported_let_init_not_unused() {
        let src = "import { p } from \"./m\"\nexport let x = p(1)\nx\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "p is used in the exported let init; u={u:?}");
    }

    #[test]
    fn genuinely_unused_import_still_flagged_with_export_present() {
        // Guard against the fix over-counting: a truly-unused import is still reported even when an
        // exported declaration is present in the module.
        let src = "import { used, dead } from \"./m\"\nexport fn f() {\n  return used()\n}\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert_eq!(u.len(), 1, "only `dead` is unused; u={u:?}");
        assert_eq!(u[0].name.as_ref(), "dead");
        assert_eq!(u[0].kind, UnusedBindingKind::Import);
    }

    // #149: a binding that is only ever WRITTEN (never read) is unused (matches ESLint).
    #[test]
    fn write_only_variables_are_flagged_unused() {
        for (src, who) in [
            ("let n = 0\nn += 5\n", "n"),     // read-modify-write
            ("let w = 0\nw = 7\n", "w"),       // plain write
            ("let p = 0\np++\n", "p"),         // postfix inc
            ("let q = 0\n++q\n", "q"),         // prefix inc
        ] {
            let program = parse(src).expect("parse");
            let u = collect_unused_bindings(&program, src);
            assert!(
                u.iter().any(|b| b.name.as_ref() == who
                    && b.kind == UnusedBindingKind::Variable),
                "{who} is write-only and should be flagged unused; src={src:?} u={u:?}"
            );
        }
    }

    #[test]
    fn variables_read_anywhere_are_not_flagged() {
        // A read on the RHS of its own assignment, or a bare use, keeps the binding live.
        for src in [
            "let count = 0\ncount = count + 1\ncount\n", // read in RHS + bare use
            "let acc = 0\nacc += 1\nacc\n",              // compound write + later read
            "let x = 1\nx\n",                            // plain read (control)
        ] {
            let program = parse(src).expect("parse");
            let u = collect_unused_bindings(&program, src);
            assert!(u.is_empty(), "no unused expected; src={src:?} u={u:?}");
        }
    }

    // #150: a function reachable only via its own recursion is dead code and should be flagged.
    #[test]
    fn self_recursive_only_function_is_flagged_unused() {
        for src in ["fn a() { return a() }\n", "fn loop_(n) { return loop_(n - 1) }\n"] {
            let program = parse(src).expect("parse");
            let u = collect_unused_bindings(&program, src);
            assert_eq!(u.len(), 1, "self-recursive-only fn is unused; src={src:?} u={u:?}");
        }
    }

    #[test]
    fn externally_called_or_mutually_recursive_functions_not_flagged() {
        // External call keeps it live; mutual recursion (each refers to the OTHER) keeps both live —
        // only pure SELF-reference is excluded.
        for src in [
            "fn c() { return 1 }\nc()\n",
            "fn p() { return q() }\nfn q() { return p() }\np()\n",
        ] {
            let program = parse(src).expect("parse");
            let u = collect_unused_bindings(&program, src);
            assert!(u.is_empty(), "no unused expected; src={src:?} u={u:?}");
        }
    }

    #[test]
    fn write_to_undeclared_still_unresolved() {
        // record_write must keep flagging a write to an undeclared name (not silently swallow it).
        let program = parse("undeclared = 5\n").expect("parse");
        let unresolved = collect_unresolved_identifiers(&program);
        assert!(
            unresolved.iter().any(|u| u.name.as_ref() == "undeclared"),
            "writing to an undeclared variable is still unresolved; got={unresolved:?}"
        );
    }

    #[test]
    fn reference_spans_include_uses_in_exported_fn_body() {
        // Directly exercises the fixed walker (also backs find-references / rename).
        let src = "import { publicUser } from \"./m\"\nexport fn f(u) {\n  return publicUser(u)\n}\n";
        let program = parse(src).expect("parse");
        let def = tishlang_ast::Span {
            start: (1, 10),
            end: (1, 20),
        };
        let refs = reference_spans_for_def(&program, src, "publicUser", def);
        assert_eq!(refs.len(), 2, "def + one use in exported body; refs={refs:?}");
        assert!(refs.contains(&def));
    }

    #[test]
    fn import_used_only_in_fn_param_default_not_unused() {
        // A name referenced only inside a parameter default was wrongly flagged unused because the
        // resolve walkers never descended into `default` expressions.
        let src = "import { base } from \"./m\"\nexport fn f(a = base(1)) {\n  return a\n}\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "base is used in the param default; u={u:?}");
    }

    #[test]
    fn import_used_only_in_arrow_param_default_not_unused() {
        let src = "import { fallback } from \"./m\"\nlet make = (x = fallback) => x\nmake()\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "fallback is used in the arrow param default; u={u:?}");
    }

    #[test]
    fn gotodef_resolves_name_used_in_param_default() {
        let src = "import { base } from \"./m\"\nfn f(a = base(1)) {\n  return a\n}\n";
        let program = parse(src).expect("parse");
        // cursor on `base` inside the default (0-indexed line 1, char 9)
        let def = definition_span(&program, src, 1, 9);
        assert_eq!(def, Some(tishlang_ast::Span { start: (1, 10), end: (1, 14) }), "def={def:?}");
    }

    #[test]
    fn gotodef_resolves_rest_param_body_use() {
        let src = "fn f(...rest) {\n  return rest\n}\n";
        let program = parse(src).expect("parse");
        // cursor on `rest` in the body (0-indexed line 1, char 9) -> the rest-param binding
        let def = definition_span(&program, src, 1, 9);
        assert_eq!(def, Some(tishlang_ast::Span { start: (1, 9), end: (1, 13) }), "def={def:?}");
    }

    #[test]
    fn name_at_cursor_finds_rest_param_decl() {
        let src = "fn f(...rest) {\n  return rest\n}\n";
        let program = parse(src).expect("parse");
        // cursor on the `...rest` declaration name (0-indexed line 0, char 8)
        let nu = name_at_cursor(&program, src, 0, 8).expect("name under cursor");
        assert_eq!(nu.name.as_ref(), "rest");
    }

    #[test]
    fn param_default_referencing_sibling_param_resolves_to_param_not_outer() {
        // `a` exists both as an outer binding (line 0) and a param (line 1). The default `b = a`
        // must resolve to the PARAM, not the outer — i.e. defaults are checked with params in scope.
        let src = "let a = 1\nfn f(a, b = a) {\n  return b\n}\nf(2)\n";
        let program = parse(src).expect("parse");
        // cursor on `a` inside `b = a` (0-indexed line 1, char 12)
        let def = definition_span(&program, src, 1, 12).expect("resolved");
        assert_eq!(def.start.0, 2, "should resolve to the param on line 2 (1-indexed); def={def:?}");
    }

    #[test]
    fn references_include_both_param_default_and_body_uses() {
        let src = "fn helper() { 1 }\nfn f(b = helper()) {\n  return helper()\n}\nf()\n";
        let program = parse(src).expect("parse");
        let def = tishlang_ast::Span { start: (1, 4), end: (1, 10) };
        let refs = reference_spans_for_def(&program, src, "helper", def);
        // def + use in param default + use in body
        assert_eq!(refs.len(), 3, "decl + default-use + body-use; refs={refs:?}");
    }

    #[test]
    fn unresolved_name_in_param_default_reported() {
        let src = "fn f(a = mispelt) {\n  return a\n}\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert_eq!(u.len(), 1, "u={u:?}");
        assert_eq!(u[0].name.as_ref(), "mispelt");
    }

    #[test]
    fn import_used_only_in_declare_fn_default_not_unused() {
        let src = "import { DEF } from \"./m\"\ndeclare fn connect(port = DEF): void\nconnect()\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        // The import used in the declare-fn param default must not be reported unused.
        // (Unrelated pre-existing declare-fn quirks may flag `connect`/`port`; not asserted here.)
        assert!(
            !u.iter().any(|b| b.name.as_ref() == "DEF"),
            "DEF is used in the declare-fn param default; u={u:?}"
        );
    }

    #[test]
    fn forward_reference_to_later_fn_not_unresolved() {
        let src = "fn caller() { return helper() }\nfn helper() { return 42 }\ncaller()\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert!(u.is_empty(), "helper is declared later but referenced (hoisted); u={u:?}");
    }

    #[test]
    fn mutual_recursion_not_unresolved_or_unused() {
        let src = "fn even(n) { if (n === 0) { return true } return odd(n - 1) }\nfn odd(n) { if (n === 0) { return false } return even(n - 1) }\neven(10)\n";
        let program = parse(src).expect("parse");
        assert!(
            collect_unresolved_identifiers(&program).is_empty(),
            "mutual recursion should resolve"
        );
        let unused = collect_unused_bindings(&program, src);
        assert!(unused.is_empty(), "both fns are used; unused={unused:?}");
    }

    #[test]
    fn gotodef_resolves_forward_fn_reference() {
        let src = "fn caller() { return helper() }\nfn helper() { return 42 }\n";
        let program = parse(src).expect("parse");
        // cursor on `helper` inside caller's body (0-indexed line 0, char 21)
        let def = definition_span(&program, src, 0, 21).expect("resolves");
        assert_eq!(def.start.0, 2, "should resolve to helper decl on line 2; def={def:?}");
    }

    #[test]
    fn genuinely_undefined_still_unresolved_with_hoisting() {
        let src = "fn f() { return nope() }\nf()\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert_eq!(u.len(), 1, "nope is undefined; u={u:?}");
        assert_eq!(u[0].name.as_ref(), "nope");
    }

    #[test]
    fn declare_fn_name_and_params_not_unused() {
        // Ambient declare-fn: the (called) name and its params must not be flagged unused.
        let src = "declare fn connect(port): void\nconnect()\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(u.is_empty(), "declare-fn name + ambient param must not be unused; u={u:?}");
        // and the call resolves (hoisted declare-fn name)
        assert!(collect_unresolved_identifiers(&program).is_empty(), "connect() should resolve");
    }

    #[test]
    fn runtime_globals_not_unresolved() {
        // Core interpreter globals must never be flagged as unresolved-name in the editor.
        let src = "let _ = [Number, Symbol, Error, TypeError, RangeError, SyntaxError, Promise, fetch, fetchAll, serve, htmlEscape, console, Math, JSON, Object, Array, String, Boolean, Date, RegExp, setTimeout]\n";
        let program = parse(src).expect("parse");
        let u = collect_unresolved_identifiers(&program);
        assert!(u.is_empty(), "no runtime global should be unresolved; u={u:?}");
    }

    #[test]
    fn jsx_component_tag_resolves_and_is_referenced() {
        // `fn Foo` used as a component. go-to-def on the tag, find-references over both tags.
        let src = "fn Foo() {}\nlet el = <Foo></Foo>\nel\n";
        let program = parse(src).expect("parse");
        // go-to-def on the opening `<Foo` (0-idx line 1, char 10) → the `fn Foo` decl (line 2, 1-idx).
        let open = definition_span(&program, src, 1, 10).expect("open tag resolves");
        assert_eq!(open.start, (1, 4), "resolves to fn Foo; got {open:?}");
        // and on the closing `</Foo` (char 16).
        let close = definition_span(&program, src, 1, 16).expect("close tag resolves");
        assert_eq!(close.start, (1, 4), "close tag resolves to fn Foo; got {close:?}");
        // find-references: the def + the opening tag + the closing tag.
        let def = tishlang_ast::Span { start: (1, 4), end: (1, 7) };
        let refs = reference_spans_for_def(&program, src, "Foo", def);
        assert_eq!(refs.len(), 3, "def + open tag + close tag; refs={refs:?}");
    }

    #[test]
    fn jsx_component_import_used_as_tag_not_unused() {
        let src = "import { Foo } from \"./c\"\nlet el = <Foo/>\nel\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(
            !u.iter().any(|b| b.name.as_ref() == "Foo"),
            "Foo is used as a component tag; u={u:?}"
        );
    }

    #[test]
    fn jsx_nested_component_tag_counted() {
        let src = "import { Row } from \"./c\"\nlet el = <div><Row/></div>\nel\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(
            !u.iter().any(|b| b.name.as_ref() == "Row"),
            "Row is used as a nested component; u={u:?}"
        );
    }

    #[test]
    fn lowercase_jsx_tag_does_not_mark_binding_used() {
        // Lowercase tags lower to string literals, not references — a same-named binding stays unused.
        let src = "let div = 1\nlet el = <div></div>\nel\n";
        let program = parse(src).expect("parse");
        let u = collect_unused_bindings(&program, src);
        assert!(
            u.iter().any(|b| b.name.as_ref() == "div"),
            "lowercase div binding is genuinely unused; u={u:?}"
        );
    }

    #[test]
    fn collect_unused_bindings_handles_large_files() {
        // A long chain of mutually-referencing functions exercises the per-binding reference scan at
        // scale. Under the old per-binding/per-identifier definition_span re-resolution this was
        // ~O(N^3) and froze (~1.4s at 300 lines); with the memo it is fast. Also a correctness check:
        // every function is reachable, so none is unused.
        let n = 300;
        let mut src = String::new();
        for i in 0..n {
            if i + 1 < n {
                src.push_str(&format!("fn f{i}() {{ return f{}() }}\n", i + 1));
            } else {
                src.push_str(&format!("fn f{i}() {{ return 0 }}\n"));
            }
        }
        src.push_str("f0()\n");
        let program = parse(&src).expect("parse");
        let u = collect_unused_bindings(&program, &src);
        assert!(
            u.is_empty(),
            "all chained fns are reachable; unused={:?}",
            u.iter().map(|b| b.name.as_ref()).collect::<Vec<_>>()
        );
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

    #[test]
    fn type_alias_rename_collects_decl_and_all_annotation_uses() {
        // #131: renaming a type alias must edit the declaration AND every `: T` annotation, or the
        // program silently breaks. Type names are in a namespace the value resolver doesn't see.
        let src = "type T = number\nlet a: T = 1\nfn f(x: T): T { return x }\n";
        let program = parse(src).expect("parse");
        fn span_text(src: &str, sp: &tishlang_ast::Span) -> String {
            let lines: Vec<&str> = src.lines().collect();
            let (sl, sc) = sp.start;
            let (_el, ec) = sp.end;
            lines[sl - 1].chars().skip(sc - 1).take(ec - sc).collect()
        }
        // Cursor on the declaration name `T` (0-indexed line 0, char 5).
        let spans = type_alias_rename_spans(&program, src, 0, 5).expect("type rename spans");
        assert_eq!(spans.len(), 4, "decl + let-ann + param-ann + return-ann; spans={spans:?}");
        for sp in &spans {
            assert_eq!(span_text(src, sp), "T", "every rename span covers the `T` token; {sp:?}");
        }
        // Initiating from a `: T` use also works (the param annotation, line 2 char 8).
        let from_use = type_alias_rename_spans(&program, src, 2, 8).expect("rename from use");
        assert_eq!(from_use.len(), 4, "same set whether initiated from decl or use");
        // A builtin used in an annotation (`number`, line 0 char 9) is not a user alias -> None.
        assert!(
            type_alias_rename_spans(&program, src, 0, 9).is_none(),
            "builtin `number` has no declaration to rename"
        );
    }
}

#[cfg(test)]
mod type_ref_unused_tests {
    use super::*;
    fn unused(src: &str) -> Vec<String> {
        collect_unused_bindings(&tishlang_parser::parse(src).expect("parse"), src)
            .iter().map(|b| b.name.to_string()).collect()
    }
    // #139: an import used only in a type annotation must not be flagged unused.
    #[test] fn type_only_import_not_unused() {
        let u = unused("import { Foo } from \"./m\"\nlet x: Foo = bar()\nx\n");
        assert!(!u.contains(&"Foo".to_string()), "type-only import Foo wrongly unused; u={u:?}");
    }
    // #139: a type alias referenced only in an annotation must not be flagged unused.
    #[test] fn type_alias_used_in_annotation_not_unused() {
        let u = unused("type T = number\nfn f(x: T) { return x }\nf(1)\n");
        assert!(!u.contains(&"T".to_string()), "type alias T wrongly unused; u={u:?}");
    }
    // guard: a genuinely-unused import is still reported.
    #[test] fn genuinely_unused_import_still_flagged() {
        let u = unused("import { Dead } from \"./m\"\nlet y = 1\ny\n");
        assert!(u.contains(&"Dead".to_string()), "Dead should be unused; u={u:?}");
    }
}
