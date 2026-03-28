//! Pretty-print Tish AST to source. Style: 2-space indent, braces for blocks, trailing newline.

use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    ExportDeclaration, Expr, ImportSpecifier, JsxAttrValue, JsxChild, JsxProp,
    Literal, LogicalAssignOp, MemberProp, ObjectProp, Program, Statement, TypeAnnotation,
    TypedParam, UnaryOp,
};

/// Format Tish source. On parse error, returns the parser message.
pub fn format_source(source: &str) -> Result<String, String> {
    let program = tishlang_parser::parse(source)?;
    Ok(format_program(&program))
}

pub fn format_program(program: &Program) -> String {
    let mut p = Printer::new();
    for (i, s) in program.statements.iter().enumerate() {
        if i > 0 {
            p.buf.push('\n');
        }
        p.stmt(s, 0);
        if !matches!(s, Statement::Import { .. } | Statement::Export { .. }) {
            p.buf.push('\n');
        }
    }
    if !p.buf.ends_with('\n') {
        p.buf.push('\n');
    }
    p.buf
}

struct Printer {
    buf: String,
}

impl Printer {
    fn new() -> Self {
        Self {
            buf: String::with_capacity(4096),
        }
    }

    fn indent(&mut self, level: usize) {
        for _ in 0..level {
            self.buf.push_str("  ");
        }
    }

