//! AST to bytecode compiler.

use std::collections::HashMap;
use std::sync::Arc;

use tish_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, DestructElement, DestructPattern, Expr, Literal,
    MemberProp, ObjectProp, Program, Statement, UnaryOp,
};

use crate::chunk::{Chunk, Constant};
use crate::opcode::Opcode;

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CompileError {}

struct Compiler<'a> {
    chunk: &'a mut Chunk,
    /// Current scope: variable name -> (depth, is_captured). Depth 0 = local.
    scope: Vec<HashMap<Arc<str>, bool>>,
}

impl<'a> Compiler<'a> {
    fn new(chunk: &'a mut Chunk) -> Self {
        Self {
            chunk,
            scope: vec![HashMap::new()],
        }
    }

    fn name_idx(&mut self, name: &Arc<str>) -> u16 {
        self.chunk.add_name(Arc::clone(name))
    }

    fn constant_idx(&mut self, c: Constant) -> u16 {
        self.chunk.add_constant(c)
    }

    fn emit(&mut self, op: Opcode) {
        self.chunk.write_u8(op as u8);
    }

    fn emit_u8(&mut self, op: Opcode, v: u8) {
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(v as u16);
    }

    fn emit_u16(&mut self, op: Opcode, v: u16) {
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(v);
    }

    fn emit_jump(&mut self, op: Opcode) -> usize {
        let pos = self.chunk.code.len();
        self.chunk.write_u8(op as u8);
        self.chunk.write_u16(0); // placeholder
        pos + 1
    }

    fn patch_jump(&mut self, patch_pos: usize, target: usize) {
        let jump_offset = target - (patch_pos + 2);
        let bytes = (jump_offset as u16).to_be_bytes();
        self.chunk.code[patch_pos] = bytes[0];
        self.chunk.code[patch_pos + 1] = bytes[1];
    }

