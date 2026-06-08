//! Phase 2: a gradual type checker over `TypeAnnotation`.
//!
//! Produces [`TypeDiagnostic`]s for annotation violations that are *provable* from local
//! information: a `let x: T = e` whose `e` has a concrete conflicting type, a `return e` that
//! conflicts with the declared return type, an assignment that conflicts with a variable's declared
//! type, and a call whose argument conflicts with the parameter type.
//!
//! It is deliberately **gradual**: when an expression's type can't be determined (a call to a
//! function with no signature, a dynamic value, `any`), [`synth`] yields `None` and nothing is
//! flagged — so valid code is never a false positive. `assignable` is likewise conservative: it
//! only reports a mismatch between two *concretely known* types (`number`/`string`/`boolean`,
//! arrays of those, and object shapes); anything it can't resolve is treated as compatible.
//!
//! This is the soundness/hardening foundation; richer unification, real unions, and control-flow
//! narrowing come later (and would move the representation to a dedicated `Ty` IR).

use std::collections::HashMap;
use tishlang_ast::{
    ArrayElement, BinOp, CallArg, Expr, FunParam, Literal, MemberProp, ObjectProp, Program, Span,
    Statement, TypeAnnotation, UnaryOp,
};

#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub message: String,
    pub span: Span,
}

#[derive(Clone)]
struct FnSig {
    params: Vec<Option<TypeAnnotation>>,
    ret: Option<TypeAnnotation>,
}

struct CheckCtx {
    scopes: Vec<HashMap<String, TypeAnnotation>>,
    sigs: HashMap<String, FnSig>,
    aliases: HashMap<String, TypeAnnotation>,
    ret_stack: Vec<Option<TypeAnnotation>>,
    diags: Vec<TypeDiagnostic>,
}

/// Check a program, returning a diagnostic for every provable annotation violation.
pub fn check_program(program: &Program) -> Vec<TypeDiagnostic> {
    let mut ctx = CheckCtx {
        scopes: vec![HashMap::new()],
        sigs: HashMap::new(),
        aliases: HashMap::new(),
        ret_stack: Vec::new(),
        diags: Vec::new(),
    };
    ctx.collect_aliases(&program.statements);
    ctx.collect_sigs(&program.statements);
    ctx.check_block(&program.statements);
    ctx.diags
}

// ── helpers: type constructors / predicates ─────────────────────────────────────────────────

fn simple(s: &str) -> TypeAnnotation {
    TypeAnnotation::Simple(s.into())
}
fn is_any(ann: &TypeAnnotation) -> bool {
    matches!(ann, TypeAnnotation::Simple(s) if s.as_ref() == "any")
}
fn is_named(ann: &TypeAnnotation, n: &str) -> bool {
    matches!(ann, TypeAnnotation::Simple(s) if s.as_ref() == n)
}

/// Display a type for diagnostics (close to the source syntax).
fn show(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Simple(s) => s.to_string(),
        TypeAnnotation::Array(t) => format!("{}[]", show(t)),
        TypeAnnotation::Object(fs) => {
            let inner: Vec<String> = fs.iter().map(|(k, t)| format!("{}: {}", k, show(t))).collect();
            format!("{{ {} }}", inner.join(", "))
        }
        TypeAnnotation::Function { params, returns } => {
            let ps: Vec<String> = params.iter().map(show).collect();
            format!("({}) => {}", ps.join(", "), show(returns))
        }
        TypeAnnotation::Union(ts) => ts.iter().map(show).collect::<Vec<_>>().join(" | "),
    }
}

/// Resolve a `Simple` alias name to its definition (bounded to avoid cycles).
fn resolve<'a>(
    ann: &'a TypeAnnotation,
    aliases: &'a HashMap<String, TypeAnnotation>,
    depth: u8,
) -> &'a TypeAnnotation {
    if depth > 8 {
        return ann;
    }
    if let TypeAnnotation::Simple(s) = ann {
        if let Some(t) = aliases.get(s.as_ref()) {
            return resolve(t, aliases, depth + 1);
        }
    }
    ann
}

