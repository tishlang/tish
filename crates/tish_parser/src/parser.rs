//! Recursive descent parser for Tish.

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
    ArrowBody, ArrayElement, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    DestructProp, ExportDeclaration, Expr, FunParam, ImportSpecifier, JsxAttrValue, JsxChild,
    JsxProp, Literal, LogicalAssignOp, MemberProp, ObjectProp, Program, Span, Statement,
    TypeAnnotation, TypedParam, UnaryOp,
};
use tishlang_lexer::{Token, TokenKind};

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
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
        let t = self.advance().ok_or_else(|| format!("Expected {:?}, got EOF", kind))?;
        if t.kind == kind {
            Ok(t)
        } else {
            Err(format!("Expected {:?}, got {:?} at {:?}", kind, t.kind, t.span))
        }
    }

    fn span_end(&self, start: (usize, usize)) -> Span {
        let end = self
            .peek()
            .map(|t| t.span.start)
            .unwrap_or(start);
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

        if matches!(self.peek_kind(), Some(TokenKind::LBrace)) {
            self.advance(); // {
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

        if matches!(self.peek_kind(), Some(TokenKind::RBrace | TokenKind::Dedent)) {
            self.advance();
        }

        Ok(Statement::Block {
            statements,
            span: Span {
                start: span_start,
                end: self
                    .peek()
                    .map(|x| x.span.end)
                    .unwrap_or(span_start),
            },
        })
    }

    fn parse_var_decl(&mut self, mutable: bool) -> Result<Statement, String> {
        let span_start = if mutable {
            self.expect(TokenKind::Let)?.span.start
        } else {
            self.expect(TokenKind::Const)?.span.start
        };
        
        // Check for destructuring pattern
        if matches!(self.peek_kind(), Some(TokenKind::LBracket) | Some(TokenKind::LBrace)) {
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
        
        let name = self
            .expect(TokenKind::Ident)?
            .literal
            .clone()
            .ok_or("Expected identifier")?;

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
                let name = self.expect(TokenKind::Ident)?.literal.clone().ok_or("Expected identifier")?;
                elements.push(Some(DestructElement::Rest(name)));
                break;
            }
            
            // Nested pattern or identifier
            let elem = match self.peek_kind() {
                Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                    let nested = self.parse_destruct_pattern()?;
                    DestructElement::Pattern(Box::new(nested))
                }
                Some(TokenKind::Ident) => {
                    let name = self.advance().ok_or("Unexpected EOF")?.literal.clone().ok_or("Expected identifier")?;
                    DestructElement::Ident(name)
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
            let key = self.expect(TokenKind::Ident)?.literal.clone().ok_or("Expected identifier")?;
            
            let value = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                self.advance();
                // Could be renamed binding or nested pattern
                match self.peek_kind() {
                    Some(TokenKind::LBracket) | Some(TokenKind::LBrace) => {
                        let nested = self.parse_destruct_pattern()?;
                        DestructElement::Pattern(Box::new(nested))
                    }
                    Some(TokenKind::Ident) => {
                        let name = self.advance().ok_or("Unexpected EOF")?.literal.clone().ok_or("Expected identifier")?;
                        DestructElement::Ident(name)
                    }
                    _ => return Err("Expected identifier or pattern after ':'".to_string()),
                }
            } else {
                // Shorthand: { key } is equivalent to { key: key }
                DestructElement::Ident(key.clone())
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
        let param_name = self
            .expect(TokenKind::Ident)?
            .literal
            .clone()
            .ok_or("Expected param name")?;
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
            type_ann,
            default,
        }))
    }
    
    /// Parse a type annotation (number, string, T[], {a: T}, etc.)
    fn parse_type_annotation(&mut self) -> Result<TypeAnnotation, String> {
        let base = self.parse_type_primary()?;
        
        // Check for array suffix: T[]
        if matches!(self.peek_kind(), Some(TokenKind::LBracket)) {
            self.advance(); // [
            self.expect(TokenKind::RBracket)?; // ]
            return Ok(TypeAnnotation::Array(Box::new(base)));
        }
        
        // Check for union: T | U
        if matches!(self.peek_kind(), Some(TokenKind::BitOr)) {
            let mut types = vec![base];
            while matches!(self.peek_kind(), Some(TokenKind::BitOr)) {
                self.advance(); // |
                types.push(self.parse_type_primary()?);
            }
            return Ok(TypeAnnotation::Union(types));
        }
        
        Ok(base)
    }
    
    /// Parse a primary type (identifier, object, or function type)
    fn parse_type_primary(&mut self) -> Result<TypeAnnotation, String> {
        match self.peek_kind() {
            Some(TokenKind::Ident) => {
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
                    let key = self.expect(TokenKind::Ident)?.literal.clone()
                        .ok_or("Expected property name")?;
                    self.expect(TokenKind::Colon)?;
                    let typ = self.parse_type_annotation()?;
                    props.push((key, typ));
                    if !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                        // Allow trailing comma or require comma between items
                        if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
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
                // Expect => for return type
                self.expect(TokenKind::Assign)?; // = 
                self.expect(TokenKind::Gt)?; // > (forms =>)
                let returns = self.parse_type_annotation()?;
                Ok(TypeAnnotation::Function {
                    params,
                    returns: Box::new(returns),
                })
            }
            _ => Err("Expected type annotation".to_string()),
        }
    }

    fn parse_fun_decl(&mut self, async_: bool) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Fn)?.span.start;
        let name = self
            .expect(TokenKind::Ident)?
            .literal
            .clone()
            .ok_or("Expected function name")?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::with_capacity(4);
        let mut rest_param = None;
        while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                self.advance();
                let param_name = self
                    .expect(TokenKind::Ident)?
                    .literal
                    .clone()
                    .ok_or("Expected rest param name")?;
                // Optional type annotation for rest param
                let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                    self.advance();
                    Some(self.parse_type_annotation()?)
                } else {
                    None
                };
                rest_param = Some(TypedParam { name: param_name, type_ann, default: None });
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

        Ok(Statement::FunDecl {
            async_,
            name,
            params,
            rest_param,
            return_type,
            body,
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
            let name = self
                .expect(TokenKind::Ident)?
                .literal
                .clone()
                .ok_or("Expected identifier")?;
            if matches!(self.peek_kind(), Some(TokenKind::Of)) {
                self.advance();
                let iterable = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                let body = Box::new(self.parse_block_or_statement()?);
                return Ok(Statement::ForOf {
                    name,
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
            if matches!(self.peek_kind(), Some(TokenKind::Semicolon)) {
                self.advance();
            }
            Some(Box::new(Statement::VarDecl {
                name,
                mutable,
                type_ann,
                init: init_expr,
                span: self.span_end(var_span_start),
            }))
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
        let cond = if matches!(self.peek_kind(), Some(TokenKind::Semicolon | TokenKind::RParen)) {
            None
        } else {
            let c = self.parse_expr()?;
            self.expect(TokenKind::Semicolon)?;
            Some(c)
        };
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
        let mut catch_body = None;
        let mut finally_body = None;
        
        if matches!(self.peek_kind(), Some(TokenKind::Catch)) {
            self.advance();
            self.expect(TokenKind::LParen)?;
            catch_param = self.expect(TokenKind::Ident)?.literal.clone();
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
                let name = self
                    .expect(TokenKind::Ident)?
                    .literal
                    .clone()
                    .ok_or("Expected identifier in import")?;
                let alias = if matches!(self.peek_kind(), Some(TokenKind::Ident))
                    && self.peek().and_then(|t| t.literal.as_deref()) == Some("as")
                {
                    self.advance(); // consume 'as'
                    Some(
                        self.expect(TokenKind::Ident)?
                            .literal
                            .clone()
                            .ok_or("Expected alias after 'as'")?,
                    )
                } else {
                    None
                };
                specs.push(ImportSpecifier::Named { name, alias });
                if !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RBrace)?;
            specs
        } else if matches!(self.peek_kind(), Some(TokenKind::Star)) {
            // Namespace: import * as M from "..."
            self.advance();
            let as_tok = self.expect(TokenKind::Ident)?;
            if as_tok.literal.as_deref() != Some("as") {
                return Err("Expected 'as' after '*' in namespace import".to_string());
            }
            let alias = self
                .expect(TokenKind::Ident)?
                .literal
                .clone()
                .ok_or("Expected identifier after 'as'")?;
            vec![ImportSpecifier::Namespace(alias)]
        } else if matches!(self.peek_kind(), Some(TokenKind::Ident)) {
            // Default: import X from "..."
            let name = self
                .expect(TokenKind::Ident)?
                .literal
                .clone()
                .ok_or("Expected identifier")?;
            vec![ImportSpecifier::Default(name)]
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
            return Err("Expected 'default', 'const', 'let', or 'fn' after export".to_string());
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
            if let Expr::Member { object, prop: MemberProp::Name(prop_name), .. } = &left {
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
    
    binary_multi_op!(parse_shift, parse_equality, Shl => BinOp::Shl, Shr => BinOp::Shr);
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
                let span = Span { start: span_start, end: span.end };
                return Ok(if is_inc {
                    Expr::PrefixInc { name, span }
                } else {
                    Expr::PrefixDec { name, span }
                });
            }
            return Err(format!("Prefix {} requires an identifier", if is_inc { "++" } else { "--" }));
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
                    span: Span { start: span_start, end },
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
                    let prop = self
                        .expect(TokenKind::Ident)?
                        .literal
                        .clone()
                        .ok_or("Expected property name")?;
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Member {
                        object: Box::new(expr),
                        prop: MemberProp::Name(prop),
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
                    let prop = self
                        .expect(TokenKind::Ident)?
                        .literal
                        .clone()
                        .ok_or("Expected property name")?;
                    let start = expr.span().start;
                    let end = self.peek().map(|x| x.span.start).unwrap_or(start);
                    expr = Expr::Member {
                        object: Box::new(expr),
                        prop: MemberProp::Name(prop),
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
                    if let Expr::Ident { name, span: ident_span } = &expr {
                        let name = Arc::clone(name);
                        let is_inc = kind == TokenKind::PlusPlus;
                        let tok = self.advance().ok_or("Unexpected EOF")?;
                        let span = Span { start: ident_span.start, end: tok.span.end };
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
                            type_ann: None,
                            default: None,
                        })],
                        body,
                        span: Span { start: span.start, end },
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
                    _ => Err(format!("Invalid JSX: expected tag name or <> after <, got {:?}", self.peek_kind())),
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
                            TokenKind::Ident => {
                                let k = key_tok
                                    .literal
                                    .clone()
                                    .ok_or("Expected key")?;
                                let sp = Span {
                                    start: key_tok.span.start,
                                    end: key_tok.span.end,
                                };
                                (k, sp, true)
                            }
                            TokenKind::String => {
                                let k = key_tok
                                    .literal
                                    .clone()
                                    .ok_or("Expected string key")?;
                                let sp = Span {
                                    start: key_tok.span.start,
                                    end: key_tok.span.end,
                                };
                                (k, sp, false)
                            }
                            _ => return Err(format!(
                                "Expected object key (ident or string), got {:?}",
                                key_tok.kind
                            )),
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
                                span: Span { start: span.start, end },
                            });
                        }
                        TokenKind::TemplateMiddle => {
                            quasis.push(next.literal.clone().unwrap_or_default());
                            // Continue parsing more expressions
                        }
                        _ => return Err(format!("Expected template continuation, got {:?}", next.kind)),
                    }
                }
            }
            _ => Err(format!("Unexpected token: {:?}", t.kind)),
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
            span: Span { start: start_span.start, end },
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
                Some(TokenKind::Ident) => {
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
                            let s = self.expect(TokenKind::String)?.literal.clone().ok_or("Expected string")?;
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
                _ => return Err(format!("Unexpected token in JSX props: {:?}", self.peek_kind())),
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
                            let name = self.expect(TokenKind::Ident)?.literal.clone().ok_or("Expected tag name")?;
                            if name.as_ref() != close_tag {
                                return Err(format!("Mismatched JSX tag: expected </{}> got </{}>", close_tag, name));
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
                                    return Ok(Expr::JsxFragment { children, span: Span { start, end } });
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

