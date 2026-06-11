//! Recursive descent parser for Tish.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Macro to generate single-operator binary parsing functions.
/// Reduces boilerplate for Or, And, BitOr, BitXor, BitAnd parsers.
macro_rules! binary_single_op {
    ($name:ident, $next:ident, $token:ident, $op:expr) => {
        fn $name(&mut self) -> Result<Expr, String> {
            let mut left = self.$next()?;
            while matches!(self.peek_kind(), Some(TokenKind::$token)) {
                self.advance();
                let right = self.$next()?;
                let start = expr_span(&left).start;
                let end = expr_span(&right).end;
                left = Expr::Binary {
                    left: Box::new(left),
                    op: $op,
                    right: Box::new(right),
                    span: Span { start, end },
                };
            }
            Ok(left)
        }
    };
}

/// Macro for multi-operator binary parsing with a match block.
macro_rules! binary_multi_op {
    ($name:ident, $next:ident, $( $token:ident => $op:expr ),+ $(,)?) => {
        fn $name(&mut self) -> Result<Expr, String> {
            let mut left = self.$next()?;
            loop {
                let op = match self.peek_kind() {
                    $( Some(TokenKind::$token) => $op, )+
                    _ => break,
                };
                self.advance();
                let right = self.$next()?;
                let start = expr_span(&left).start;
                let end = expr_span(&right).end;
                left = Expr::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                    span: Span { start, end },
                };
            }
            Ok(left)
        }
    };
}

use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    DestructProp, ExportDeclaration, Expr, FunParam, ImportSpecifier, JsxAttrValue, JsxChild,
    JsxProp, Literal, LogicalAssignOp, MemberProp, ObjectProp, Program, Span, Statement,
    TypeAnnotation, TypeLiteral, TypedParam, UnaryOp,
};
use tishlang_lexer::{Token, TokenKind};

/// Mangle a generic instantiation `base<args>` into a Rust-ident-safe alias name (`Box__number`).
fn mangle_generic(base: &str, args: &[TypeAnnotation]) -> String {
    let parts: Vec<String> = args.iter().map(mangle_type).collect();
    format!("{}__{}", base, parts.join("_"))
}

fn mangle_type(t: &TypeAnnotation) -> String {
    match t {
        TypeAnnotation::Simple(s) => s.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect(),
        TypeAnnotation::Array(inner) => format!("{}Arr", mangle_type(inner)),
        TypeAnnotation::Tuple(es) => {
            format!("Tup{}", es.iter().map(mangle_type).collect::<Vec<_>>().join(""))
        }
        TypeAnnotation::Object(_) => "Obj".to_string(),
        TypeAnnotation::Union(_) => "Un".to_string(),
        TypeAnnotation::Intersection(_) => "Is".to_string(),
        TypeAnnotation::Function { .. } => "Fn".to_string(),
        TypeAnnotation::Literal(_) => "Lit".to_string(),
    }
}