/// Is `actual` assignable to `expected`? Conservative: only the concretely-known primitive / array
/// / object-shape mismatches return `false`; everything uncertain returns `true` (no false flag).
fn assignable(
    actual: &TypeAnnotation,
    expected: &TypeAnnotation,
    aliases: &HashMap<String, TypeAnnotation>,
) -> bool {
    let a = resolve(actual, aliases, 0);
    let e = resolve(expected, aliases, 0);
    if is_any(a) || is_any(e) {
        return true;
    }
    // `null`/`void`/`undefined` are leniently compatible (tish uses `null` for optionals; checking
    // it strictly would false-positive without real union/optional support).
    if matches!(a, TypeAnnotation::Simple(s) if matches!(s.as_ref(), "null" | "void" | "undefined")) {
        return true;
    }
    use TypeAnnotation::*;
    match (a, e) {
        (Simple(x), Simple(y)) => {
            // Strict only among the three scalar primitives; any user-defined / unresolved name is
            // treated as compatible.
            let strict = |s: &str| matches!(s, "number" | "string" | "boolean");
            if strict(x.as_ref()) && strict(y.as_ref()) {
                x.as_ref() == y.as_ref()
            } else {
                true
            }
        }
        (Array(ax), Array(ey)) => assignable(ax, ey, aliases),
        // array vs non-array (after alias/any resolution) is a clear mismatch
        (Array(_), Simple(_)) | (Simple(_), Array(_)) => false,
        (Object(af), Object(ef)) => ef.iter().all(|(k, et)| {
            af.iter()
                .find(|(ak, _)| ak.as_ref() == k.as_ref())
                .map(|(_, at)| assignable(at, et, aliases))
                .unwrap_or(false)
        }),
        // a union actual fits only if every member fits; a union expected accepts any matching member
        (Union(axs), _) => axs.iter().all(|t| assignable(t, e, aliases)),
        (_, Union(eys)) => eys.iter().any(|t| assignable(a, t, aliases)),
        // anything else (functions, object-vs-named we couldn't resolve, …) -> lenient
        _ => true,
    }
}

// ── pre-passes: collect aliases + function signatures (recursively) ──────────────────────────

impl CheckCtx {
    fn collect_aliases(&mut self, stmts: &[Statement]) {
        for s in stmts {
            match s {
                Statement::TypeAlias { name, ty, .. } => {
                    self.aliases.insert(name.to_string(), ty.clone());
                }
                _ => for_each_child_block(s, &mut |b| self.collect_aliases(b)),
            }
        }
    }

    fn collect_sigs(&mut self, stmts: &[Statement]) {
        for s in stmts {
            if let Statement::FunDecl {
                name,
                params,
                return_type,
                body,
                ..
            } = s
            {
                let p = params
                    .iter()
                    .map(|fp| match fp {
                        FunParam::Simple(tp) => tp.type_ann.clone(),
                        _ => None,
                    })
                    .collect();
                self.sigs.insert(
                    name.to_string(),
                    FnSig {
                        params: p,
                        ret: return_type.clone(),
                    },
                );
                self.collect_sigs(std::slice::from_ref(body));
            } else {
                for_each_child_block(s, &mut |b| self.collect_sigs(b));
            }
        }
    }

