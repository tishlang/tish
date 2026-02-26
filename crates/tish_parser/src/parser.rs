//! Recursive descent parser for Tish.

use std::sync::Arc;

use tish_ast::{
    BinOp, Expr, Literal, MemberProp, Program, Span, Statement, UnaryOp,
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
        let mut statements = Vec::new();
        while self.peek_kind().is_some() && !matches!(self.peek_kind(), Some(TokenKind::Eof)) {
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
            TokenKind::Any => self.parse_var_decl()?,
            TokenKind::Fun => self.parse_fun_decl()?,
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

        let mut statements = Vec::new();
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

    fn parse_var_decl(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Any)?.span.start;
        let name = self
            .expect(TokenKind::Ident)?
            .literal
            .clone()
            .ok_or("Expected identifier")?;
        let init = if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Statement::VarDecl {
            name,
            init,
            span: self.span_end(span_start),
        })
    }

    fn parse_fun_decl(&mut self) -> Result<Statement, String> {
        let span_start = self.expect(TokenKind::Fun)?.span.start;
        let name = self
            .expect(TokenKind::Ident)?
            .literal
            .clone()
            .ok_or("Expected function name")?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut rest_param = None;
        while !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
            if matches!(self.peek_kind(), Some(TokenKind::Spread)) {
                self.advance();
                let name = self
                    .expect(TokenKind::Ident)?
                    .literal
                    .clone()
                    .ok_or("Expected rest param name")?;
                rest_param = Some(name);
                if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                    return Err("Rest parameter must be last".to_string());
                }
                break;
            }
            let p = self
                .expect(TokenKind::Ident)?
                .literal
                .clone()
                .ok_or("Expected param name")?;
            params.push(p);
            if !matches!(self.peek_kind(), Some(TokenKind::RParen)) {
                self.expect(TokenKind::Comma)?;
            }
        }
        self.expect(TokenKind::RParen)?;

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
        let init = if matches!(self.peek_kind(), Some(TokenKind::Any)) {
            let any_span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
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
                init: init_expr,
                span: self.span_end(any_span_start),
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
        self.expect(TokenKind::Catch)?;
        self.expect(TokenKind::LParen)?;
        let catch_param = self
            .expect(TokenKind::Ident)?
            .literal
            .clone();
        self.expect(TokenKind::RParen)?;
        let catch_body = Box::new(self.parse_block_or_statement()?);
        Ok(Statement::Try {
            body,
            catch_param,
            catch_body,
            span: self.span_end(span_start),
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        let left = self.parse_conditional()?;
        if matches!(self.peek_kind(), Some(TokenKind::Assign)) {
            if let Expr::Ident { name, span } = &left {
                let name = Arc::clone(name);
                let start = span.start;
                self.advance(); // =
                let value = self.parse_assign()?;
                let end = value.span().end;
                return Ok(Expr::Assign {
                    name,
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

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek_kind(), Some(TokenKind::Or)) {
            self.advance();
            let right = self.parse_and()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bit_or()?;
        while matches!(self.peek_kind(), Some(TokenKind::And)) {
            self.advance();
            let right = self.parse_bit_or()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    fn parse_bit_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bit_xor()?;
        while matches!(self.peek_kind(), Some(TokenKind::BitOr)) {
            self.advance();
            let right = self.parse_bit_xor()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitOr,
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    fn parse_bit_xor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bit_and()?;
        while matches!(self.peek_kind(), Some(TokenKind::BitXor)) {
            self.advance();
            let right = self.parse_bit_and()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitXor,
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    fn parse_bit_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_shift()?;
        while matches!(self.peek_kind(), Some(TokenKind::BitAnd)) {
            self.advance();
            let right = self.parse_shift()?;
            let start = left.span().start;
            let end = right.span().end;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::BitAnd,
                right: Box::new(right),
                span: Span { start, end },
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Shl) => BinOp::Shl,
                Some(TokenKind::Shr) => BinOp::Shr,
                _ => break,
            };
            self.advance();
            let right = self.parse_equality()?;
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

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::StrictEq) => BinOp::StrictEq,
                Some(TokenKind::StrictNe) => BinOp::StrictNe,
                Some(TokenKind::Eq) => BinOp::Eq,
                Some(TokenKind::Ne) => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
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

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_term()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Lt) => BinOp::Lt,
                Some(TokenKind::Le) => BinOp::Le,
                Some(TokenKind::Gt) => BinOp::Gt,
                Some(TokenKind::Ge) => BinOp::Ge,
                Some(TokenKind::In) => BinOp::In,
                _ => break,
            };
            self.advance();
            let right = self.parse_term()?;
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

    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_factor()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Plus) => BinOp::Add,
                Some(TokenKind::Minus) => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_factor()?;
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

    fn parse_factor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_pow()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Star) => BinOp::Mul,
                Some(TokenKind::Slash) => BinOp::Div,
                Some(TokenKind::Percent) => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_pow()?;
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
        if matches!(self.peek_kind(), Some(TokenKind::PlusPlus)) {
            let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
            self.advance();
            let operand = self.parse_unary()?;
            if let Expr::Ident { name, span } = &operand {
                let name = Arc::clone(name);
                let end = span.end;
                return Ok(Expr::PrefixInc {
                    name,
                    span: Span {
                        start: span_start,
                        end,
                    },
                });
            }
            return Err("Prefix ++ requires an identifier".to_string());
        }
        if matches!(self.peek_kind(), Some(TokenKind::MinusMinus)) {
            let span_start = self.peek().map(|t| t.span.start).unwrap_or((0, 0));
            self.advance();
            let operand = self.parse_unary()?;
            if let Expr::Ident { name, span } = &operand {
                let name = Arc::clone(name);
                let end = span.end;
                return Ok(Expr::PrefixDec {
                    name,
                    span: Span {
                        start: span_start,
                        end,
                    },
                });
            }
            return Err("Prefix -- requires an identifier".to_string());
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
                        args.push(self.parse_expr()?);
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
                TokenKind::PlusPlus => {
                    if let Expr::Ident { name, span } = &expr {
                        let name = Arc::clone(name);
                        let tok = self.advance().ok_or("Unexpected EOF")?;
                        expr = Expr::PostfixInc {
                            name,
                            span: Span {
                                start: span.start,
                                end: tok.span.end,
                            },
                        };
                    } else {
                        break;
                    }
                }
                TokenKind::MinusMinus => {
                    if let Expr::Ident { name, span } = &expr {
                        let name = Arc::clone(name);
                        let tok = self.advance().ok_or("Unexpected EOF")?;
                        expr = Expr::PostfixDec {
                            name,
                            span: Span {
                                start: span.start,
                                end: tok.span.end,
                            },
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
            TokenKind::Ident => Ok(Expr::Ident {
                name: t.literal.clone().ok_or("Expected ident")?,
                span,
            }),
            TokenKind::LParen => {
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                let mut elements = Vec::new();
                while !matches!(self.peek_kind(), Some(TokenKind::RBracket)) {
                    elements.push(self.parse_expr()?);
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
                    let key = self
                        .expect(TokenKind::Ident)?
                        .literal
                        .clone()
                        .ok_or("Expected key")?;
                    self.expect(TokenKind::Colon)?;
                    let value = self.parse_expr()?;
                    props.push((key, value));
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
            _ => Err(format!("Unexpected token: {:?}", t.kind)),
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
        }
    }
}