    fn compile_program(&mut self, program: &Program) -> Result<(), CompileError> {
        for stmt in &program.statements {
            self.compile_statement(stmt)?;
        }
        let idx = self.constant_idx(Constant::Null);
        self.emit(Opcode::LoadConst);
        self.chunk.write_u16(idx);
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } => {
                self.scope.push(HashMap::new());
                for s in statements {
                    self.compile_statement(s)?;
                }
                self.scope.pop();
            }
            Statement::VarDecl {
                name,
                init,
                mutable: _,
                ..
            } => {
                if let Some(expr) = init {
                    self.compile_expr(expr)?;
                } else {
                    let idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::StoreVar, idx);
                self.scope
                    .last_mut()
                    .unwrap()
                    .insert(Arc::clone(name), false);
            }
            Statement::VarDeclDestructure { pattern, init, .. } => {
                self.compile_expr(init)?;
                self.compile_destructure(pattern, false)?;
            }
            Statement::ExprStmt { expr, .. } => {
                self.compile_expr(expr)?;
                self.emit(Opcode::Pop);
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.compile_expr(cond)?;
                let jump_else = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::Pop); // pop condition if we take then
                self.compile_statement(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
                self.emit(Opcode::Pop); // pop condition if we took else
                if let Some(else_s) = else_branch {
                    self.compile_statement(else_s)?;
                }
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Statement::While { cond, body, .. } => {
                let start = self.chunk.code.len();
                self.compile_expr(cond)?;
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::Pop);
                self.compile_statement(body)?;
                let jump_back_dist = self.chunk.code.len() + 3 - start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                self.patch_jump(jump_out, self.chunk.code.len());
                self.emit(Opcode::Pop);
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                self.scope.push(HashMap::new());
                if let Some(i) = init {
                    self.compile_statement(i)?;
                }
                let cond_start = self.chunk.code.len();
                if let Some(c) = cond {
                    self.compile_expr(c)?;
                } else {
                    let idx = self.constant_idx(Constant::Bool(true));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::Pop);
                let _body_start = self.chunk.code.len();
                self.compile_statement(body)?;
                if let Some(u) = update {
                    self.compile_expr(u)?;
                    self.emit(Opcode::Pop);
                }
                let jump_back_dist = self.chunk.code.len() + 3 - cond_start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                self.patch_jump(jump_out, self.chunk.code.len());
                self.emit(Opcode::Pop);
                self.scope.pop();
            }
            Statement::ForOf { .. } => {
                return Err(CompileError {
                    message: "for-of not yet supported in bytecode".to_string(),
                });
            }
            Statement::Return { value, .. } => {
                if let Some(v) = value {
                    self.compile_expr(v)?;
                } else {
                    let idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                }
                self.emit(Opcode::Return);
            }
            Statement::Break { .. } => {
                return Err(CompileError {
                    message: "break not yet supported in bytecode".to_string(),
                });
            }
            Statement::Continue { .. } => {
                return Err(CompileError {
                    message: "continue not yet supported in bytecode".to_string(),
                });
            }
            Statement::FunDecl {
                name,
                params,
                body,
                rest_param,
                async_: _,
                ..
            } => {
                if rest_param.is_some() {
                    return Err(CompileError {
                        message: "rest parameters not yet supported in bytecode".to_string(),
                    });
                }
                let mut inner = Chunk::new();
                let param_names: Vec<Arc<str>> =
                    params.iter().map(|p| Arc::clone(&p.name)).collect();
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                let mut inner_comp = Compiler::new(&mut inner);
                inner_comp.scope = vec![param_names
                    .iter()
                    .map(|n| (Arc::clone(n), false))
                    .collect::<HashMap<_, _>>()];
                inner_comp.compile_statement(body)?;
                inner_comp.emit(Opcode::LoadConst);
                let idx = inner_comp.constant_idx(Constant::Null);
                inner_comp.chunk.write_u16(idx);
                inner_comp.emit(Opcode::Return);
                let nested_idx = self.chunk.add_nested(inner);
                self.emit(Opcode::LoadConst);
                let idx = self.constant_idx(Constant::Closure(nested_idx));
                self.chunk.write_u16(idx);
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::StoreVar, idx);
                self.scope.last_mut().unwrap().insert(Arc::clone(name), false);
            }
            Statement::Switch { .. }
            | Statement::DoWhile { .. }
            | Statement::Throw { .. }
            | Statement::Try { .. }
            | Statement::Import { .. }
            | Statement::Export { .. } => {
                return Err(CompileError {
                    message: format!(
                        "Statement not yet supported in bytecode: {:?}",
                        std::mem::discriminant(stmt)
                    ),
                });
            }
        }
        Ok(())
    }

    fn compile_destructure(
        &mut self,
        pattern: &DestructPattern,
        mutable: bool,
    ) -> Result<(), CompileError> {
        match pattern {
            DestructPattern::Array(elements) => {
                for (i, elem) in elements.iter().enumerate() {
                    match elem {
                        Some(DestructElement::Ident(name)) => {
                            self.emit(Opcode::Dup);
                            let idx = self.constant_idx(Constant::Number(i as f64));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.emit(Opcode::GetIndex);
                            let idx = self.name_idx(name);
                            self.emit_u16(Opcode::StoreVar, idx);
                            self.scope
                                .last_mut()
                                .unwrap()
                                .insert(Arc::clone(name), false);
                        }
                        _ => {
                            return Err(CompileError {
                                message: "Complex destructuring not yet supported".to_string(),
                            });
                        }
                    }
                }
                self.emit(Opcode::Pop);
            }
            DestructPattern::Object(props) => {
                for prop in props {
                    self.emit(Opcode::Dup);
                    let key_idx = self.constant_idx(Constant::String(Arc::clone(&prop.key)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(key_idx);
                    self.emit(Opcode::GetMember);
                    match &prop.value {
                        DestructElement::Ident(name) => {
                            let idx = self.name_idx(name);
                            self.emit_u16(Opcode::StoreVar, idx);
                            if mutable {
                                self.scope
                                    .last_mut()
                                    .unwrap()
                                    .insert(Arc::clone(name), false);
                            }
                        }
                        _ => {
                            return Err(CompileError {
                                message: "Nested object destructuring not yet supported"
                                    .to_string(),
                            });
                        }
                    }
                }
                self.emit(Opcode::Pop);
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Literal { value, .. } => {
                let c = match value {
                    Literal::Number(n) => Constant::Number(*n),
                    Literal::String(s) => Constant::String(Arc::clone(s)),
                    Literal::Bool(b) => Constant::Bool(*b),
                    Literal::Null => Constant::Null,
                };
                let idx = self.constant_idx(c);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
            }
            Expr::Ident { name, .. } => {
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::LoadVar, idx);
            }
            Expr::Binary { left, op, right, .. } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit_u8(Opcode::BinOp, binop_to_u8(*op));
            }
            Expr::Unary { op, operand, .. } => {
                self.compile_expr(operand)?;
                self.emit_u8(Opcode::UnaryOp, unaryop_to_u8(*op));
            }
            Expr::Call { callee, args, .. } => {
                self.compile_expr(callee)?;
                let mut argc = 0u16;
                for arg in args {
                    match arg {
                        CallArg::Expr(e) => {
                            self.compile_expr(e)?;
                            argc += 1;
                        }
                        CallArg::Spread(_) => {
                            return Err(CompileError {
                                message: "Spread in call not yet supported in bytecode".to_string(),
                            });
                        }
                    }
                }
                self.emit_u16(Opcode::Call, argc);
            }
            Expr::Member {
                object,
                prop,
                optional: _,
                ..
            } => {
                self.compile_expr(object)?;
                match prop {
                    MemberProp::Name(key) => {
                        let idx = self.name_idx(key);
                        self.emit_u16(Opcode::GetMember, idx);
                    }
                    MemberProp::Expr(e) => {
                        self.compile_expr(e)?;
                        self.emit(Opcode::GetIndex);
                    }
                }
            }
            Expr::Index { object, index, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Opcode::GetIndex);
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.compile_expr(cond)?;
                let jump_else = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::Pop);
                self.compile_expr(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
                self.emit(Opcode::Pop);
                self.compile_expr(else_branch)?;
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.compile_expr(left)?;
                self.emit(Opcode::Dup);
                let idx = self.constant_idx(Constant::Null);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
                self.emit_u8(Opcode::BinOp, binop_to_u8(BinOp::StrictNe));
                let jump_skip = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::Pop); // pop left
                self.compile_expr(right)?;
                self.patch_jump(jump_skip, self.chunk.code.len());
                self.emit(Opcode::Pop); // pop condition
            }
            Expr::Array { elements, .. } => {
                for elem in elements {
                    match elem {
                        ArrayElement::Expr(e) => self.compile_expr(e)?,
                        ArrayElement::Spread(_) => {
                            return Err(CompileError {
                                message: "Spread in array not yet supported in bytecode"
                                    .to_string(),
                            });
                        }
                    }
                }
                self.emit_u16(Opcode::NewArray, elements.len() as u16);
            }
            Expr::Object { props, .. } => {
                for prop in props {
                    match prop {
                        ObjectProp::KeyValue(k, v) => {
                            let idx = self.constant_idx(Constant::String(Arc::clone(k)));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.compile_expr(v)?;
                        }
                        ObjectProp::Spread(_) => {
                            return Err(CompileError {
                                message: "Spread in object not yet supported in bytecode"
                                    .to_string(),
                            });
                        }
                    }
                }
                self.emit_u16(Opcode::NewObject, props.len() as u16);
            }
            Expr::Assign { name, value, .. } => {
                self.compile_expr(value)?;
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::StoreVar, idx);
                self.emit_u16(Opcode::LoadVar, idx); // assign yields value
            }
            Expr::TypeOf { operand, .. } => {
                let typeof_idx = self.name_idx(&Arc::from("typeof"));
                self.emit_u16(Opcode::LoadGlobal, typeof_idx);
                self.compile_expr(operand)?;
                self.emit_u16(Opcode::Call, 1);
            }
            Expr::ArrowFunction { params, body, .. } => {
                let mut inner = Chunk::new();
                let param_names: Vec<Arc<str>> =
                    params.iter().map(|p| Arc::clone(&p.name)).collect();
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                let mut inner_comp = Compiler::new(&mut inner);
                inner_comp.scope = vec![param_names
                    .iter()
                    .map(|n| (Arc::clone(n), false))
                    .collect::<HashMap<_, _>>()];
                match body {
                    ArrowBody::Expr(e) => {
                        inner_comp.compile_expr(&e)?;
                        inner_comp.emit(Opcode::Return);
                    }
                    ArrowBody::Block(s) => {
                        inner_comp.compile_statement(&s)?;
                        let idx = inner_comp.constant_idx(Constant::Null);
                        inner_comp.emit(Opcode::LoadConst);
                        inner_comp.chunk.write_u16(idx);
                        inner_comp.emit(Opcode::Return);
                    }
                }
                let nested_idx = self.chunk.add_nested(inner);
                let idx = self.constant_idx(Constant::Closure(nested_idx));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(idx);
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                if exprs.is_empty() {
                    let s = quasis[0].to_string();
                    let idx = self.constant_idx(Constant::String(Arc::from(s)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                } else {
                    self.compile_expr(&exprs[0])?;
                    for (i, q) in quasis.iter().enumerate().skip(1) {
                        let s = q.to_string();
                        let idx = self.constant_idx(Constant::String(Arc::from(s)));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(idx);
                        self.emit_u8(Opcode::BinOp, 0); // Add (string concat)
                        if i < exprs.len() {
                            self.compile_expr(&exprs[i])?;
                            self.emit_u8(Opcode::BinOp, 0); // Add
                        }
                    }
                }
            }
            Expr::PostfixInc { .. }
            | Expr::PostfixDec { .. }
            | Expr::PrefixInc { .. }
            | Expr::PrefixDec { .. }
            | Expr::CompoundAssign { .. }
            | Expr::LogicalAssign { .. }
            | Expr::MemberAssign { .. }
            | Expr::IndexAssign { .. }
            | Expr::Await { .. }
            | Expr::JsxElement { .. }
            | Expr::JsxFragment { .. }
            | Expr::NativeModuleLoad { .. } => {
                return Err(CompileError {
                    message: format!(
                        "Expression not yet supported in bytecode: {:?}",
                        std::mem::discriminant(expr)
                    ),
                });
            }
        }
        Ok(())
    }
}

fn binop_to_u8(op: BinOp) -> u8 {
    use tish_ast::BinOp::*;
    match op {
        Add => 0,
        Sub => 1,
        Mul => 2,
        Div => 3,
        Mod => 4,
        Pow => 5,
        Eq => 6,
        Ne => 7,
        StrictEq => 8,
        StrictNe => 9,
        Lt => 10,
        Le => 11,
        Gt => 12,
        Ge => 13,
        And => 14,
        Or => 15,
        BitAnd => 16,
        BitOr => 17,
        BitXor => 18,
        Shl => 19,
        Shr => 20,
        In => 21,
    }
}

fn unaryop_to_u8(op: UnaryOp) -> u8 {
    use tish_ast::UnaryOp::*;
    match op {
        Not => 0,
        Neg => 1,
        Pos => 2,
        BitNot => 3,
        Void => 4,
    }
}

/// Compile a Tish program to bytecode.
pub fn compile(program: &Program) -> Result<Chunk, CompileError> {
    let mut chunk = Chunk::new();
    let mut compiler = Compiler::new(&mut chunk);
    compiler.compile_program(program)?;
    Ok(chunk)
}