/// Substitute generic type parameters with their concrete arguments throughout a type body.
fn subst_type(t: &TypeAnnotation, map: &HashMap<&str, &TypeAnnotation>) -> TypeAnnotation {
    match t {
        TypeAnnotation::Simple(s) => map
            .get(s.as_ref())
            .map(|rep| (*rep).clone())
            .unwrap_or_else(|| t.clone()),
        TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(subst_type(inner, map))),
        TypeAnnotation::Object(fields) => TypeAnnotation::Object(
            fields.iter().map(|(k, v)| (k.clone(), subst_type(v, map))).collect(),
        ),
        TypeAnnotation::Tuple(es) => {
            TypeAnnotation::Tuple(es.iter().map(|e| subst_type(e, map)).collect())
        }
        TypeAnnotation::Union(es) => {
            TypeAnnotation::Union(es.iter().map(|e| subst_type(e, map)).collect())
        }
        TypeAnnotation::Intersection(es) => {
            TypeAnnotation::Intersection(es.iter().map(|e| subst_type(e, map)).collect())
        }
        TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
            params: params.iter().map(|p| subst_type(p, map)).collect(),
            returns: Box::new(subst_type(returns, map)),
        },
        TypeAnnotation::Literal(_) => t.clone(),
    }
}

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Outstanding `>` owed to enclosing generic-arg lists after a `>>` (`Shr`) token was split,
    /// so nested `Array<Array<T>>` closes correctly (tish lexes `>>` as one token).
    gt_debt: u32,
    /// Generic `type`/`interface` decls (`type Box<T> = …`) → (type-param names, body), for
    /// monomorphizing a reference `Box<number>` into a concrete native struct.
    generic_aliases: HashMap<String, (Vec<Arc<str>>, TypeAnnotation)>,
    /// Synthetic specialized aliases (e.g. `type Box__number = { value: number }`) generated for
    /// each distinct `Generic<Args>` reference; appended to the program at the end.
    generic_specializations: Vec<Statement>,
    /// Names of specializations already generated (dedup).
    generic_done: HashSet<String>,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            gt_debt: 0,
            generic_aliases: HashMap::new(),
            generic_specializations: Vec::new(),
            generic_done: HashSet::new(),
        }
    }

    /// Close one generic-arg `>`: use a previously-split `>>` debt, consume a `>`, or split a
    /// `>>` (consuming it and owing one `>` to the parent). Returns false if there's no closer.
    fn try_close_angle(&mut self) -> bool {
        if self.gt_debt > 0 {
            self.gt_debt -= 1;
            return true;
        }
        match self.peek_kind() {
            Some(TokenKind::Gt) => {
                self.advance();
                true
            }
            Some(TokenKind::Shr) => {
                self.advance();
                self.gt_debt += 1;
                true
            }
            Some(TokenKind::UShr) => {
                // `>>>` closing three generic args, e.g. `Foo<Bar<Baz<T>>>`.
                self.advance();
                self.gt_debt += 2;
                true
            }
            _ => false,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|t| t.kind)
    }

    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, kind: TokenKind) -> Result<&Token, String> {
        let t = self
            .advance()
            .ok_or_else(|| format!("Expected {:?}, got EOF", kind))?;
        if t.kind == kind {
            Ok(t)
        } else {
            Err(format!(
                "Expected {:?}, got {:?} at {:?}",
                kind, t.kind, t.span
            ))
        }
    }

    /// After `.` / `?.`, allow contextual keywords as member names. In JS any `IdentifierName`
    /// (including reserved words) is a valid property name — `arr.of`, `x.as`, `o.in`, `o.type` —
    /// but the lexer emits dedicated keyword tokens for these, so they must be accepted explicitly
    /// here. (`TypedArray.of` is the motivating case.) See `docs/js-emit-philosophy.md`.
    fn expect_ident_or_type_member_name(&mut self) -> Result<&Token, String> {
        match self.peek_kind() {
            Some(TokenKind::Ident) => self.expect(TokenKind::Ident),
            Some(TokenKind::Type) => self.expect(TokenKind::Type),
            Some(TokenKind::Of) => self.expect(TokenKind::Of),
            Some(TokenKind::As) => self.expect(TokenKind::As),
            Some(TokenKind::In) => self.expect(TokenKind::In),
            other => Err(format!(
                "Expected property name after `.` or `?.`, got {:?}",
                other
            )),
        }
    }

    fn span_end(&self, start: (usize, usize)) -> Span {
        let end = self.peek().map(|t| t.span.start).unwrap_or(start);
        Span { start, end }
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut statements = Vec::with_capacity(8);
        while self.peek_kind().is_some() {
            if matches!(self.peek_kind(), Some(TokenKind::Dedent)) {
                self.advance();
                continue;
            }
            statements.push(self.parse_statement()?);
        }
        // Prepend the synthetic monomorphized aliases (`type Box__number = …`) so they're declared
        // before any use, for alias resolution + native struct emission.
        if !self.generic_specializations.is_empty() {
            let mut out = std::mem::take(&mut self.generic_specializations);
            out.append(&mut statements);
            statements = out;
        }
        Ok(Program { statements })
    }

    fn parse_statement(&mut self) -> Result<Statement, String> {
        let kind = self.peek_kind().ok_or("Unexpected EOF")?;
        let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));

        let stmt = match kind {
            TokenKind::LBrace | TokenKind::Indent => self.parse_block()?,
            TokenKind::Let => self.parse_var_decl(true)?,
            TokenKind::Const => self.parse_var_decl(false)?,
            TokenKind::Async => {
                self.advance(); // consume 'async'
                self.parse_fun_decl(true)? // parse_fun_decl expects 'fn' next
            }
            TokenKind::Fn => self.parse_fun_decl(false)?,
            TokenKind::If => self.parse_if()?,
            TokenKind::While => self.parse_while()?,
            TokenKind::For => self.parse_for()?,
            TokenKind::Return => self.parse_return()?,
            TokenKind::Switch => self.parse_switch()?,
            TokenKind::Do => self.parse_do_while()?,
            TokenKind::Throw => self.parse_throw()?,
            TokenKind::Try => self.parse_try()?,
            TokenKind::Break => {
                self.advance();
                let span_end = self.peek().map(|t| t.span.end).unwrap_or(span_start);
                Statement::Break {
                    span: Span {
                        start: span_start,
                        end: span_end,
                    },
                }
            }
            TokenKind::Continue => {
                self.advance();
                let span_end = self.peek().map(|t| t.span.end).unwrap_or(span_start);
                Statement::Continue {
                    span: Span {
                        start: span_start,
                        end: span_end,
                    },
                }
            }
            TokenKind::Import => self.parse_import()?,
            TokenKind::Export => self.parse_export()?,
            TokenKind::Type => self.parse_type_alias()?,
            TokenKind::Declare => self.parse_declare()?,
            TokenKind::Interface => self.parse_interface()?,
            _ => {
                let expr = self.parse_expr()?;
                let span_end = expr.span().end;
                Statement::ExprStmt {
                    expr,
                    span: Span {
                        start: span_start,
                        end: span_end,
                    },
                }
            }
        };

        // Optional semicolon
        if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
            self.advance();
        }

        Ok(stmt)
    }

    fn parse_block_or_statement(&mut self) -> Result<Statement, String> {
        if matches!(self.peek_kind(), Some(TokenKind::LBrace))
            || matches!(self.peek_kind(), Some(TokenKind::Indent))
        {
            return self.parse_block();
        }
        self.parse_statement()
    }

    fn parse_block(&mut self) -> Result<Statement, String> {
        let span_start = self.peek().ok_or("Unexpected EOF")?.span.start;

        let opened_with_brace = matches!(self.peek_kind(), Some(TokenKind::LBrace));
        if opened_with_brace {
            self.advance(); // {
                            // After `{`, the lexer often emits `Indent` for the first indented line of the body.
                            // `parse_statement` treats a leading `Indent` as starting a *nested* indent-block, so
                            // without consuming this token we get `Block { Block { let ... } ; ... }` and the first
                            // `let`/`const` is scoped too narrowly (JS ReferenceError). This indent is layout for
                            // *this* brace block, not an inner block.
            if matches!(self.peek_kind(), Some(TokenKind::Indent)) {
                self.advance();
            }
        } else if matches!(self.peek_kind(), Some(TokenKind::Indent)) {
            self.advance(); // Indent
        }

        let mut statements = Vec::with_capacity(4);
        loop {
            if matches!(self.peek_kind(), Some(TokenKind::RBrace))
                || matches!(self.peek_kind(), Some(TokenKind::Dedent))
                || self.peek_kind().is_none()
            {
                break;
            }
            statements.push(self.parse_statement()?);
        }

        if matches!(
            self.peek_kind(),
            Some(TokenKind::RBrace | TokenKind::Dedent)
        ) {
            self.advance();
        }

        let peek_end = self.peek().map(|x| x.span.end);
        let last_end = statements.last().map(|s| s.span().end);
        let end = match (peek_end, last_end) {
            (Some(p), Some(l)) => {
                if p.0 > l.0 || (p.0 == l.0 && p.1 > l.1) {
                    p
                } else {
                    l
                }
            }
            (Some(p), None) => p,
            (None, Some(l)) => l,
            (None, None) => span_start,
        };

        Ok(Statement::Block {
            statements,
            span: Span {
                start: span_start,
                end,
            },
        })
    }

    fn parse_var_decl(&mut self, mutable: bool) -> Result<Statement, String> {
        let span_start = if mutable {
            self.expect(TokenKind::Let)?.span.start
        } else {
            self.expect(TokenKind::Const)?.span.start
        };

        // First declarator keeps the `let`/`const`-anchored span (single-decl is
        // the overwhelmingly common case and stays byte-identical to before).
        let first = self.parse_one_declarator(mutable, span_start)?;
        if !matches!(self.peek_kind(), Some(TokenKind::Comma)) {
            return Ok(first);
        }

        // Comma-separated declarators: `let a = 1, b = 2, c`. Each lowers to its
        // own VarDecl inside a transparent `Multi` group (no new scope), so it
        // composes anywhere a single statement is expected (incl. `for` init).
        let mut statements = vec![first];
        while matches!(self.peek_kind(), Some(TokenKind::Comma)) {
            self.advance(); // consume ','
            let decl_start = self.peek().map(|t| t.span.start).unwrap_or(span_start);
            statements.push(self.parse_one_declarator(mutable, decl_start)?);
        }
        Ok(Statement::Multi {
            statements,
            span: self.span_end(span_start),
        })
    }

    /// Parse one declarator — `ident[: Type] [= init]` or `pattern = init` — into
    /// its own statement. The leading `let`/`const` keyword is consumed by the
    /// caller; `span_start` anchors this declarator's span.
    fn parse_one_declarator(
        &mut self,
        mutable: bool,
        span_start: (usize, usize),
    ) -> Result<Statement, String> {
        // Destructuring pattern declarator.
        if matches!(
            self.peek_kind(),
            Some(TokenKind::LBracket) | Some(TokenKind::LBrace)
        ) {
            let pattern = self.parse_destruct_pattern()?;
            self.expect(TokenKind::Assign)?;
            let init = self.parse_expr()?;
            return Ok(Statement::VarDeclDestructure {
                pattern,
                mutable,
                init,
                span: self.span_end(span_start),
            });
        }

        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected identifier")?;

        // Optional type annotation: `: Type`
        let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
            self.advance(); // consume :
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        let init = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Statement::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init,
            span: self.span_end(span_start),
        })
    }

    fn parse_destruct_pattern(&mut self) -> Result<DestructPattern, String> {
        match self.peek_kind() {
            Some(TokenKind::LBracket) => self.parse_array_destruct_pattern(),
            Some(TokenKind::LBrace) => self.parse_object_destruct_pattern(),
            _ => Err("Expected destructuring pattern".to_string()),
        }
    }

    fn parse_array_destruct_pattern(&mut self) -> Result<DestructPattern, String> {
        self.expect(TokenKind::LBracket)?;
        let mut elements = Vec::new();

        while !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
            // Handle holes (elision): [a, , b]
            if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                elements.push(None);
                self.advance();
                continue;
            }

            // Rest element: ...rest
            if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                self.advance();
                let name_tok = self.expect(TokenKind::Ident)?;
                let name_span = Span {
                    start: name_tok.span.start,
                    end: name_tok.span.end,
                };
                let name = name_tok.literal.clone().ok_or("Expected identifier")?;
                elements.push(Some(DestructElement::Rest(name, name_span)));
                break;
            }

            // Nested pattern or identifier
            let elem = match self.peek_kind() {
                Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    let nested = self.parse_destruct_pattern()?;
                    DestructElement::Pattern(Box::new(nested))
                }
                Some(TokenKind::Ident) => {
                    let name_tok = self.advance().ok_or("Unexpected EOF")?;
                    let name_span = Span {
                        start: name_tok.span.start,
                        end: name_tok.span.end,
                    };
                    let name = name_tok.literal.clone().ok_or("Expected identifier")?;
                    DestructElement::Ident(name, name_span)
                }
                _ => return Err("Expected identifier or pattern in destructuring".to_string()),
            };
            elements.push(Some(elem));

            if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(TokenKind::RBracket)?;
        Ok(DestructPattern::Array(elements))
    }

    fn parse_object_destruct_pattern(&mut self) -> Result<DestructPattern, String> {
        self.expect(TokenKind::LBrace)?;
        let mut props = Vec::new();

        while !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
            let key_tok = self.expect(TokenKind::Ident)?;
            let key_span = Span {
                start: key_tok.span.start,
                end: key_tok.span.end,
            };
            let key = key_tok.literal.clone().ok_or("Expected identifier")?;

            let value = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                self.advance();
                // Could be renamed binding or nested pattern
                match self.peek_kind() {
                    Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                        let nested = self.parse_destruct_pattern()?;
                        DestructElement::Pattern(Box::new(nested))
                    }
                    Some(TokenKind::Ident) => {
                        let name_tok = self.advance().ok_or("Unexpected EOF")?;
                        let name_span = Span {
                            start: name_tok.span.start,
                            end: name_tok.span.end,
                        };
                        let name = name_tok.literal.clone().ok_or("Expected identifier")?;
                        DestructElement::Ident(name, name_span)
                    }
                    _ => return Err("Expected identifier or pattern after ':'".to_string()),
                }
            } else {
                // Shorthand: { key } is equivalent to { key: key }
                DestructElement::Ident(key.clone(), key_span)
            };

            props.push(DestructProp { key, value });

            if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(TokenKind::RBrace)?;
        Ok(DestructPattern::Object(props))
    }

    /// One formal parameter: `name`, `name: T`, `name = expr`, or a destructuring pattern.
    fn parse_fun_param(&mut self) -> Result<FunParam, String> {
        if matches!(
            self.peek_kind(),
            Some(TokenKind::LBracket | TokenKind::LBrace)
        ) {
            let pattern = self.parse_destruct_pattern()?;
            let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                self.advance();
                Some(self.parse_type_annotation()?)
            } else {
                None
            };
            let default = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            return Ok(FunParam::Destructure {
                pattern,
                type_ann,
                default,
            });
        }
        let param_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: param_tok.span.start,
            end: param_tok.span.end,
        };
        let param_name = param_tok.literal.clone().ok_or("Expected param name")?;
        let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type_annotation()?)
        } else {
            None
        };
        let default = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(FunParam::Simple(TypedParam {
            name: param_name,
            name_span,
            type_ann,
            default,
        }))
    }

    /// Parse a generic type-parameter list `<T, U, …>` on a `fn` / `type` declaration, returning the
    /// parameter names. On functions the names are ignored (generic fns run gradually/boxed); on
    /// `type`/`interface` they drive struct monomorphization (`Box<number>` → a native struct).
    fn parse_type_params(&mut self) -> Result<Vec<Arc<str>>, String> {
        let mut params = Vec::new();
        if matches!(self.peek_kind(), Some(TokenKind::Lt)) {
            self.advance(); // <
            while !matches!(self.peek_kind(), Some(TokenKind::Gt)) {
                let tok = self.expect(TokenKind::Ident)?;
                if let Some(n) = &tok.literal {
                    params.push(Arc::from(n.as_ref()));
                }
                if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::Gt)?; // >
        }
        Ok(params)
    }

    /// Specialize a generic struct reference `base<args>` into a concrete synthetic alias name
    /// (e.g. `Box__number`), emitting `type Box__number = { value: number }` once per instantiation
    /// so it lowers to a native struct. Returns `None` if `base` isn't a known generic alias or the
    /// arity mismatches (caller then falls back to erasing the args).
    fn monomorphize_generic(&mut self, base: &str, args: &[TypeAnnotation]) -> Option<String> {
        let (params, body) = self.generic_aliases.get(base)?.clone();
        if params.len() != args.len() {
            return None;
        }
        let spec_name = mangle_generic(base, args);
        if self.generic_done.insert(spec_name.clone()) {
            let subst: HashMap<&str, &TypeAnnotation> =
                params.iter().map(|p| p.as_ref()).zip(args.iter()).collect();
            let concrete = subst_type(&body, &subst);
            let z = Span {
                start: (0, 0),
                end: (0, 0),
            };
            self.generic_specializations.push(Statement::TypeAlias {
                name: Arc::from(spec_name.as_str()),
                name_span: z,
                ty: concrete,
                span: z,
            });
        }
        Some(spec_name)
    }

    /// Parse a generic type-argument list `<T, U, …>` on a type reference (`Array<number>`,
    /// `Map<string, number>`). Nested `Array<Array<T>>` works because tish has no `>>` token.
    fn parse_type_args(&mut self) -> Result<Vec<TypeAnnotation>, String> {
        self.expect(TokenKind::Lt)?; // <
        let mut args = Vec::new();
        if !self.try_close_angle() {
            loop {
                args.push(self.parse_type_annotation()?);
                if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                    continue;
                }
                break;
            }
            if !self.try_close_angle() {
                return Err("expected `>` to close type arguments".to_string());
            }
        }
        Ok(args)
    }

    /// Parse a type annotation (number, string, T[], T?, {a: T}, A | B, A & B, Array<T>, etc.)
    fn parse_type_annotation(&mut self) -> Result<TypeAnnotation, String> {
        let base = self.parse_type_intersection()?;

        // Union: T | U | ... (binds looser than `&`)
        if matches!(self.peek_kind(), Some(TokenKind::BitOr)) {
            let mut types = vec![base];
            while matches!(self.peek_kind(), Some(TokenKind::BitOr)) {
                self.advance(); // |
                types.push(self.parse_type_intersection()?);
            }
            return Ok(TypeAnnotation::Union(types));
        }

        Ok(base)
    }

    /// `A & B & …` intersection (binds tighter than `|`).
    fn parse_type_intersection(&mut self) -> Result<TypeAnnotation, String> {
        let base = self.parse_type_postfix()?;
        if matches!(self.peek_kind(), Some(TokenKind::BitAnd)) {
            let mut types = vec![base];
            while matches!(self.peek_kind(), Some(TokenKind::BitAnd)) {
                self.advance(); // &
                types.push(self.parse_type_postfix()?);
            }
            return Ok(TypeAnnotation::Intersection(types));
        }
        Ok(base)
    }

    /// A primary type plus any postfix `[]` (array) and `?` (optional, `T? === T | null`),
    /// chained: `T[]`, `T[][]`, `T?`, `T?[]`, …
    fn parse_type_postfix(&mut self) -> Result<TypeAnnotation, String> {
        let mut t = self.parse_type_primary()?;
        loop {
            match self.peek_kind() {
                Some(TokenKind::LBracket) => {
                    self.advance(); // [
                    self.expect(TokenKind::RBracket)?; // ]
                    t = TypeAnnotation::Array(Box::new(t));
                }
                Some(TokenKind::Question) => {
                    self.advance(); // ?
                    t = TypeAnnotation::Union(vec![t, TypeAnnotation::Simple("null".into())]);
                }
                _ => break,
            }
        }
        Ok(t)
    }

    /// Parse a primary type (identifier, object, or function type)
    fn parse_type_primary(&mut self) -> Result<TypeAnnotation, String> {
        match self.peek_kind() {
            Some(TokenKind::Ident) => {
                let tok = self.advance().ok_or("Expected type name")?;
                let name = tok.literal.clone().ok_or("Expected type name")?;
                // Generic reference `Name<Args>`: `Array<T>` desugars to the native `T[]`; other
                // generic refs erase their args (the base name resolves to its alias, whose type
                // params already act as unknown -> `Value`).
                if matches!(self.peek_kind(), Some(TokenKind::Lt)) {
                    let args = self.parse_type_args()?;
                    if name.as_ref() == "Array" && args.len() == 1 {
                        return Ok(TypeAnnotation::Array(Box::new(
                            args.into_iter().next().unwrap(),
                        )));
                    }
                    // Monomorphize a generic struct ref `Box<number>` into a synthetic concrete
                    // alias `Box__number` (a native struct). Falls back to erasing the args when
                    // `name` isn't a known generic alias (e.g. forward reference) or arity mismatches.
                    if let Some(spec) = self.monomorphize_generic(name.as_ref(), &args) {
                        return Ok(TypeAnnotation::Simple(Arc::from(spec.as_str())));
                    }
                    return Ok(TypeAnnotation::Simple(name));
                }
                Ok(TypeAnnotation::Simple(name))
            }
            Some(TokenKind::Type | TokenKind::Declare) => {
                let tok = self.advance().ok_or("Expected type name")?;
                let name = tok.literal.clone().ok_or("Expected type name")?;
                Ok(TypeAnnotation::Simple(name))
            }
            // Handle keywords that can be type names
            Some(TokenKind::Null) => {
                self.advance();
                Ok(TypeAnnotation::Simple("null".into()))
            }
            Some(TokenKind::Void) => {
                self.advance();
                Ok(TypeAnnotation::Simple("void".into()))
            }
            Some(TokenKind::LBrace) => {
                // Object type: { key: Type, ... }
                self.advance(); // {
                let mut props = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                    // `for` is a keyword but a common method name (`Symbol.for`); allow it here.
                    let key: Arc<str> = match self.peek_kind() {
                        Some(TokenKind::Ident) => {
                            let tok = self.expect(TokenKind::Ident)?;
                            Arc::from(
                                tok.literal
                                    .as_deref()
                                    .ok_or("Expected property name")?,
                            )
                        }
                        Some(TokenKind::For) => {
                            self.advance();
                            Arc::from("for")
                        }
                        _ => {
                            return Err(format!(
                                "Expected Ident or `for` as object type property name, got {:?}",
                                self.peek_kind()
                            ));
                        }
                    };
                    self.expect(TokenKind::Colon)?;
                    let typ = self.parse_type_annotation()?;
                    props.push((key, typ));
                    if !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                        // Accept `,` or `;` between items (TypeScript-style
                        // semicolons are common in interface/object type
                        // declarations); also tolerate a trailing separator.
                        if matches!(
                            self.peek_kind(),
                            Some(TokenKind::Comma) | Some(TokenKind::Semicolon)
                        ) {
                            self.advance();
                        }
                    }
                }
                self.expect(TokenKind::RBrace)?;
                Ok(TypeAnnotation::Object(props))
            }
            Some(TokenKind::LParen) => {
                // Function type: (T1, T2) => R
                self.advance(); // (
                let mut params = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    params.push(self.parse_type_annotation()?);
                    if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                        self.expect(TokenKind::Comma)?;
                    }
                }
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::Arrow)?;
                let returns = self.parse_type_annotation()?;
                Ok(TypeAnnotation::Function {
                    params,
                    returns: Box::new(returns),
                })
            }
            // Tuple type: [T1, T2, ...]
            Some(TokenKind::LBracket) => {
                self.advance(); // [
                let mut elems = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
                    elems.push(self.parse_type_annotation()?);
                    if !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
                        self.expect(TokenKind::Comma)?;
                    }
                }
                self.expect(TokenKind::RBracket)?;
                Ok(TypeAnnotation::Tuple(elems))
            }
            // Literal types: "foo", 42, true, false
            Some(TokenKind::String) => {
                let tok = self.advance().ok_or("Expected string literal type")?;
                let s = tok.literal.clone().ok_or("Expected string literal")?;
                Ok(TypeAnnotation::Literal(TypeLiteral::Str(s)))
            }
            Some(TokenKind::Number) => {
                let tok = self.advance().ok_or("Expected number literal type")?;
                let s = tok.literal.as_ref().ok_or("Expected number literal")?;
                let n: f64 = s.parse().map_err(|_| format!("Invalid number: {}", s))?;
                Ok(TypeAnnotation::Literal(TypeLiteral::Num(n)))
            }
            Some(TokenKind::True) => {
                self.advance();
                Ok(TypeAnnotation::Literal(TypeLiteral::Bool(true)))
            }
            Some(TokenKind::False) => {
                self.advance();
                Ok(TypeAnnotation::Literal(TypeLiteral::Bool(false)))
            }
            _ => Err("Expected type annotation".to_string()),
        }
    }

    fn parse_fun_decl(&mut self, async_: bool) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Fn)?.span.start;
        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected function name")?;
        self.parse_type_params()?; // generic `fn f<T, U>(…)` — fn type params run gradually (boxed)
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::with_capacity(4);
        let mut rest_param = None;
        while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                self.advance();
                let rest_tok = self.expect(TokenKind::Ident)?;
                let rest_name_span = Span {
                    start: rest_tok.span.start,
                    end: rest_tok.span.end,
                };
                let param_name = rest_tok.literal.clone().ok_or("Expected rest param name")?;
                // Optional type annotation for rest param
                let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                    self.advance();
                    Some(self.parse_type_annotation()?)
                } else {
                    None
                };
                rest_param = Some(TypedParam {
                    name: param_name,
                    name_span: rest_name_span,
                    type_ann,
                    default: None,
                });
                if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    return Err("Rest parameter must be last".to_string());
                }
                break;
            }
            params.push(self.parse_fun_param()?);
            if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                self.expect(TokenKind::Comma)?;
            }
        }
        self.expect(TokenKind::RParen)?;

        // Optional return type: `: Type`
        let return_type = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        let body = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            self.advance(); // =
            let expr = self.parse_expr()?;
            let span = expr.span();
            Box::new(Statement::Block {
                statements: vec![Statement::Return {
                    value: Some(expr),
                    span,
                }],
                span,
            })
        } else {
            Box::new(self.parse_block()?)
        };

        // Span must cover the whole declaration through the body. `peek().start` alone can sit on
        // the opening `{` (same as `span_start` at EOF) or otherwise truncate before inner spans.
        let peek_start = self.peek().map(|t| t.span.start).unwrap_or(span_start);
        let body_end = body.as_ref().span().end;
        let end = if peek_start.0 > body_end.0
            || (peek_start.0 == body_end.0 && peek_start.1 > body_end.1)
        {
            peek_start
        } else {
            body_end
        };

        Ok(Statement::FunDecl {
            async_,
            name,
            name_span,
            params,
            rest_param,
            return_type,
            body,
            span: Span {
                start: span_start,
                end,
            },
        })
    }

    fn parse_type_alias(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Type)?.span.start;
        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected type alias name")?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::Assign)?;
        let ty = self.parse_type_annotation()?;
        if !type_params.is_empty() {
            self.generic_aliases
                .insert(name.to_string(), (type_params, ty.clone()));
        }
        Ok(Statement::TypeAlias {
            name,
            name_span,
            ty,
            span: self.span_end(span_start),
        })
    }

    /// `interface Name { k: T, ... }` — desugared to `type Name = { ... }` so the checker
    /// (structural matching) and codegen (native `TishStruct_*`) treat it exactly like an
    /// object-type alias. (`extends` is not yet supported.)
    fn parse_interface(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Interface)?.span.start;
        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected interface name")?;
        let type_params = self.parse_type_params()?;

        // `extends Parent1, Parent2` — desugar to an intersection of the parents with the body, so
        // the inherited fields participate in structural checking + native struct emission.
        let mut parents: Vec<TypeAnnotation> = Vec::new();
        if matches!(self.peek_kind(), Some(TokenKind::Ident))
            && self.peek().and_then(|t| t.literal.as_deref()) == Some("extends")
        {
            self.advance(); // extends
            loop {
                parents.push(self.parse_type_postfix()?);
                if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                    continue;
                }
                break;
            }
        }

        let body = self.parse_type_annotation()?;
        let ty = if parents.is_empty() {
            body
        } else {
            parents.push(body);
            TypeAnnotation::Intersection(parents)
        };
        if !type_params.is_empty() {
            self.generic_aliases
                .insert(name.to_string(), (type_params, ty.clone()));
        }
        Ok(Statement::TypeAlias {
            name,
            name_span,
            ty,
            span: self.span_end(span_start),
        })
    }

    fn parse_declare(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Declare)?.span.start;
        let async_ = if matches!(self.peek_kind(), Some(TokenKind::Async)) {
            self.advance();
            true
        } else {
            false
        };
        if matches!(self.peek_kind(), Some(TokenKind::Fn)) {
            return self.parse_declare_fun(span_start, async_);
        }
        let const_ = match self.peek_kind() {
            Some(TokenKind::Let) => {
                self.advance();
                false
            }
            Some(TokenKind::Const) => {
                self.advance();
                true
            }
            _ => {
                return Err(
                    "Expected `let`, `const`, `async fn`, or `fn` after `declare`".to_string(),
                );
            }
        };
        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected identifier")?;
        let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type_annotation()?)
        } else {
            None
        };
        if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            return Err("`declare` cannot have an initializer".to_string());
        }
        Ok(Statement::DeclareVar {
            name,
            name_span,
            type_ann,
            const_,
            span: self.span_end(span_start),
        })
    }

    fn parse_declare_fun(
        &mut self,
        span_start: (usize, usize),
        async_: bool,
    ) -> Result<Statement, String> {
        self.expect(TokenKind::Fn)?;
        let name_tok = self.expect(TokenKind::Ident)?;
        let name_span = Span {
            start: name_tok.span.start,
            end: name_tok.span.end,
        };
        let name = name_tok.literal.clone().ok_or("Expected function name")?;
        self.parse_type_params()?; // generic `fn f<T, U>(…)` — fn type params run gradually (boxed)
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::with_capacity(4);
        let mut rest_param = None;
        while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                self.advance();
                let rest_tok = self.expect(TokenKind::Ident)?;
                let rest_name_span = Span {
                    start: rest_tok.span.start,
                    end: rest_tok.span.end,
                };
                let param_name = rest_tok.literal.clone().ok_or("Expected rest param name")?;
                let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                    self.advance();
                    Some(self.parse_type_annotation()?)
                } else {
                    None
                };
                rest_param = Some(TypedParam {
                    name: param_name,
                    name_span: rest_name_span,
                    type_ann,
                    default: None,
                });
                if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    return Err("Rest parameter must be last".to_string());
                }
                break;
            }
            params.push(self.parse_fun_param()?);
            if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                self.expect(TokenKind::Comma)?;
            }
        }
        self.expect(TokenKind::RParen)?;
        let return_type = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type_annotation()?)
        } else {
            None
        };
        if matches!(
            self.peek_kind(),
            Some(TokenKind::Assign | TokenKind::LBrace | TokenKind::Indent)
        ) {
            return Err("`declare function` must not have a body".to_string());
        }
        if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
            self.advance();
        }
        Ok(Statement::DeclareFun {
            async_,
            name,
            name_span,
            params,
            rest_param,
            return_type,
            span: self.span_end(span_start),
        })
    }

    fn parse_if(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::If)?.span.start;
        self.expect(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        let then_branch = Box::new(self.parse_block_or_statement()?);
        let else_branch = if matches!(self.peek_kind(), Some(TokenKind::Else)) {
            self.advance();
            Some(Box::new(self.parse_block_or_statement()?))
        } else {
            None
        };
        Ok(Statement::If {
            cond,
            then_branch,
            else_branch,
            span: self.span_end(span_start),
        })
    }

    fn parse_while(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::While)?.span.start;
        self.expect(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        let body = Box::new(self.parse_block_or_statement()?);
        Ok(Statement::While {
            cond,
            body,
            span: self.span_end(span_start),
        })
    }

    fn parse_for(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::For)?.span.start;
        self.expect(TokenKind::LParen)?;
        let init = if matches!(self.peek_kind(), Some(TokenKind::Let | TokenKind::Const)) {
            let mutable = matches!(self.peek_kind(), Some(TokenKind::Let));
            let var_span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
            self.advance();
            let for_name_tok = self.expect(TokenKind::Ident)?;
            let name_span = Span {
                start: for_name_tok.span.start,
                end: for_name_tok.span.end,
            };
            let name = for_name_tok.literal.clone().ok_or("Expected identifier")?;
            if matches!(self.peek_kind(), Some(TokenKind::Of)) {
                self.advance();
                let iterable = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                let body = Box::new(self.parse_block_or_statement()?);
                return Ok(Statement::ForOf {
                    name,
                    name_span,
                    iterable,
                    body,
                    span: self.span_end(span_start),
                });
            }
            let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                self.advance();
                Some(self.parse_type_annotation()?)
            } else {
                None
            };
            let init_expr = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            let first = Statement::VarDecl {
                name,
                name_span,
                mutable,
                type_ann,
                init: init_expr,
                span: self.span_end(var_span_start),
            };
            // Comma-separated for-init declarators: `for (let i = 0, n = len; ...)`.
            let decl = if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                let mut statements = vec![first];
                while matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                    let decl_start = self.peek().map(|t| t.span.start).unwrap_or(var_span_start);
                    statements.push(self.parse_one_declarator(mutable, decl_start)?);
                }
                Statement::Multi {
                    statements,
                    span: self.span_end(var_span_start),
                }
            } else {
                first
            };
            if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
                self.advance();
            }
            Some(Box::new(decl))
        } else if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
            None
        } else {
            let st = self.peek().ok_or("Unexpected EOF in for init")?;
            let span_start = st.span.start;
            Some(Box::new(Statement::ExprStmt {
                expr: self.parse_expr()?,
                span: Span {
                    start: span_start,
                    end: self.peek().map(|x| x.span.end).unwrap_or(span_start),
                },
            }))
        };
        if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
            self.advance();
        }
        let cond = if matches!(
            self.peek_kind(),
            Some(TokenKind::Semicolon | TokenKind::RParen)
        ) {
            None
        } else {
            let c = self.parse_expr()?;
            self.expect(TokenKind::Semicolon)?;
            Some(c)
        };
        // `for (init; ; update)` — when the condition is empty we matched `;` above but did not
        // consume it; skip it so `update` / `)` parse correctly (e.g. `for (;;)`).
        if cond.is_none() && matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
            self.advance();
        }
        let update = if matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            None
        } else {
            let u = self.parse_expr()?;
            Some(u)
        };
        self.expect(TokenKind::RParen)?;
        let body = Box::new(self.parse_block_or_statement()?);
        Ok(Statement::For {
            init,
            cond,
            update,
            body,
            span: self.span_end(span_start),
        })
    }

    fn parse_return(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Return)?.span.start;
        let value = if matches!(self.peek_kind(), Some(TokenKind::Semicolon))
            || matches!(self.peek_kind(), Some(TokenKind::Dedent))
            || matches!(self.peek_kind(), Some(TokenKind::RBrace))
            || self.peek_kind().is_none()
        {
            None
        } else {
            Some(self.parse_expr()?)
        };
        Ok(Statement::Return {
            value,
            span: self.span_end(span_start),
        })
    }

    fn parse_switch(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Switch)?.span.start;
        self.expect(TokenKind::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::LBrace)?;
        let mut cases = Vec::new();
        let mut default_body = None;
        loop {
            if matches!(self.peek_kind(), Some(TokenKind::Case)) {
                self.advance();
                let case_expr = self.parse_expr()?;
                self.expect(TokenKind::Colon)?;
                let mut body = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::Case))
                    && !matches!(self.peek_kind(), Some(TokenKind::Default))
                    && !matches!(self.peek_kind(), Some(TokenKind::RBrace))
                    && self.peek_kind().is_some()
                {
                    body.push(self.parse_statement()?);
                }
                cases.push((Some(case_expr), body));
            } else if matches!(self.peek_kind(), Some(TokenKind::Default)) {
                self.advance();
                self.expect(TokenKind::Colon)?;
                let mut body = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBrace))
                    && self.peek_kind().is_some()
                {
                    body.push(self.parse_statement()?);
                }
                default_body = Some(body);
                break;
            } else if matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                break;
            } else {
                return Err("Expected case or default in switch".to_string());
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Statement::Switch {
            expr,
            cases,
            default_body,
            span: self.span_end(span_start),
        })
    }

    fn parse_do_while(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Do)?.span.start;
        let body = Box::new(self.parse_block_or_statement()?);
        self.expect(TokenKind::While)?;
        self.expect(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Statement::DoWhile {
            body,
            cond,
            span: self.span_end(span_start),
        })
    }

    fn parse_throw(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Throw)?.span.start;
        let value = self.parse_expr()?;
        Ok(Statement::Throw {
            value,
            span: self.span_end(span_start),
        })
    }

    fn parse_try(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Try)?.span.start;
        let body = Box::new(self.parse_block_or_statement()?);

        let mut catch_param = None;
        let mut catch_param_span = None;
        let mut catch_body = None;
        let mut finally_body = None;

        if matches!(self.peek_kind(), Some(TokenKind::Catch)) {
            self.advance();
            self.expect(TokenKind::LParen)?;
            let catch_tok = self.expect(TokenKind::Ident)?;
            catch_param_span = Some(Span {
                start: catch_tok.span.start,
                end: catch_tok.span.end,
            });
            catch_param = catch_tok.literal.clone();
            self.expect(TokenKind::RParen)?;
            catch_body = Some(Box::new(self.parse_block_or_statement()?));
        }

        if matches!(self.peek_kind(), Some(TokenKind::Finally)) {
            self.advance();
            finally_body = Some(Box::new(self.parse_block_or_statement()?));
        }

        if catch_body.is_none() && finally_body.is_none() {
            return Err("try statement requires catch or finally".to_string());
        }

        Ok(Statement::Try {
            body,
            catch_param,
            catch_param_span,
            catch_body,
            finally_body,
            span: self.span_end(span_start),
        })
    }

    fn parse_import(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Import)?.span.start;
        let specifiers = if matches!(self.peek_kind(), Some(TokenKind::LBrace)) {
            // Named: import { a, b as c } from "..."
            self.advance();
            let mut specs = Vec::new();
            while !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                let name_tok = self.expect(TokenKind::Ident)?;
                let name_span = Span {
                    start: name_tok.span.start,
                    end: name_tok.span.end,
                };
                let name = name_tok
                    .literal
                    .clone()
                    .ok_or("Expected identifier in import")?;
                // `as` is a dedicated keyword token (also used for casts); in import-specifier
                // position it introduces a rename: `{ foo as bar }`.
                let (alias, alias_span) = if matches!(self.peek_kind(), Some(TokenKind::As)) {
                    self.advance(); // consume 'as'
                    let alias_tok = self.expect(TokenKind::Ident)?;
                    let asp = Span {
                        start: alias_tok.span.start,
                        end: alias_tok.span.end,
                    };
                    (
                        Some(
                            alias_tok
                                .literal
                                .clone()
                                .ok_or("Expected alias after 'as'")?,
                        ),
                        Some(asp),
                    )
                } else {
                    (None, None)
                };
                specs.push(ImportSpecifier::Named {
                    name,
                    name_span,
                    alias,
                    alias_span,
                });
                if !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RBrace)?;
            specs
        } else if matches!(self.peek_kind(), Some(TokenKind::Star)) {
            // Namespace: import * as M from "..."
            self.advance();
            self.expect(TokenKind::As)?;
            let alias_tok = self.expect(TokenKind::Ident)?;
            let name_span = Span {
                start: alias_tok.span.start,
                end: alias_tok.span.end,
            };
            let alias = alias_tok
                .literal
                .clone()
                .ok_or("Expected identifier after 'as'")?;
            vec![ImportSpecifier::Namespace {
                name: alias,
                name_span,
            }]
        } else if matches!(self.peek_kind(), Some(TokenKind::Ident)) {
            // Default: import X from "..."
            let def_tok = self.expect(TokenKind::Ident)?;
            let name_span = Span {
                start: def_tok.span.start,
                end: def_tok.span.end,
            };
            let name = def_tok.literal.clone().ok_or("Expected identifier")?;
            vec![ImportSpecifier::Default { name, name_span }]
        } else {
            return Err("Expected { }, * as name, or default import".to_string());
        };
        let from_tok = self.expect(TokenKind::Ident)?;
        if from_tok.literal.as_deref() != Some("from") {
            return Err("Expected 'from' in import statement".to_string());
        }
        let from = self
            .expect(TokenKind::String)?
            .literal
            .clone()
            .ok_or("Expected string path in import")?;
        Ok(Statement::Import {
            specifiers,
            from,
            span: self.span_end(span_start),
        })
    }

    fn parse_export(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Export)?.span.start;
        let declaration = if matches!(self.peek_kind(), Some(TokenKind::Default)) {
            self.advance();
            let expr = self.parse_expr()?;
            ExportDeclaration::Default(expr)
        } else if matches!(self.peek_kind(), Some(TokenKind::Type)) {
            ExportDeclaration::Named(Box::new(self.parse_type_alias()?))
        } else if matches!(self.peek_kind(), Some(TokenKind::Declare)) {
            ExportDeclaration::Named(Box::new(self.parse_declare()?))
        } else if matches!(self.peek_kind(), Some(TokenKind::Const)) {
            ExportDeclaration::Named(Box::new(self.parse_var_decl(false)?))
        } else if matches!(self.peek_kind(), Some(TokenKind::Let)) {
            ExportDeclaration::Named(Box::new(self.parse_var_decl(true)?))
        } else if matches!(self.peek_kind(), Some(TokenKind::Async))
            || matches!(self.peek_kind(), Some(TokenKind::Fn))
        {
            let async_ = matches!(self.peek_kind(), Some(TokenKind::Async));
            if async_ {
                self.advance();
            }
            ExportDeclaration::Named(Box::new(self.parse_fun_decl(async_)?))
        } else {
            return Err(
                "Expected 'default', 'type', 'declare', 'const', 'let', or 'fn' after export"
                    .to_string(),
            );
        };
        Ok(Statement::Export {
            declaration: Box::new(declaration),
            span: self.span_end(span_start),
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        let left = self.parse_conditional()?;

        // Check for simple assignment
        if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            let start = left.span().start;

            // Variable assignment: x = val
            if let Expr::Ident { name, .. } = &left {
                let name = Arc::clone(name);
                self.advance(); // =
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::Assign {
                    name,
                    value: Box::new(value),
                    span: Span { start, end },
                });
            }

            // Member assignment: obj.prop = val
            if let Expr::Member {
                object,
                prop: MemberProp::Name {
                    name: prop_name, ..
                },
                ..
            } = &left
            {
                let object = Box::clone(object);
                let prop = Arc::clone(prop_name);
                self.advance(); // =
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::MemberAssign {
                    object,
                    prop,
                    value: Box::new(value),
                    span: Span { start, end },
                });
            }

            // Index assignment: arr[idx] = val or obj[key] = val
            if let Expr::Index { object, index, .. } = &left {
                let object = Box::clone(object);
                let index = Box::clone(index);
                self.advance(); // =
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::IndexAssign {
                    object,
                    index,
                    value: Box::new(value),
                    span: Span { start, end },
                });
            }
        }

        // Check for compound assignment (+=, -=, *=, /=, %=)
        let compound_op = match self.peek_kind() {
            Some(TokenKind::PlusAssign) => Some(CompoundOp::Add),
            Some(TokenKind::MinusAssign) => Some(CompoundOp::Sub),
            Some(TokenKind::StarAssign) => Some(CompoundOp::Mul),
            Some(TokenKind::SlashAssign) => Some(CompoundOp::Div),
            Some(TokenKind::PercentAssign) => Some(CompoundOp::Mod),
            _ => None,
        };

        if let Some(op) = compound_op {
            if let Expr::Ident { name, span } = &left {
                let name = Arc::clone(name);
                let start = span.start;
                self.advance(); // consume the compound operator
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::CompoundAssign {
                    name,
                    op,
                    value: Box::new(value),
                    span: Span { start, end },
                });
            }
        }

        // Check for logical assignment (&&=, ||=, ??=)
        let logical_op = match self.peek_kind() {
            Some(TokenKind::AndAndAssign) => Some(LogicalAssignOp::AndAnd),
            Some(TokenKind::OrOrAssign) => Some(LogicalAssignOp::OrOr),
            Some(TokenKind::NullishAssign) => Some(LogicalAssignOp::Nullish),
            _ => None,
        };

        if let Some(op) = logical_op {
            if let Expr::Ident { name, span } = &left {
                let name = Arc::clone(name);
                let start = span.start;
                self.advance(); // consume the logical assign operator
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::LogicalAssign {
                    name,
                    op,
                    value: Box::new(value),
                    span: Span { start, end },
                });
            }
        }

        Ok(left)
    }

    fn parse_conditional(&mut self) -> Result<Expr, String> {
        let cond = self.parse_nullish_coalesce()?;
        if matches!(self.peek_kind(), Some(TokenKind::Question)) {
            let start = cond.span().start;
            self.advance(); // ?
            let then_branch = self.parse_expr()?;
            self.expect(TokenKind::Colon)?;
            let else_branch = self.parse_conditional()?; // right-associative
            let end = else_branch.span().end;
            Ok(Expr::Conditional {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
                span: Span { start, end },
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_nullish_coalesce(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_or()?;
        while matches!(self.peek_kind(), Some(TokenKind::NullishCoalesce)) {
            self.advance();
            let right = self.parse_or()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::NullishCoalesce {
                left: Box::new(left),
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    // Binary operators generated by macros to reduce duplication
    binary_single_op!(parse_or, parse_and, Or, BinOp::Or);
    binary_single_op!(parse_and, parse_bit_or, And, BinOp::And);
    binary_single_op!(parse_bit_or, parse_bit_xor, BitOr, BinOp::BitOr);
    binary_single_op!(parse_bit_xor, parse_bit_and, BitXor, BinOp::BitXor);
    binary_single_op!(parse_bit_and, parse_shift, BitAnd, BinOp::BitAnd);

    binary_multi_op!(parse_shift, parse_equality, Shl => BinOp::Shl, Shr => BinOp::Shr, UShr => BinOp::UShr);
    binary_multi_op!(parse_equality, parse_comparison,
        StrictEq => BinOp::StrictEq, StrictNe => BinOp::StrictNe,
        Eq => BinOp::Eq, Ne => BinOp::Ne);
    binary_multi_op!(parse_comparison, parse_term,
        Lt => BinOp::Lt, Le => BinOp::Le, Gt => BinOp::Gt, Ge => BinOp::Ge, In => BinOp::In);
    binary_multi_op!(parse_term, parse_factor, Plus => BinOp::Add, Minus => BinOp::Sub);
    binary_multi_op!(parse_factor, parse_pow, Star => BinOp::Mul, Slash => BinOp::Div, Percent => BinOp::Mod);

    fn parse_pow(&mut self) -> Result<Expr, String> {
        let left = self.parse_unary()?;
        if matches!(self.peek_kind(), Some(TokenKind::StarStar)) {
            self.advance();
            let right = self.parse_pow()?; // right-associative
            let start = left.span().start;
            let end = right.span().end;
            return Ok(Expr::Binary {
                left: Box::new(left),
                op: BinOp::Pow,
                right: Box::new(right),
                span: Span { start, end },
            });
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        // Handle prefix ++/-- (consolidated)
        if let Some(is_inc) = match self.peek_kind() {
            Some(TokenKind::PlusPlus) => Some(true),
            Some(TokenKind::MinusMinus) => Some(false),
            _ => None,
        } {
            let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
            self.advance();
            let operand = self.parse_unary()?;
            if let Expr::Ident { name, span } = &operand {
                let name = Arc::clone(name);
                let span = Span {
                    start: span_start,
                    end: span.end,
                };
                return Ok(if is_inc {
                    Expr::PrefixInc { name, span }
                } else {
                    Expr::PrefixDec { name, span }
                });
            }
            return Err(format!(
                "Prefix {} requires an identifier",
                if is_inc { "++" } else { "--" }
            ));
        }
        let op = match self.peek_kind() {
            Some(TokenKind::Not) => UnaryOp::Not,
            Some(TokenKind::Minus) => UnaryOp::Neg,
            Some(TokenKind::Plus) => UnaryOp::Pos,
            Some(TokenKind::BitNot) => UnaryOp::BitNot,
            Some(TokenKind::Void) => UnaryOp::Void,
            Some(TokenKind::TypeOf) => {
                let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
                self.advance();
                let operand = self.parse_unary()?;
                let end = operand.span().end;
                return Ok(Expr::TypeOf {
                    operand: Box::new(operand),
                    span: Span {
                        start: span_start,
                        end,
                    },
                });
            }
            Some(TokenKind::Await) => {
                let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
                self.advance();
                let operand = self.parse_unary()?;
                let end = operand.span().end;
                return Ok(Expr::Await {
                    operand: Box::new(operand),
                    span: Span {
                        start: span_start,
                        end,
                    },
                });
            }
            _ => return self.parse_postfix(),
        };
        let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
        self.advance();
        let operand = self.parse_unary()?;
        let end = operand.span().end;
        Ok(Expr::Unary {
            op,
            operand: Box::new(operand),
            span: Span {
                start: span_start,
                end,
            },
        })
    }

    /// Member chain (`.`, `?.`, `[]`) without consuming a call `(...)`.
    fn parse_member_expression_no_call(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        while let Some(kind) = self.peek_kind() {
            match kind {
                TokenKind::Dot | TokenKind::OptionalChain => {
                    let optional = kind == TokenKind::OptionalChain;
                    self.advance();
                    let prop_tok = self.expect_ident_or_type_member_name()?;
                    let prop = prop_tok.literal.clone().ok_or("Expected property name")?;
                    let prop_span = Span {
                        start: prop_tok.span.start,
                        end: prop_tok.span.end,
                    };
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Member {
                        object: Box::new(expr),
                        prop: MemberProp::Name {
                            name: prop,
                            span: prop_span,
                        },
                        optional,
                        span: Span { start, end },
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(TokenKind::RBracket)?;
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                        optional: false,
                        span: Span { start, end },
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// ECMAScript `NewExpression`: `new` chains, then member expression without call, optional `(...)`.
    fn parse_new_expression(&mut self) -> Result<Expr, String> {
        if matches!(self.peek_kind(), Some(TokenKind::New)) {
            let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
            self.advance();
            let callee = Box::new(self.parse_new_expression()?);
            let args = if matches!(self.peek_kind(), Some(TokenKind::LParen)) {
                self.advance();
                let mut args = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                        self.advance();
                        let arg_expr = self.parse_expr()?;
                        args.push(CallArg::Spread(arg_expr));
                    } else {
                        let arg_expr = self.parse_expr()?;
                        args.push(CallArg::Expr(arg_expr));
                    }
                    if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                        self.expect(TokenKind::Comma)?;
                    }
                }
                self.expect(TokenKind::RParen)?;
                args
            } else {
                Vec::new()
            };
            let end = self
                .peek()
                .map(|x| x.span.start)
                .unwrap_or(callee.as_ref().span().end);
            Ok(Expr::New {
                callee,
                args,
                span: Span {
                    start: span_start,
                    end,
                },
            })
        } else {
            self.parse_member_expression_no_call()
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = if matches!(self.peek_kind(), Some(TokenKind::New)) {
            self.parse_new_expression()?
        } else {
            self.parse_primary()?
        };
        while let Some(kind) = self.peek_kind() {
            match kind {
                // `expr as Type` — a type assertion. Gradual + erased: consume the type and keep the
                // expression unchanged (no runtime effect; the checker is already lenient on what it
                // can't prove, so the assertion's only job — silencing a strict error — is moot).
                TokenKind::As => {
                    self.advance(); // as
                    self.parse_type_annotation()?;
                }
                TokenKind::LParen => {
                    self.advance();
                    let mut args = Vec::new();
                    while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                        if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                            self.advance();
                            let arg_expr = self.parse_expr()?;
                            args.push(CallArg::Spread(arg_expr));
                        } else {
                            let arg_expr = self.parse_expr()?;
                            args.push(CallArg::Expr(arg_expr));
                        }
                        if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                            self.expect(TokenKind::Comma)?;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                        span: Span { start, end },
                    };
                }
                TokenKind::Dot | TokenKind::OptionalChain => {
                    let optional = kind == TokenKind::OptionalChain;
                    self.advance();
                    let prop_tok = self.expect_ident_or_type_member_name()?;
                    let prop = prop_tok.literal.clone().ok_or("Expected property name")?;
                    let prop_span = Span {
                        start: prop_tok.span.start,
                        end: prop_tok.span.end,
                    };
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Member {
                        object: Box::new(expr),
                        prop: MemberProp::Name {
                            name: prop,
                            span: prop_span,
                        },
                        optional,
                        span: Span { start, end },
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(TokenKind::RBracket)?;
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                        optional: false,
                        span: Span { start, end },
                    };
                }
                TokenKind::PlusPlus | TokenKind::MinusMinus => {
                    if let Expr::Ident {
                        name,
                        span: ident_span,
                    } = &expr
                    {
                        let name = Arc::clone(name);
                        let is_inc = kind == TokenKind::PlusPlus;
                        let tok = self.advance().ok_or("Unexpected EOF")?;
                        let span = Span {
                            start: ident_span.start,
                            end: tok.span.end,
                        };
                        expr = if is_inc {
                            Expr::PostfixInc { name, span }
                        } else {
                            Expr::PostfixDec { name, span }
                        };
                    } else {
                        break;
                    }
                }
                TokenKind::Question => {
                    // Ternary is parsed in parse_conditional for correct precedence
                    break;
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let t = self.advance().ok_or("Unexpected EOF")?;
        let span = Span {
            start: t.span.start,
            end: t.span.end,
        };
        match t.kind {
            TokenKind::Number => {
                let s = t.literal.as_ref().ok_or("Expected number")?;
                let n: f64 = s.parse().map_err(|_| format!("Invalid number: {}", s))?;
                Ok(Expr::Literal {
                    value: Literal::Number(n),
                    span,
                })
            }
            TokenKind::String => {
                let s = t.literal.clone().ok_or("Expected string")?;
                Ok(Expr::Literal {
                    value: Literal::String(s),
                    span,
                })
            }
            TokenKind::True => Ok(Expr::Literal {
                value: Literal::Bool(true),
                span,
            }),
            TokenKind::False => Ok(Expr::Literal {
                value: Literal::Bool(false),
                span,
            }),
            TokenKind::Null => Ok(Expr::Literal {
                value: Literal::Null,
                span,
            }),
            TokenKind::Ident => {
                let name = t.literal.clone().ok_or("Expected ident")?;
                // Check if this is a single-param arrow function: x => ...
                if matches!(self.peek_kind(), Some(TokenKind::Arrow)) {
                    self.advance(); // consume =>
                    let body = self.parse_arrow_body()?;
                    let end = self.previous_span_end();
                    return Ok(Expr::ArrowFunction {
                        params: vec![FunParam::Simple(TypedParam {
                            name: name.clone(),
                            name_span: span,
                            type_ann: None,
                            default: None,
                        })],
                        body,
                        span: Span {
                            start: span.start,
                            end,
                        },
                    });
                }
                Ok(Expr::Ident { name, span })
            }
            TokenKind::LParen => {
                // Check if this is an arrow function: (params) => ...
                if let Some(arrow_fn) = self.try_parse_arrow_function(&span)? {
                    return Ok(arrow_fn);
                }
                // Otherwise it's a grouping expression
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                let mut elements = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
                    if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                        self.advance();
                        let expr = self.parse_expr()?;
                        elements.push(ArrayElement::Spread(expr));
                    } else {
                        let expr = self.parse_expr()?;
                        elements.push(ArrayElement::Expr(expr));
                    }
                    if !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
                        self.expect(TokenKind::Comma)?;
                    }
                }
                self.expect(TokenKind::RBracket)?;
                Ok(Expr::Array {
                    elements,
                    span: Span {
                        start: span.start,
                        end: self.peek().map(|x| x.span.end).unwrap_or(span.end),
                    },
                })
            }
            TokenKind::Lt => {
                // JSX: <Tag or <>
                match self.peek_kind() {
                    Some(TokenKind::Ident) => self.parse_jsx_element(span.start),
                    Some(TokenKind::Gt) => self.parse_jsx_fragment(span.start),
                    _ => Err(format!(
                        "Invalid JSX: expected tag name or <> after <, got {:?}",
                        self.peek_kind()
                    )),
                }
            }
            TokenKind::LBrace => {
                let mut props = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                    if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                        self.advance();
                        let expr = self.parse_expr()?;
                        props.push(ObjectProp::Spread(expr));
                    } else {
                        let key_tok = self.advance().ok_or("Expected object key")?;
                        let (key, key_span, is_ident_key) = match key_tok.kind {
                            TokenKind::Ident | TokenKind::Type | TokenKind::Declare => {
                                let k = key_tok.literal.clone().ok_or("Expected key")?;
                                let sp = Span {
                                    start: key_tok.span.start,
                                    end: key_tok.span.end,
                                };
                                (k, sp, true)
                            }
                            TokenKind::String => {
                                let k = key_tok.literal.clone().ok_or("Expected string key")?;
                                let sp = Span {
                                    start: key_tok.span.start,
                                    end: key_tok.span.end,
                                };
                                (k, sp, false)
                            }
                            _ => {
                                return Err(format!(
                                    "Expected object key (ident or string), got {:?}",
                                    key_tok.kind
                                ))
                            }
                        };
                        let value = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                            self.expect(TokenKind::Colon)?;
                            self.parse_expr()?
                        } else {
                            // ES6 shorthand: { key } => { key: key } (ident only, not string keys)
                            if is_ident_key {
                                Expr::Ident {
                                    name: key.clone(),
                                    span: key_span,
                                }
                            } else {
                                return Err("String key in object literal requires explicit value (key: value)".to_string());
                            }
                        };
                        props.push(ObjectProp::KeyValue(key, value));
                    }
                    if !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                        self.expect(TokenKind::Comma)?;
                    }
                }
                self.expect(TokenKind::RBrace)?;
                Ok(Expr::Object {
                    props,
                    span: Span {
                        start: span.start,
                        end: self.peek().map(|x| x.span.end).unwrap_or(span.end),
                    },
                })
            }
            TokenKind::TemplateNoSub => {
                // Simple template literal without interpolation
                Ok(Expr::TemplateLiteral {
                    quasis: vec![t.literal.clone().unwrap_or_default()],
                    exprs: vec![],
                    span,
                })
            }
            TokenKind::TemplateHead => {
                // Template literal with interpolation: `text${
                let mut quasis = vec![t.literal.clone().unwrap_or_default()];
                let mut exprs = Vec::new();

                loop {
                    // Parse the expression inside ${}
                    let expr = self.parse_expr()?;
                    exprs.push(expr);

                    // Next token should be TemplateMiddle or TemplateTail
                    let next = self.advance().ok_or("Unexpected EOF in template literal")?;
                    match next.kind {
                        TokenKind::TemplateTail => {
                            quasis.push(next.literal.clone().unwrap_or_default());
                            let end = self.previous_span_end();
                            return Ok(Expr::TemplateLiteral {
                                quasis,
                                exprs,
                                span: Span {
                                    start: span.start,
                                    end,
                                },
                            });
                        }
                        TokenKind::TemplateMiddle => {
                            quasis.push(next.literal.clone().unwrap_or_default());
                            // Continue parsing more expressions
                        }
                        _ => {
                            return Err(format!(
                                "Expected template continuation, got {:?}",
                                next.kind
                            ))
                        }
                    }
                }
            }
            // Include the token span (matching `expect`'s `at {:?}` convention) so error consumers
            // — notably the LSP's `parse_error_pos` — can place the diagnostic at the real location
            // instead of falling back to (0, 0) / top-of-file.
            _ => Err(format!("Unexpected token: {:?} at {:?}", t.kind, t.span)),
        }
    }

    /// Try to parse an arrow function starting with '(' already consumed.
    /// Returns Some(Expr) if successful, None if it's not an arrow function.
    fn try_parse_arrow_function(&mut self, start_span: &Span) -> Result<Option<Expr>, String> {
        // Save position for backtracking
        let saved_pos = self.pos;

        // Try to parse as arrow function params
        let mut params = Vec::new();
        let mut is_arrow = false;

        // Check for empty params: () => ...
        if matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            self.advance(); // consume )
            if matches!(self.peek_kind(), Some(TokenKind::Arrow)) {
                self.advance(); // consume =>
                is_arrow = true;
            }
        } else {
            // Try to parse params: (x, y), ({ a }), ([a, b]), with optional types/defaults
            let mut params_ok = true;
            loop {
                if matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    break;
                }
                match self.parse_fun_param() {
                    Ok(param) => params.push(param),
                    Err(_) => {
                        params_ok = false;
                        break;
                    }
                }
                if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
            if params_ok && matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                self.advance(); // consume )
                if matches!(self.peek_kind(), Some(TokenKind::Arrow)) {
                    self.advance(); // consume =>
                    is_arrow = true;
                }
            }
        }

        if !is_arrow {
            // Backtrack - it's not an arrow function
            self.pos = saved_pos;
            return Ok(None);
        }

        let body = self.parse_arrow_body()?;
        let end = self.previous_span_end();

        Ok(Some(Expr::ArrowFunction {
            params,
            body,
            span: Span {
                start: start_span.start,
                end,
            },
        }))
    }

    /// Parse the body of an arrow function (either expression or block)
    fn parse_arrow_body(&mut self) -> Result<ArrowBody, String> {
        if matches!(self.peek_kind(), Some(TokenKind::LBrace)) {
            // Block body
            let block = self.parse_block()?;
            Ok(ArrowBody::Block(Box::new(block)))
        } else {
            // Expression body
            let expr = self.parse_expr()?;
            Ok(ArrowBody::Expr(Box::new(expr)))
        }
    }

    fn previous_span_end(&self) -> (usize, usize) {
        if self.pos > 0 && self.pos <= self.tokens.len() {
            self.tokens[self.pos - 1].span.end
        } else {
            (1, 1)
        }
    }

    /// Parse JSX element: <Tag props>children</Tag> or <Tag props />
    /// Caller has already consumed <.
    fn parse_jsx_element(&mut self, start: (usize, usize)) -> Result<Expr, String> {
        let tag_tok = self.expect(TokenKind::Ident)?;
        let tag = tag_tok.literal.clone().ok_or("Expected tag name")?;

        let mut props = Vec::new();
        loop {
            match self.peek_kind() {
                Some(TokenKind::Slash) => {
                    // Self-closing: />
                    self.advance();
                    self.expect(TokenKind::Gt)?;
                    let end = self.previous_span_end();
                    return Ok(Expr::JsxElement {
                        tag,
                        props,
                        children: vec![],
                        span: Span { start, end },
                    });
                }
                Some(TokenKind::Gt) => break,
                Some(TokenKind::Spread) => {
                    self.advance(); // ...
                    let expr = self.parse_expr()?;
                    self.expect(TokenKind::RBrace)?; // }
                    props.push(JsxProp::Spread(expr));
                }
                // `type` is `TokenKind::Type` but valid as a JSX attr name; see docs/js-emit-philosophy.md.
                Some(TokenKind::Ident) | Some(TokenKind::Type) => {
                    let name_tok = self.advance().unwrap();
                    let name = name_tok.literal.clone().ok_or("Expected attr name")?;
                    if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
                        self.advance(); // =
                        let value = if matches!(self.peek_kind(), Some(TokenKind::LBrace)) {
                            self.advance(); // {
                            let expr = self.parse_expr()?;
                            self.expect(TokenKind::RBrace)?; // }
                            JsxAttrValue::Expr(expr)
                        } else {
                            let s = self
                                .expect(TokenKind::String)?
                                .literal
                                .clone()
                                .ok_or("Expected string")?;
                            JsxAttrValue::String(s)
                        };
                        props.push(JsxProp::Attr { name, value });
                    } else {
                        props.push(JsxProp::Attr {
                            name,
                            value: JsxAttrValue::ImplicitTrue,
                        });
                    }
                }
                _ => {
                    return Err(format!(
                        "Unexpected token in JSX props: {:?}",
                        self.peek_kind()
                    ))
                }
            }
        }
        self.advance(); // consume >

        let children = self.parse_jsx_children(&tag)?;
        let end = self.previous_span_end();
        Ok(Expr::JsxElement {
            tag,
            props,
            children,
            span: Span { start, end },
        })
    }

    fn token_as_jsx_text(kind: TokenKind) -> Option<&'static str> {
        use TokenKind::*;
        match kind {
            Not => Some("!"),
            Question => Some("?"),
            Dot => Some("."),
            Comma => Some(","),
            Colon => Some(":"),
            Semicolon => Some(";"),
            Plus => Some("+"),
            Minus => Some("-"),
            Star => Some("*"),
            Slash => Some("/"),
            Percent => Some("%"),
            Eq | Assign => Some("="),
            Gt => Some(">"),
            Le => Some("<="),
            Ge => Some(">="),
            Ne => Some("!="),
            StrictEq => Some("==="),
            StrictNe => Some("!=="),
            BitAnd => Some("&"),
            BitOr => Some("|"),
            BitXor => Some("^"),
            BitNot => Some("~"),
            And => Some("&&"),
            Or => Some("||"),
            LParen => Some("("),
            RParen => Some(")"),
            LBracket => Some("["),
            RBracket => Some("]"),
            PlusPlus => Some("++"),
            MinusMinus => Some("--"),
            StarStar => Some("**"),
            Arrow => Some("=>"),
            OptionalChain => Some("?."),
            NullishCoalesce => Some("??"),
            Shl => Some("<<"),
            Shr => Some(">>"),
            _ => None,
        }
    }

    /// Merge text. Add space between words (Ident/Number/String); no space before/after punctuation.
    fn push_or_merge_text(&self, children: &mut Vec<JsxChild>, s: Arc<str>, is_punctuation: bool) {
        if let Some(JsxChild::Text(prev)) = children.last() {
            let sep = if is_punctuation { "" } else { " " };
            let merged = format!("{}{}{}", prev.as_ref(), sep, s.as_ref());
            let last = children.len() - 1;
            children[last] = JsxChild::Text(Arc::from(merged.as_str()));
        } else {
            children.push(JsxChild::Text(s));
        }
    }

    /// Parse JSX children until </Tag> or </>
    fn parse_jsx_children(&mut self, close_tag: &str) -> Result<Vec<JsxChild>, String> {
        let mut children = Vec::new();
        loop {
            match self.peek_kind() {
                None => return Err("Unexpected EOF in JSX".to_string()),
                Some(TokenKind::Lt) => {
                    let next = self.tokens.get(self.pos + 1);
                    if let Some(t) = next {
                        if t.kind == TokenKind::Slash {
                            // </ closing tag
                            self.advance(); // <
                            self.advance(); // /
                            let name = self
                                .expect(TokenKind::Ident)?
                                .literal
                                .clone()
                                .ok_or("Expected tag name")?;
                            if name.as_ref() != close_tag {
                                return Err(format!(
                                    "Mismatched JSX tag: expected </{}> got </{}>",
                                    close_tag, name
                                ));
                            }
                            self.expect(TokenKind::Gt)?; // >
                            return Ok(children);
                        }
                        if t.kind == TokenKind::Gt {
                            return Err("Unexpected <> in JSX children".to_string());
                        }
                    }
                    // <Tag - nested element
                    let nested_start = self.peek().unwrap().span.start;
                    self.advance(); // <
                    let elem = self.parse_jsx_element(nested_start)?;
                    children.push(JsxChild::Expr(elem));
                }
                Some(TokenKind::LBrace) => {
                    self.advance(); // {
                    let expr = self.parse_expr()?;
                    self.expect(TokenKind::RBrace)?; // }
                    children.push(JsxChild::Expr(expr));
                }
                Some(TokenKind::JsxText) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::String) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::Number) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::Ident) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(k) => {
                    if let Some(s) = Self::token_as_jsx_text(k) {
                        self.advance();
                        self.push_or_merge_text(&mut children, Arc::from(s), true);
                    } else {
                        return Err(format!("Unexpected token in JSX children: {:?}", k));
                    }
                }
            }
        }
    }

    fn parse_jsx_fragment(&mut self, start: (usize, usize)) -> Result<Expr, String> {
        self.advance(); // consume >
        let mut children = Vec::new();
        loop {
            match self.peek_kind() {
                None => return Err("Unexpected EOF in JSX fragment".to_string()),
                Some(TokenKind::Lt) => {
                    let next = self.tokens.get(self.pos + 1);
                    if let Some(t) = next {
                        if t.kind == TokenKind::Slash {
                            // </
                            let next2 = self.tokens.get(self.pos + 2);
                            if let Some(t2) = next2 {
                                if t2.kind == TokenKind::Gt {
                                    // </>
                                    self.advance();
                                    self.advance();
                                    self.advance();
                                    let end = self.previous_span_end();
                                    return Ok(Expr::JsxFragment {
                                        children,
                                        span: Span { start, end },
                                    });
                                }
                            }
                            return Err("Expected </> to close fragment".to_string());
                        }
                    }
                    let nested_start = self.peek().unwrap().span.start;
                    self.advance(); // <
                    let elem = self.parse_jsx_element(nested_start)?;
                    children.push(JsxChild::Expr(elem));
                }
                Some(TokenKind::LBrace) => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    self.expect(TokenKind::RBrace)?;
                    children.push(JsxChild::Expr(expr));
                }
                Some(TokenKind::JsxText) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::String) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::Number) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(TokenKind::Ident) => {
                    let t = self.advance().unwrap();
                    let s = t.literal.clone().unwrap_or_default();
                    if !s.is_empty() {
                        self.push_or_merge_text(&mut children, s, false);
                    }
                }
                Some(k) => {
                    if let Some(s) = Self::token_as_jsx_text(k) {
                        self.advance();
                        self.push_or_merge_text(&mut children, Arc::from(s), true);
                    } else {
                        return Err(format!("Unexpected token in JSX fragment: {:?}", k));
                    }
                }
            }
        }
    }
}