    fn stmt(&mut self, s: &Statement, level: usize) {
        match s {
            Statement::Block { statements, .. } => {
                self.indent(level);
                self.buf.push_str("{\n");
                for st in statements {
                    self.stmt(st, level + 1);
                    self.buf.push('\n');
                }
                self.indent(level);
                self.buf.push('}');
            }
            Statement::VarDecl {
                name,
                mutable,
                type_ann,
                init,
                ..
            } => {
                self.indent(level);
                self.buf.push_str(if *mutable { "let " } else { "const " });
                self.buf.push_str(name);
                if let Some(t) = type_ann {
                    self.buf.push_str(": ");
                    self.type_ann(t);
                }
                if let Some(e) = init {
                    self.buf.push_str(" = ");
                    self.expr(e);
                }
            }
            Statement::VarDeclDestructure {
                pattern, mutable, init, ..
            } => {
                self.indent(level);
                self.buf.push_str(if *mutable { "let " } else { "const " });
                self.destruct_pat(pattern);
                self.buf.push_str(" = ");
                self.expr(init);
            }
            Statement::ExprStmt { expr, .. } => {
                self.indent(level);
                self.expr(expr);
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("if (");
                self.expr(cond);
                self.buf.push_str(") ");
                self.stmt_inline_or_block(then_branch, level);
                if let Some(else_b) = else_branch {
                    self.buf.push_str(" else ");
                    self.stmt_inline_or_block(else_b, level);
                }
            }
            Statement::While { cond, body, .. } => {
                self.indent(level);
                self.buf.push_str("while (");
                self.expr(cond);
                self.buf.push_str(") ");
                self.stmt_inline_or_block(body, level);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("for (");
                if let Some(i) = init {
                    self.stmt_for_header(i);
                }
                self.buf.push_str("; ");
                if let Some(c) = cond {
                    self.expr(c);
                }
                self.buf.push_str("; ");
                if let Some(u) = update {
                    self.expr(u);
                }
                self.buf.push_str(") ");
                self.stmt_inline_or_block(body, level);
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("for (let ");
                self.buf.push_str(name);
                self.buf.push_str(" of ");
                self.expr(iterable);
                self.buf.push_str(") ");
                self.stmt_inline_or_block(body, level);
            }
            Statement::Return { value, .. } => {
                self.indent(level);
                self.buf.push_str("return");
                if let Some(v) = value {
                    self.buf.push(' ');
                    self.expr(v);
                }
            }
            Statement::Break { .. } => {
                self.indent(level);
                self.buf.push_str("break");
            }
            Statement::Continue { .. } => {
                self.indent(level);
                self.buf.push_str("continue");
            }
            Statement::FunDecl {
                async_,
                name,
                params,
                rest_param,
                return_type,
                body,
                ..
            } => {
                self.indent(level);
                if *async_ {
                    self.buf.push_str("async ");
                }
                self.buf.push_str("fn ");
                self.buf.push_str(name);
                self.buf.push('(');
                self.param_list(params, rest_param);
                self.buf.push(')');
                if let Some(rt) = return_type {
                    self.buf.push_str(": ");
                    self.type_ann(rt);
                }
                if let Statement::ExprStmt { expr, .. } = body.as_ref() {
                    self.buf.push_str(" = ");
                    self.expr(expr);
                } else {
                    self.buf.push(' ');
                    self.stmt(body, level);
                }
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("switch (");
                self.expr(expr);
                self.buf.push_str(") {\n");
                for (case_e, stmts) in cases {
                    self.indent(level + 1);
                    match case_e {
                        Some(e) => {
                            self.buf.push_str("case ");
                            self.expr(e);
                            self.buf.push_str(":\n");
                        }
                        None => self.buf.push_str("default:\n"),
                    }
                    for st in stmts {
                        self.stmt(st, level + 2);
                        self.buf.push('\n');
                    }
                }
                if let Some(def) = default_body {
                    self.indent(level + 1);
                    self.buf.push_str("default:\n");
                    for st in def {
                        self.stmt(st, level + 2);
                        self.buf.push('\n');
                    }
                }
                self.indent(level);
                self.buf.push('}');
            }
            Statement::DoWhile { body, cond, .. } => {
                self.indent(level);
                self.buf.push_str("do ");
                self.stmt_inline_or_block(body, level);
                self.buf.push_str(" while (");
                self.expr(cond);
                self.buf.push(')');
            }
            Statement::Throw { value, .. } => {
                self.indent(level);
                self.buf.push_str("throw ");
                self.expr(value);
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("try ");
                self.stmt_inline_or_block(body, level);
                if let (Some(p), Some(cb)) = (catch_param, catch_body) {
                    self.buf.push_str(" catch (");
                    self.buf.push_str(p);
                    self.buf.push_str(") ");
                    self.stmt_inline_or_block(cb, level);
                }
                if let Some(fb) = finally_body {
                    self.buf.push_str(" finally ");
                    self.stmt_inline_or_block(fb, level);
                }
            }
            Statement::Import { specifiers, from, .. } => {
                self.indent(level);
                self.buf.push_str("import ");
                self.import_specs(specifiers);
                self.buf.push_str(" from ");
                self.string_lit(from.as_ref());
            }
            Statement::Export { declaration, .. } => {
                self.indent(level);
                self.buf.push_str("export ");
                match declaration.as_ref() {
                    ExportDeclaration::Named(inner) => {
                        if let Statement::FunDecl { async_, name, params, rest_param, return_type, body, .. } =
                            inner.as_ref()
                        {
                            if *async_ {
                                self.buf.push_str("async ");
                            }
                            self.buf.push_str("fn ");
                            self.buf.push_str(name);
                            self.buf.push('(');
                            self.param_list(params, rest_param);
                            self.buf.push(')');
                            if let Some(rt) = return_type {
                                self.buf.push_str(": ");
                                self.type_ann(rt);
                            }
                            self.buf.push(' ');
                            self.stmt(body, level);
                        } else {
                            self.stmt(inner, level);
                        }
                    }
                    ExportDeclaration::Default(e) => {
                        self.buf.push_str("default ");
                        self.expr(e);
                    }
                }
            }
        }
    }

    fn stmt_for_header(&mut self, s: &Statement) {
        match s {
            Statement::VarDecl {
                name,
                mutable,
                type_ann,
                init,
                ..
            } => {
                self.buf.push_str(if *mutable { "let " } else { "const " });
                self.buf.push_str(name);
                if let Some(t) = type_ann {
                    self.buf.push_str(": ");
                    self.type_ann(t);
                }
                if let Some(e) = init {
                    self.buf.push_str(" = ");
                    self.expr(e);
                }
            }
            Statement::ExprStmt { expr, .. } => self.expr(expr),
            _ => {}
        }
    }

    fn stmt_inline_or_block(&mut self, s: &Statement, level: usize) {
        if let Statement::Block { .. } = s {
            self.stmt(s, level);
        } else {
            self.buf.push_str("{\n");
            self.stmt(s, level + 1);
            self.buf.push('\n');
            self.indent(level);
            self.buf.push('}');
        }
    }

