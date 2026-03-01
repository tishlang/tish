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
                let start = left.span().start;
                let end = right.span().end;
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
                let start = left.span().start;
                let end = right.span().end;
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

use tish_ast::{
    ArrowBody, ArrayElement, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    DestructProp, Expr, Literal, MemberProp, ObjectProp, Program, Span, Statement, TypeAnnotation,
    TypedParam, UnaryOp,
};
use tish_lexer::{Token, TokenKind};

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
            TokenKind::Fn => self.parse_fun_decl()?,
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

    fn parse_fun_decl(&mut self) -> Result<Statement, String> {
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
            let param_name = self
                .expect(TokenKind::Ident)?
                .literal
                .clone()
                .ok_or("Expected param name")?;
            // Optional type annotation
            let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                self.advance();
                Some(self.parse_type_annotation()?)
            } else {
                None
            };
            // Optional default value
            let default = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(TypedParam { name: param_name, type_ann, default });
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
                type_ann: None, // For-loop variables don't have type annotations (yet)
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

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
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
                        params: vec![TypedParam { name: name.clone(), type_ann: None, default: None }],
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
            TokenKind::LBrace => {
                let mut props = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBrace)) {
                    if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                        self.advance();
                        let expr = self.parse_expr()?;
                        props.push(ObjectProp::Spread(expr));
                    } else {
                        let key = self
                            .expect(TokenKind::Ident)?
                            .literal
                            .clone()
                            .ok_or("Expected key")?;
                        self.expect(TokenKind::Colon)?;
                        let value = self.parse_expr()?;
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
            // Try to parse params: (x, y, z) or (x: Type, y: Type)
            loop {
                if !matches!(self.peek_kind(), Some(TokenKind::Ident)) {
                    break; // Not a valid arrow function param list
                }
                let name = self.advance().unwrap().literal.clone().ok_or("Expected param name")?;
                
                // Optional type annotation
                let type_ann = if matches!(self.peek_kind(), Some(TokenKind::Colon)) {
                    self.advance();
                    Some(self.parse_type_annotation()?)
                } else {
                    None
                };
                
                // Optional default value
                let default = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                
                params.push(TypedParam { name, type_ann, default });
                
                if matches!(self.peek_kind(), Some(TokenKind::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
            
            if matches!(self.peek_kind(), Some(TokenKind::RParen)) {
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
}

// Helper to get span from Expr
trait ExprSpan {
    fn span(&self) -> Span;
}

impl ExprSpan for Expr {
    fn span(&self) -> Span {
        match self {
            Expr::Literal { span, .. } => *span,
            Expr::Ident { span, .. } => *span,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
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
            Expr::MemberAssign { span, .. } => *span,
            Expr::IndexAssign { span, .. } => *span,
            Expr::ArrowFunction { span, .. } => *span,
            Expr::TemplateLiteral { span, .. } => *span,
        }
    }
}