// Helper to get span from Expr. Uses trait so ExprSpan is referenced.
trait ExprSpan {
    fn span(&self) -> Span;
}

#[inline(always)]
fn expr_span(e: &impl ExprSpan) -> Span {
    e.span()
}

impl ExprSpan for Expr {
    fn span(&self) -> Span {
        match self {
            Expr::Literal { span, .. } => *span,
            Expr::Ident { span, .. } => *span,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::New { span, .. } => *span,
            Expr::Member { span, .. } => *span,
            Expr::Index { span, .. } => *span,
            Expr::Conditional { span, .. } => *span,
            Expr::NullishCoalesce { span, .. } => *span,
            Expr::Array { span, .. } => *span,
            Expr::Object { span, .. } => *span,
            Expr::Assign { span, .. } => *span,
            Expr::TypeOf { span, .. } => *span,
            Expr::PostfixInc { span, .. } => *span,
            Expr::PostfixDec { span, .. } => *span,
            Expr::PrefixInc { span, .. } => *span,
            Expr::PrefixDec { span, .. } => *span,
            Expr::CompoundAssign { span, .. } => *span,
            Expr::LogicalAssign { span, .. } => *span,
            Expr::MemberAssign { span, .. } => *span,
            Expr::IndexAssign { span, .. } => *span,
            Expr::ArrowFunction { span, .. } => *span,
            Expr::TemplateLiteral { span, .. } => *span,
            Expr::Await { span, .. } => *span,
            Expr::JsxElement { span, .. } => *span,
            Expr::JsxFragment { span, .. } => *span,
            Expr::NativeModuleLoad { span, .. } => *span,
        }
    }
}