    fn import_specs(&mut self, specs: &[ImportSpecifier]) {
        if specs.len() == 1 {
            match &specs[0] {
                ImportSpecifier::Default(n) => self.buf.push_str(n.as_ref()),
                ImportSpecifier::Namespace(n) => {
                    self.buf.push_str("* as ");
                    self.buf.push_str(n.as_ref());
                }
                ImportSpecifier::Named { name, alias } => {
                    self.buf.push_str("{ ");
                    self.buf.push_str(name.as_ref());
                    if let Some(a) = alias {
                        self.buf.push_str(" as ");
                        self.buf.push_str(a.as_ref());
                    }
                    self.buf.push_str(" }");
                }
            }
            return;
        }
        self.buf.push_str("{ ");
        for (i, sp) in specs.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            match sp {
                ImportSpecifier::Named { name, alias } => {
                    self.buf.push_str(name.as_ref());
                    if let Some(a) = alias {
                        self.buf.push_str(" as ");
                        self.buf.push_str(a.as_ref());
                    }
                }
                _ => {}
            }
        }
        self.buf.push_str(" }");
    }

    fn param_list(&mut self, params: &[TypedParam], rest: &Option<TypedParam>) {
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.buf.push_str(p.name.as_ref());
            if let Some(t) = &p.type_ann {
                self.buf.push_str(": ");
                self.type_ann(t);
            }
            if let Some(e) = &p.default {
                self.buf.push_str(" = ");
                self.expr(e);
            }
        }
        if let Some(r) = rest {
            if !params.is_empty() {
                self.buf.push_str(", ");
            }
            self.buf.push_str("...");
            self.buf.push_str(r.name.as_ref());
            if let Some(t) = &r.type_ann {
                self.buf.push_str(": ");
                self.type_ann(t);
            }
        }
    }

    fn destruct_pat(&mut self, p: &DestructPattern) {
        match p {
            DestructPattern::Array(elems) => {
                self.buf.push('[');
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    match e {
                        Some(DestructElement::Ident(n)) => self.buf.push_str(n.as_ref()),
                        Some(DestructElement::Pattern(inner)) => self.destruct_pat(inner),
                        Some(DestructElement::Rest(n)) => {
                            self.buf.push_str("...");
                            self.buf.push_str(n.as_ref());
                        }
                        None => {}
                    }
                }
                self.buf.push(']');
            }
            DestructPattern::Object(props) => {
                self.buf.push_str("{ ");
                for (i, pr) in props.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.buf.push_str(pr.key.as_ref());
                    match &pr.value {
                        DestructElement::Ident(n) if n.as_ref() != pr.key.as_ref() => {
                            self.buf.push_str(": ");
                            self.buf.push_str(n.as_ref());
                        }
                        DestructElement::Ident(_) => {}
                        DestructElement::Pattern(inner) => {
                            self.buf.push_str(": ");
                            self.destruct_pat(inner);
                        }
                        DestructElement::Rest(n) => {
                            self.buf.push_str(": ...");
                            self.buf.push_str(n.as_ref());
                        }
                    }
                }
                self.buf.push_str(" }");
            }
        }
    }

    fn type_ann(&mut self, t: &TypeAnnotation) {
        match t {
            TypeAnnotation::Simple(s) => self.buf.push_str(s.as_ref()),
            TypeAnnotation::Array(inner) => {
                self.type_ann(inner);
                self.buf.push_str("[]");
            }
            TypeAnnotation::Object(props) => {
                self.buf.push_str("{ ");
                for (i, (k, v)) in props.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.buf.push_str(k.as_ref());
                    self.buf.push_str(": ");
                    self.type_ann(v);
                }
                self.buf.push_str(" }");
            }
            TypeAnnotation::Function { params, returns } => {
                self.buf.push('(');
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.type_ann(p);
                }
                self.buf.push_str(") => ");
                self.type_ann(returns);
            }
            TypeAnnotation::Union(u) => {
                for (i, x) in u.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(" | ");
                    }
                    self.type_ann(x);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => {
                    if n.fract() == 0.0 && n.abs() < 1e15 {
                        self.buf.push_str(&format!("{}", *n as i64));
                    } else {
                        self.buf.push_str(&format!("{}", n));
                    }
                }
                Literal::String(s) => self.string_lit(s.as_ref()),
                Literal::Bool(b) => self.buf.push_str(if *b { "true" } else { "false" }),
                Literal::Null => self.buf.push_str("null"),
            },
            Expr::Ident { name, .. } => self.buf.push_str(name.as_ref()),
            Expr::Binary { left, op, right, .. } => {
                self.expr(left);
                self.buf.push(' ');
                self.buf.push_str(binop(*op));
                self.buf.push(' ');
                self.expr(right);
            }
            Expr::Unary { op, operand, .. } => {
                match op {
                    UnaryOp::Not => self.buf.push_str("!"),
                    UnaryOp::Neg => self.buf.push_str("-"),
                    UnaryOp::Pos => self.buf.push_str("+"),
                    UnaryOp::BitNot => self.buf.push_str("~"),
                    UnaryOp::Void => self.buf.push_str("void "),
                }
                self.expr(operand);
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                self.buf.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    match a {
                        CallArg::Expr(ex) => self.expr(ex),
                        CallArg::Spread(ex) => {
                            self.buf.push_str("...");
                            self.expr(ex);
                        }
                    }
                }
                self.buf.push(')');
            }
            Expr::New { callee, args, .. } => {
                self.buf.push_str("new ");
                self.expr(callee);
                if !args.is_empty() {
                    self.buf.push('(');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            self.buf.push_str(", ");
                        }
                        match a {
                            CallArg::Expr(ex) => self.expr(ex),
                            CallArg::Spread(ex) => {
                                self.buf.push_str("...");
                                self.expr(ex);
                            }
                        }
                    }
                    self.buf.push(')');
                }
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                self.expr(object);
                if *optional {
                    self.buf.push_str("?.");
                } else {
                    self.buf.push('.');
                }
                match prop {
                    MemberProp::Name(n) => self.buf.push_str(n.as_ref()),
                    MemberProp::Expr(ex) => {
                        self.buf.push('[');
                        self.expr(ex);
                        self.buf.push(']');
                    }
                }
            }
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                self.expr(object);
                if *optional {
                    self.buf.push_str("?.[");
                } else {
                    self.buf.push('[');
                }
                self.expr(index);
                self.buf.push(']');
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(cond);
                self.buf.push_str(" ? ");
                self.expr(then_branch);
                self.buf.push_str(" : ");
                self.expr(else_branch);
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.expr(left);
                self.buf.push_str(" ?? ");
                self.expr(right);
            }
            Expr::Array { elements, .. } => {
                self.buf.push('[');
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    match el {
                        ArrayElement::Expr(ex) => self.expr(ex),
                        ArrayElement::Spread(ex) => {
                            self.buf.push_str("...");
                            self.expr(ex);
                        }
                    }
                }
                self.buf.push(']');
            }
            Expr::Object { props, .. } => {
                self.buf.push_str("{ ");
                for (i, pr) in props.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    match pr {
                        ObjectProp::KeyValue(k, v) => {
                            self.buf.push_str(k.as_ref());
                            self.buf.push_str(": ");
                            self.expr(v);
                        }
                        ObjectProp::Spread(ex) => {
                            self.buf.push_str("...");
                            self.expr(ex);
                        }
                    }
                }
                self.buf.push_str(" }");
            }
            Expr::Assign { name, value, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str(" = ");
                self.expr(value);
            }
            Expr::TypeOf { operand, .. } => {
                self.buf.push_str("typeof ");
                self.expr(operand);
            }
            Expr::PostfixInc { name, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str("++");
            }
            Expr::PostfixDec { name, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str("--");
            }
            Expr::PrefixInc { name, .. } => {
                self.buf.push_str("++");
                self.buf.push_str(name.as_ref());
            }
            Expr::PrefixDec { name, .. } => {
                self.buf.push_str("--");
                self.buf.push_str(name.as_ref());
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str(compound(*op));
                self.expr(value);
            }
            Expr::LogicalAssign { name, op, value, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str(logical_assign(*op));
                self.expr(value);
            }
            Expr::MemberAssign {
                object,
                prop,
                value,
                ..
            } => {
                self.expr(object);
                self.buf.push('.');
                self.buf.push_str(prop.as_ref());
                self.buf.push_str(" = ");
                self.expr(value);
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.expr(object);
                self.buf.push('[');
                self.expr(index);
                self.buf.push_str("] = ");
                self.expr(value);
            }
            Expr::ArrowFunction { params, body, .. } => {
                self.buf.push('(');
                self.param_list(params, &None);
                self.buf.push_str(") => ");
                match body {
                    ArrowBody::Expr(e) => self.expr(e),
                    ArrowBody::Block(b) => self.stmt(b, 0),
                }
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                self.buf.push('`');
                for (i, q) in quasis.iter().enumerate() {
                    self.buf.push_str(&escape_template(q.as_ref()));
                    if i < exprs.len() {
                        self.buf.push_str("${");
                        self.expr(&exprs[i]);
                        self.buf.push('}');
                    }
                }
                self.buf.push('`');
            }
            Expr::Await { operand, .. } => {
                self.buf.push_str("await ");
                self.expr(operand);
            }
            Expr::JsxElement {
                tag,
                props,
                children,
                ..
            } => {
                self.buf.push('<');
                self.buf.push_str(tag.as_ref());
                for pr in props {
                    match pr {
                        JsxProp::Attr { name, value } => {
                            self.buf.push(' ');
                            self.buf.push_str(name.as_ref());
                            match value {
                                JsxAttrValue::String(s) => {
                                    self.buf.push('=');
                                    self.string_lit(s.as_ref());
                                }
                                JsxAttrValue::Expr(e) => {
                                    self.buf.push_str("={");
                                    self.expr(e);
                                    self.buf.push('}');
                                }
                                JsxAttrValue::ImplicitTrue => {}
                            }
                        }
                        JsxProp::Spread(e) => {
                            self.buf.push_str(" {...");
                            self.expr(e);
                            self.buf.push_str("} ");
                        }
                    }
                }
                if children.is_empty() {
                    self.buf.push_str(" />");
                } else {
                    self.buf.push('>');
                    for ch in children {
                        match ch {
                            JsxChild::Text(t) => self.buf.push_str(t.as_ref()),
                            JsxChild::Expr(e) => {
                                self.buf.push('{');
                                self.expr(e);
                                self.buf.push('}');
                            }
                        }
                    }
                    self.buf.push_str("</");
                    self.buf.push_str(tag.as_ref());
                    self.buf.push('>');
                }
            }
            Expr::JsxFragment { children, .. } => {
                self.buf.push_str("<>");
                for ch in children {
                    match ch {
                        JsxChild::Text(t) => self.buf.push_str(t.as_ref()),
                        JsxChild::Expr(e) => {
                            self.buf.push('{');
                            self.expr(e);
                            self.buf.push('}');
                        }
                    }
                }
                self.buf.push_str("</>");
            }
            Expr::NativeModuleLoad {
                spec,
                export_name,
                ..
            } => {
                self.buf.push_str("import { ");
                self.buf.push_str(export_name.as_ref());
                self.buf.push_str(" } from ");
                self.string_lit(spec.as_ref());
            }
        }
    }

    fn string_lit(&mut self, s: &str) {
        self.buf.push('"');
        for c in s.chars() {
            match c {
                '\\' => self.buf.push_str("\\\\"),
                '"' => self.buf.push_str("\\\""),
                '\n' => self.buf.push_str("\\n"),
                '\r' => self.buf.push_str("\\r"),
                '\t' => self.buf.push_str("\\t"),
                c if c.is_control() => self.buf.push_str(&format!("\\u{:04x}", c as u32)),
                c => self.buf.push(c),
            }
        }
        self.buf.push('"');
    }
}

fn escape_template(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$")
}

fn binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::StrictEq => "===",
        BinOp::StrictNe => "!==",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::In => "in",
    }
}

fn compound(op: CompoundOp) -> &'static str {
    match op {
        CompoundOp::Add => " += ",
        CompoundOp::Sub => " -= ",
        CompoundOp::Mul => " *= ",
        CompoundOp::Div => " /= ",
        CompoundOp::Mod => " %= ",
    }
}

fn logical_assign(op: LogicalAssignOp) -> &'static str {
    match op {
        LogicalAssignOp::AndAnd => " &&= ",
        LogicalAssignOp::OrOr => " ||= ",
        LogicalAssignOp::Nullish => " ??= ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple() {
        let src = "fn add(a, b) {\n  return a + b\n}\n";
        let out = format_source(src).unwrap();
        let _ = tishlang_parser::parse(&out).unwrap();
    }
}