    // ── scope helpers ────────────────────────────────────────────────────────────────────────

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
    fn define(&mut self, name: &str, ty: TypeAnnotation) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), ty);
        }
    }
    fn lookup(&self, name: &str) -> Option<TypeAnnotation> {
        self.scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }

    // ── statement checking ─────────────────────────────────────────────────────────────────────

    fn check_block(&mut self, stmts: &[Statement]) {
        self.push();
        for s in stmts {
            self.check_stmt(s);
        }
        self.pop();
    }

    fn check_stmt(&mut self, s: &Statement) {
        match s {
            Statement::VarDecl {
                name,
                type_ann,
                init,
                ..
            } => {
                if let Some(e) = init {
                    let t = self.synth(e);
                    if let (Some(ann), Some(t)) = (type_ann, &t) {
                        if !assignable(t, ann, &self.aliases) {
                            self.diags.push(TypeDiagnostic {
                                message: format!(
                                    "Type '{}' is not assignable to type '{}'.",
                                    show(t),
                                    show(ann)
                                ),
                                span: e.span(),
                            });
                        }
                    }
                }
                // Bind the *declared* type if annotated; otherwise the local is dynamic — bind `any`
                // so later uses/reassignments are never flagged. (tish is gradual: an unannotated
                // `let x = 5` may legitimately be reassigned `x = "s"`, unlike TS let-widening.)
                let bound = type_ann.clone().unwrap_or_else(|| simple("any"));
                self.define(name.as_ref(), bound);
            }
            Statement::VarDeclDestructure { init, .. } => {
                self.synth(init);
            }
            Statement::ExprStmt { expr, .. } => {
                self.synth(expr);
            }
            Statement::Return { value, .. } => {
                let expected = self.ret_stack.last().cloned().flatten();
                if let Some(e) = value {
                    let t = self.synth(e);
                    if let (Some(rt), Some(t)) = (&expected, &t) {
                        if !assignable(t, rt, &self.aliases) {
                            self.diags.push(TypeDiagnostic {
                                message: format!(
                                    "Type '{}' is not assignable to the declared return type '{}'.",
                                    show(t),
                                    show(rt)
                                ),
                                span: e.span(),
                            });
                        }
                    }
                }
            }
            Statement::FunDecl {
                params,
                rest_param,
                return_type,
                body,
                ..
            } => {
                self.push();
                for fp in params {
                    if let FunParam::Simple(tp) = fp {
                        if let Some(ann) = &tp.type_ann {
                            self.define(tp.name.as_ref(), ann.clone());
                        }
                    }
                }
                if let Some(rp) = rest_param {
                    if let Some(ann) = &rp.type_ann {
                        self.define(rp.name.as_ref(), ann.clone());
                    }
                }
                self.ret_stack.push(return_type.clone());
                self.check_stmt(body);
                self.ret_stack.pop();
                self.pop();
            }
            Statement::Block { statements, .. } => self.check_block(statements),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.synth(cond);
                self.check_stmt(then_branch);
                if let Some(e) = else_branch {
                    self.check_stmt(e);
                }
            }
            Statement::While { cond, body, .. } | Statement::DoWhile { cond, body, .. } => {
                self.synth(cond);
                self.check_stmt(body);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.push();
                if let Some(i) = init {
                    self.check_stmt(i);
                }
                if let Some(c) = cond {
                    self.synth(c);
                }
                if let Some(u) = update {
                    self.synth(u);
                }
                self.check_stmt(body);
                self.pop();
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                self.push();
                // Bind the loop var to the element type when the iterable is a known `T[]`.
                if let Some(TypeAnnotation::Array(elem)) =
                    self.synth(iterable).map(|t| resolve(&t, &self.aliases, 0).clone())
                {
                    self.define(name.as_ref(), *elem);
                }
                self.check_stmt(body);
                self.pop();
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                self.synth(expr);
                for (g, body) in cases {
                    if let Some(g) = g {
                        self.synth(g);
                    }
                    self.check_block(body);
                }
                if let Some(b) = default_body {
                    self.check_block(b);
                }
            }
            Statement::Try {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                self.check_stmt(body);
                if let Some(b) = catch_body {
                    self.check_stmt(b);
                }
                if let Some(b) = finally_body {
                    self.check_stmt(b);
                }
            }
            Statement::Throw { value, .. } => {
                self.synth(value);
            }
            _ => {}
        }
    }

    // ── expression type synthesis (gradual) + nested call/assign checks ──────────────────────────

    fn synth(&mut self, e: &Expr) -> Option<TypeAnnotation> {
        match e {
            Expr::Literal { value, .. } => Some(match value {
                Literal::Number(_) => simple("number"),
                Literal::String(_) => simple("string"),
                Literal::Bool(_) => simple("boolean"),
                Literal::Null => simple("null"),
            }),
            Expr::Ident { name, .. } => self.lookup(name.as_ref()),
            Expr::Binary { left, op, right, .. } => {
                let lt = self.synth(left);
                let rt = self.synth(right);
                bin_type(*op, lt.as_ref(), rt.as_ref())
            }
            Expr::Unary { op, operand, .. } => {
                let t = self.synth(operand);
                match op {
                    UnaryOp::Not => Some(simple("boolean")),
                    UnaryOp::Neg | UnaryOp::Pos => {
                        if t.as_ref().map(|x| is_named(x, "number")).unwrap_or(false) {
                            Some(simple("number"))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            Expr::Call { callee, args, .. } => {
                let arg_types: Vec<Option<TypeAnnotation>> = args
                    .iter()
                    .map(|a| match a {
                        CallArg::Expr(x) => self.synth(x),
                        CallArg::Spread(x) => {
                            self.synth(x);
                            None
                        }
                    })
                    .collect();
                if let Expr::Ident { name, .. } = callee.as_ref() {
                    if let Some(sig) = self.sigs.get(name.as_ref()).cloned() {
                        for (i, pt) in sig.params.iter().enumerate() {
                            if let (Some(pt), Some(Some(at))) = (pt, arg_types.get(i)) {
                                if !assignable(at, pt, &self.aliases) {
                                    let span = match &args[i] {
                                        CallArg::Expr(x) | CallArg::Spread(x) => x.span(),
                                    };
                                    self.diags.push(TypeDiagnostic {
                                        message: format!(
                                            "Argument of type '{}' is not assignable to parameter of type '{}'.",
                                            show(at),
                                            show(pt)
                                        ),
                                        span,
                                    });
                                }
                            }
                        }
                        return sig.ret.clone();
                    }
                } else {
                    self.synth(callee);
                }
                None
            }
            Expr::Member { object, prop, .. } => {
                let ot = self.synth(object);
                if let (Some(ot), MemberProp::Name { name, .. }) = (ot, prop) {
                    let resolved = resolve(&ot, &self.aliases, 0).clone();
                    match &resolved {
                        TypeAnnotation::Object(fields) => {
                            return fields
                                .iter()
                                .find(|(k, _)| k.as_ref() == name.as_ref())
                                .map(|(_, t)| t.clone());
                        }
                        TypeAnnotation::Array(_) if name.as_ref() == "length" => {
                            return Some(simple("number"));
                        }
                        TypeAnnotation::Simple(s)
                            if s.as_ref() == "string" && name.as_ref() == "length" =>
                        {
                            return Some(simple("number"));
                        }
                        _ => {}
                    }
                }
                None
            }
            Expr::Index { object, index, .. } => {
                let ot = self.synth(object);
                self.synth(index);
                if let Some(TypeAnnotation::Array(elem)) =
                    ot.map(|t| resolve(&t, &self.aliases, 0).clone())
                {
                    return Some(*elem);
                }
                None
            }
            Expr::Assign { name, value, .. } => {
                let vt = self.synth(value);
                if let (Some(target), Some(vt)) = (self.lookup(name.as_ref()), &vt) {
                    if !assignable(vt, &target, &self.aliases) {
                        self.diags.push(TypeDiagnostic {
                            message: format!(
                                "Type '{}' is not assignable to type '{}'.",
                                show(vt),
                                show(&target)
                            ),
                            span: value.span(),
                        });
                    }
                    return Some(target);
                }
                vt
            }
            Expr::CompoundAssign { value, .. } | Expr::LogicalAssign { value, .. } => {
                self.synth(value);
                None
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.synth(cond);
                let t = self.synth(then_branch);
                let f = self.synth(else_branch);
                match (t, f) {
                    (Some(a), Some(b)) if a == b => Some(a),
                    _ => None,
                }
            }
            Expr::Array { elements, .. } => {
                let mut elem: Option<TypeAnnotation> = None;
                for el in elements {
                    match el {
                        ArrayElement::Expr(x) => {
                            let t = self.synth(x)?;
                            match &elem {
                                None => elem = Some(t),
                                Some(p) if *p != t => return None,
                                _ => {}
                            }
                        }
                        ArrayElement::Spread(x) => {
                            self.synth(x);
                            return None;
                        }
                    }
                }
                elem.map(|t| TypeAnnotation::Array(Box::new(t)))
            }
            Expr::Object { props, .. } => {
                let mut fields = Vec::new();
                for p in props {
                    match p {
                        ObjectProp::KeyValue(k, v) => {
                            let t = self.synth(v)?;
                            fields.push((k.clone(), t));
                        }
                        ObjectProp::Spread(v) => {
                            self.synth(v);
                            return None;
                        }
                    }
                }
                Some(TypeAnnotation::Object(fields))
            }
            _ => None,
        }
    }
}

/// Result type of a binary op given (optional) operand types. Mirrors the runtime/codegen rules,
/// gradual: any uncertainty -> `None`.
fn bin_type(
    op: BinOp,
    lt: Option<&TypeAnnotation>,
    rt: Option<&TypeAnnotation>,
) -> Option<TypeAnnotation> {
    let both = |n: &str| {
        lt.map(|t| is_named(t, n)).unwrap_or(false) && rt.map(|t| is_named(t, n)).unwrap_or(false)
    };
    use BinOp::*;
    match op {
        Add => {
            if both("number") {
                Some(simple("number"))
            } else if both("string") {
                Some(simple("string"))
            } else {
                None
            }
        }
        Sub | Mul | Div | Mod | Pow => {
            if both("number") {
                Some(simple("number"))
            } else {
                None
            }
        }
        Lt | Le | Gt | Ge | StrictEq | StrictNe => {
            if both("number") || both("string") || both("boolean") {
                Some(simple("boolean"))
            } else {
                None
            }
        }
        And | Or => {
            if both("boolean") {
                Some(simple("boolean"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Run `f` over the child statement-blocks of `s` (loop/if/fn/switch/try bodies) for the
/// recursive pre-passes. Single-statement bodies are passed as one-element slices.
fn for_each_child_block(s: &Statement, f: &mut dyn FnMut(&[Statement])) {
    match s {
        Statement::Block { statements, .. } => f(statements),
        Statement::If {
            then_branch,
            else_branch,
            ..
        } => {
            f(std::slice::from_ref(then_branch));
            if let Some(e) = else_branch {
                f(std::slice::from_ref(e));
            }
        }
        Statement::While { body, .. }
        | Statement::DoWhile { body, .. }
        | Statement::ForOf { body, .. }
        | Statement::FunDecl { body, .. } => f(std::slice::from_ref(body)),
        Statement::For { init, body, .. } => {
            if let Some(i) = init {
                f(std::slice::from_ref(i));
            }
            f(std::slice::from_ref(body));
        }
        Statement::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, body) in cases {
                f(body);
            }
            if let Some(b) = default_body {
                f(b);
            }
        }
        Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_parser::parse;

    fn diags(src: &str) -> Vec<String> {
        let prog = parse(src).unwrap();
        check_program(&prog).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn ok_programs_have_no_diagnostics() {
        for src in [
            "let x: number = 5",
            "let s: string = \"hi\"",
            "let b: boolean = true",
            "let x: number = 1 + 2 * 3",
            "let s: string = \"a\" + \"b\"",
            "fn f(a: number): number { return a + 1 }",
            "fn f(a: number) {} f(5)",
            "let x: number = unknownCall()",            // gradual: unknown -> no error
            "let x: any = \"anything\"",                 // any accepts anything
            "type P = { x: number, y: number }\nlet p: P = { x: 1, y: 2 }",
            "let xs: number[] = [1, 2, 3]",
            "fn f(a: number): number { return a }\nlet n: number = f(2)",
        ] {
            assert_eq!(diags(src), Vec::<String>::new(), "unexpected diagnostics for: {src}");
        }
    }

    #[test]
    fn flags_decl_mismatch() {
        assert_eq!(diags("let x: number = \"s\"").len(), 1);
        assert_eq!(diags("let s: string = 42").len(), 1);
        assert_eq!(diags("let b: boolean = 1").len(), 1);
    }

    #[test]
    fn flags_return_mismatch() {
        assert_eq!(diags("fn f(): number { return \"s\" }").len(), 1);
        assert_eq!(diags("fn f(): string { return 5 }").len(), 1);
    }

    #[test]
    fn flags_call_arg_mismatch() {
        assert_eq!(diags("fn f(a: number) {}\nf(\"s\")").len(), 1);
        assert_eq!(diags("fn f(a: number, b: string) {}\nf(1, 2)").len(), 1);
    }

    #[test]
    fn flags_reassignment_mismatch() {
        assert_eq!(diags("let x: number = 5\nx = \"s\"").len(), 1);
    }

    #[test]
    fn flags_struct_field_mismatch() {
        assert_eq!(diags("let p: { x: number } = { x: \"s\" }").len(), 1);
    }

    #[test]
    fn gradual_no_false_positive_on_unknown_arg() {
        // arg type unknown (param of an unknown fn) -> no error
        assert_eq!(diags("fn f(a: number) {}\nf(someUnknown())"), Vec::<String>::new());
    }
}
