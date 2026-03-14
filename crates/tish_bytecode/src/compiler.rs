//! AST to bytecode compiler.

use std::collections::HashMap;
use std::sync::Arc;

use tish_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, DestructElement, DestructPattern, Expr,
    Literal, MemberProp, ObjectProp, Program, Span, Statement,
};

use crate::chunk::{Chunk, Constant};
use crate::encoding::{binop_to_u8, compound_op_to_u8, unaryop_to_u8};
use crate::opcode::Opcode;

enum SimpleMapResult {
    Identity,
    BinOp(BinOp, Constant, bool), // op, constant, param_on_left
}

fn literal_to_constant(expr: &Expr) -> Option<Constant> {
    if let Expr::Literal { value, .. } = expr {
        Some(match value {
            Literal::Number(n) => Constant::Number(*n),
            Literal::String(s) => Constant::String(Arc::clone(s)),
            Literal::Bool(b) => Constant::Bool(*b),
            Literal::Null => Constant::Null,
        })
    } else {
        None
    }
}

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

/// Loop boundary for break/continue.
struct LoopInfo {
    break_patches: Vec<usize>,
    continue_patches: Vec<usize>,
}

/// Switch boundary: break exits the switch.
struct SwitchInfo {
    break_patches: Vec<usize>,
}

struct Compiler<'a> {
    chunk: &'a mut Chunk,
    /// Current scope: variable name -> (depth, is_captured). Depth 0 = local.
    scope: Vec<HashMap<Arc<str>, bool>>,
    /// Stack of loop info for break/continue.
    loop_stack: Vec<LoopInfo>,
    switch_stack: Vec<SwitchInfo>,
}

impl<'a> Compiler<'a> {
    fn new(chunk: &'a mut Chunk) -> Self {
        Self {
            chunk,
            scope: vec![HashMap::new()],
            loop_stack: Vec::new(),
            switch_stack: Vec::new(),
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

    /// Detect property-based numeric sort: (a, b) => a.prop - b.prop or (a, b) => b.prop - a.prop.
    /// Returns Some((prop_name, asc)) or None.
    fn detect_property_sort_comparator(expr: &Expr) -> Option<(Arc<str>, bool)> {
        if let Expr::ArrowFunction { params, body, .. } = expr {
            if params.len() != 2 {
                return None;
            }
            let param_a = params[0].name.as_ref();
            let param_b = params[1].name.as_ref();
            let body_expr = match body {
                ArrowBody::Expr(e) => e.as_ref(),
                ArrowBody::Block(stmt) => {
                    if let Statement::ExprStmt { expr: e, .. } = stmt.as_ref() {
                        e
                    } else {
                        return None;
                    }
                }
            };
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = body_expr
            {
                if let (Expr::Member { object: lo, prop: MemberProp::Name(p), .. }, Expr::Member { object: ro, prop: MemberProp::Name(pr), .. }) =
                    (left.as_ref(), right.as_ref())
                {
                    if p != pr {
                        return None;
                    }
                    if let (Expr::Ident { name: ln, .. }, Expr::Ident { name: rn, .. }) =
                        (lo.as_ref(), ro.as_ref())
                    {
                        if ln.as_ref() == param_a && rn.as_ref() == param_b {
                            return Some((Arc::clone(p), true));
                        }
                        if ln.as_ref() == param_b && rn.as_ref() == param_a {
                            return Some((Arc::clone(p), false));
                        }
                    }
                }
            }
        }
        None
    }

    /// Detect numeric sort comparator: (a, b) => a - b (asc) or (a, b) => b - a (desc).
    fn detect_numeric_sort_comparator(expr: &Expr) -> Option<bool> {
        if let Expr::ArrowFunction { params, body, .. } = expr {
            if params.len() != 2 {
                return None;
            }
            let param_a = params[0].name.as_ref();
            let param_b = params[1].name.as_ref();
            let body_expr = match body {
                ArrowBody::Expr(e) => e.as_ref(),
                ArrowBody::Block(stmt) => {
                    if let Statement::ExprStmt { expr: e, .. } = stmt.as_ref() {
                        e
                    } else {
                        return None;
                    }
                }
            };
            if let Expr::Binary {
                left,
                op: BinOp::Sub,
                right,
                ..
            } = body_expr
            {
                if let (Expr::Ident { name: left_name, .. }, Expr::Ident { name: right_name, .. }) =
                    (left.as_ref(), right.as_ref())
                {
                    if left_name.as_ref() == param_a && right_name.as_ref() == param_b {
                        return Some(true);
                    }
                    if left_name.as_ref() == param_b && right_name.as_ref() == param_a {
                        return Some(false);
                    }
                }
            }
        }
        None
    }

    /// Detect simple map callback: x => x (identity) or x => x op const / x => const op x.
    /// Returns SimpleMapResult for map optimization.
    fn detect_simple_map_callback(expr: &Expr) -> Option<SimpleMapResult> {
        let (params, body) = match expr {
            Expr::ArrowFunction { params, body, .. } => (params, body),
            _ => return None,
        };
        if params.len() != 1 {
            return None;
        }
        let param_name = params[0].name.as_ref();
        let expr_ref: &Expr = match body {
            ArrowBody::Expr(e) => e.as_ref(),
            ArrowBody::Block(stmt) => {
                let s = stmt.as_ref();
                if let Statement::Return { value: Some(ref e), .. } = s {
                    e
                } else if let Statement::ExprStmt { expr: ref e, .. } = s {
                    e
                } else {
                    return None;
                }
            }
        };
        // Identity: x => x
        if let Expr::Ident { name, .. } = expr_ref {
            if name.as_ref() == param_name {
                return Some(SimpleMapResult::Identity);
            }
        }
        // Binary: x op const or const op x
        if let Expr::Binary { left, op, right, .. } = expr_ref {
            let left_is_param = matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let right_is_param = matches!(right.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let left_is_literal = matches!(left.as_ref(), Expr::Literal { .. });
            let right_is_literal = matches!(right.as_ref(), Expr::Literal { .. });
            if left_is_param && right_is_literal {
                if let Some(c) = literal_to_constant(right.as_ref()) {
                    return Some(SimpleMapResult::BinOp(*op, c, true));
                }
            }
            if left_is_literal && right_is_param {
                if let Some(c) = literal_to_constant(left.as_ref()) {
                    return Some(SimpleMapResult::BinOp(*op, c, false));
                }
            }
        }
        None
    }

    /// Detect simple filter callback: x => x op const or x => const op x (comparison that returns bool).
    fn detect_simple_filter_callback(expr: &Expr) -> Option<(BinOp, Constant, bool)> {
        let (params, body) = match expr {
            Expr::ArrowFunction { params, body, .. } => (params, body),
            _ => return None,
        };
        if params.len() != 1 {
            return None;
        }
        let param_name = params[0].name.as_ref();
        let expr_ref: &Expr = match body {
            ArrowBody::Expr(e) => e.as_ref(),
            ArrowBody::Block(stmt) => {
                let s = stmt.as_ref();
                if let Statement::Return { value: Some(ref e), .. } = s {
                    e
                } else if let Statement::ExprStmt { expr: ref e, .. } = s {
                    e
                } else {
                    return None;
                }
            }
        };
        if let Expr::Binary { left, op, right, .. } = expr_ref {
            if !matches!(op, BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::And | BinOp::Or) {
                return None;
            }
            let left_is_param = matches!(left.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let right_is_param = matches!(right.as_ref(), Expr::Ident { name, .. } if name.as_ref() == param_name);
            let left_is_literal = matches!(left.as_ref(), Expr::Literal { .. });
            let right_is_literal = matches!(right.as_ref(), Expr::Literal { .. });
            if left_is_param && right_is_literal {
                if let Some(c) = literal_to_constant(right.as_ref()) {
                    return Some((*op, c, true));
                }
            }
            if left_is_literal && right_is_param {
                if let Some(c) = literal_to_constant(left.as_ref()) {
                    return Some((*op, c, false));
                }
            }
        }
        None
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
                self.compile_statement(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
                if let Some(else_s) = else_branch {
                    self.compile_statement(else_s)?;
                }
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Statement::While { cond, body, .. } => {
                let start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                });
                self.compile_expr(cond)?;
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                // JumpIfFalse already pops condition when taking body
                self.compile_statement(body)?;
                let jump_back_dist = self.chunk.code.len() + 3 - start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
                let info = self.loop_stack.pop().unwrap();
                for p in info.continue_patches {
                    self.patch_jump(p, start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
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
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                });
                self.compile_statement(body)?;
                let update_start = self.chunk.code.len();
                if let Some(u) = update {
                    self.compile_expr(u)?;
                    self.emit(Opcode::Pop);
                }
                let info = self.loop_stack.pop().unwrap();
                for p in info.continue_patches {
                    self.patch_jump(p, update_start);
                }
                let jump_back_dist = self.chunk.code.len() + 3 - cond_start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                self.scope.pop();
            }
            Statement::ForOf { name, iterable, body, .. } => {
                self.compile_expr(iterable)?;
                self.scope.push(HashMap::new());
                let arr_name = Arc::from("__forof_arr__");
                let i_name = Arc::from("__forof_i__");
                let len_name = Arc::from("__forof_len__");
                let arr_idx = self.name_idx(&arr_name);
                let i_idx = self.name_idx(&i_name);
                let len_idx = self.name_idx(&len_name);
                let name_idx = self.name_idx(name);
                self.emit_u16(Opcode::StoreVar, arr_idx);
                self.scope.last_mut().unwrap().insert(arr_name.clone(), false);
                self.emit_u16(Opcode::LoadVar, arr_idx);
                let len_name_idx = self.name_idx(&Arc::from("length"));
                self.emit_u16(Opcode::GetMember, len_name_idx);
                self.emit_u16(Opcode::StoreVar, len_idx);
                self.scope.last_mut().unwrap().insert(len_name.clone(), false);
                let zero_idx = self.constant_idx(Constant::Number(0.0));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(zero_idx);
                self.emit_u16(Opcode::StoreVar, i_idx);
                self.scope.last_mut().unwrap().insert(i_name.clone(), false);
                let loop_start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                });
                self.emit_u16(Opcode::LoadVar, arr_idx);
                self.emit_u16(Opcode::LoadVar, i_idx);
                self.emit(Opcode::GetIndex);
                self.emit_u16(Opcode::StoreVar, name_idx);
                self.scope.last_mut().unwrap().insert(Arc::clone(name), false);
                self.compile_statement(body)?;
                self.emit_u16(Opcode::LoadVar, i_idx);
                let one_idx = self.constant_idx(Constant::Number(1.0));
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one_idx);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit_u16(Opcode::StoreVar, i_idx);
                self.emit_u16(Opcode::LoadVar, i_idx);
                self.emit_u16(Opcode::LoadVar, len_idx);
                self.emit_u8(Opcode::BinOp, 10);
                let jump_out = self.emit_jump(Opcode::JumpIfFalse);
                let jump_back_dist = self.chunk.code.len() + 3 - loop_start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_out, end);
                let info = self.loop_stack.pop().unwrap();
                for p in info.continue_patches {
                    self.patch_jump(p, loop_start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
                self.scope.pop();
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
                let pos = self.emit_jump(Opcode::Jump);
                if let Some(sw) = self.switch_stack.last_mut() {
                    sw.break_patches.push(pos);
                } else if let Some(lo) = self.loop_stack.last_mut() {
                    lo.break_patches.push(pos);
                } else {
                    return Err(CompileError {
                        message: "break not inside a loop or switch".to_string(),
                    });
                }
            }
            Statement::Continue { .. } => {
                let pos = self.emit_jump(Opcode::Jump);
                self.loop_stack.last_mut().ok_or_else(|| CompileError {
                    message: "continue not inside a loop".to_string(),
                })?.continue_patches.push(pos);
            }
            Statement::FunDecl {
                name,
                params,
                body,
                rest_param,
                async_: _,
                ..
            } => {
                let mut inner = Chunk::new();
                let mut param_names: Vec<Arc<str>> =
                    params.iter().map(|p| Arc::clone(&p.name)).collect();
                if let Some(rp) = rest_param {
                    param_names.push(rp.name.clone());
                    inner.rest_param_index = param_names.len() as u16 - 1;
                }
                for p in &param_names {
                    inner.add_name(Arc::clone(p));
                }
                inner.param_count = param_names.len() as u16;
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
            Statement::DoWhile { body, cond, .. } => {
                let start = self.chunk.code.len();
                self.loop_stack.push(LoopInfo {
                    break_patches: Vec::new(),
                    continue_patches: Vec::new(),
                });
                self.compile_statement(body)?;
                let cond_start = self.chunk.code.len();
                self.compile_expr(cond)?;
                let jump_back = self.emit_jump(Opcode::JumpIfFalse);
                let jump_back_dist = self.chunk.code.len() + 3 - start;
                self.emit_u16(Opcode::JumpBack, jump_back_dist as u16);
                let end = self.chunk.code.len();
                self.patch_jump(jump_back, end);
                let info = self.loop_stack.pop().unwrap();
                for p in info.continue_patches {
                    self.patch_jump(p, cond_start);
                }
                for p in info.break_patches {
                    self.patch_jump(p, end);
                }
            }
            Statement::Switch { expr, cases, default_body, .. } => {
                self.switch_stack.push(SwitchInfo {
                    break_patches: Vec::new(),
                });
                self.compile_expr(expr)?;
                self.emit(Opcode::Dup);
                let mut end_patches = Vec::new();
                for (case_expr, case_body) in cases {
                    self.emit(Opcode::Dup);
                    if let Some(ce) = case_expr {
                        self.compile_expr(ce)?;
                        self.emit_u8(Opcode::BinOp, 8);
                        let jump_next = self.emit_jump(Opcode::JumpIfFalse);
                        // JumpIfFalse already pops the match result when taking this case
                        self.compile_statement(&Statement::Block {
                            statements: case_body.clone(),
                            span: Span {
                                start: (0, 0),
                                end: (0, 0),
                            },
                        })?;
                        let jump_end = self.emit_jump(Opcode::Jump);
                        end_patches.push(jump_end);
                        self.patch_jump(jump_next, self.chunk.code.len());
                    } else {
                        self.emit(Opcode::Pop);
                        self.compile_statement(&Statement::Block {
                            statements: case_body.clone(),
                            span: Span {
                                start: (0, 0),
                                end: (0, 0),
                            },
                        })?;
                    }
                }
                if let Some(body) = default_body {
                    self.emit(Opcode::Pop);
                    self.compile_statement(&Statement::Block {
                        statements: body.clone(),
                        span: Span {
                            start: (0, 0),
                            end: (0, 0),
                        },
                    })?;
                } else {
                    self.emit(Opcode::Pop);
                }
                for p in end_patches {
                    self.patch_jump(p, self.chunk.code.len());
                }
                let sw = self.switch_stack.pop().unwrap();
                for p in sw.break_patches {
                    self.patch_jump(p, self.chunk.code.len());
                }
            }
            Statement::Throw { value, .. } => {
                self.compile_expr(value)?;
                self.emit(Opcode::Throw);
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                let catch_offset_pos = self.chunk.code.len();
                self.emit(Opcode::EnterTry);
                self.chunk.write_u16(0);
                self.compile_statement(body)?;
                self.emit(Opcode::ExitTry);
                let jump_over_catch = self.emit_jump(Opcode::Jump);
                let catch_start = self.chunk.code.len();
                if let Some(catch_stmt) = catch_body {
                    if let Some(param) = catch_param {
                        let param_idx = self.name_idx(param);
                        self.emit_u16(Opcode::StoreVar, param_idx);
                        self.scope
                            .last_mut()
                            .unwrap()
                            .insert(Arc::clone(param), false);
                    } else {
                        self.emit(Opcode::Pop);
                    }
                    self.compile_statement(catch_stmt)?;
                } else {
                    self.emit(Opcode::Throw);
                }
                let after_catch = self.chunk.code.len();
                self.patch_jump(jump_over_catch, after_catch);
                if let Some(finally) = finally_body {
                    self.compile_statement(finally)?;
                }
                let catch_offset = catch_start.wrapping_sub(catch_offset_pos).wrapping_sub(3) as u16;
                self.chunk.code[catch_offset_pos + 1] = (catch_offset >> 8) as u8;
                self.chunk.code[catch_offset_pos + 2] = (catch_offset & 0xff) as u8;
            }
            Statement::Import { .. } | Statement::Export { .. } => {
                return Err(CompileError {
                    message: "Import/Export not supported in bytecode".to_string(),
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
                    self.emit(Opcode::GetIndex); // GetIndex pops obj, index and uses get_member
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
                // Fast path: arr.sort((a,b)=>a-b) or arr.sort((a,b)=>b-a) -> ArraySortNumeric
                if !args.iter().any(|a| matches!(a, CallArg::Spread(_)))
                    && args.len() == 1
                    && matches!(args[0], CallArg::Expr(_))
                {
                    if let (Expr::Member { object, prop: MemberProp::Name(key), optional: false, .. }, CallArg::Expr(cmp_expr)) =
                        (callee.as_ref(), &args[0])
                    {
                        if key.as_ref() == "sort" {
                            if let Some(ascending) = Self::detect_numeric_sort_comparator(cmp_expr) {
                                self.compile_expr(object)?;
                                self.emit_u8(Opcode::ArraySortNumeric, if ascending { 0 } else { 1 });
                                return Ok(());
                            }
                            if let Some((prop, ascending)) = Self::detect_property_sort_comparator(cmp_expr) {
                                self.compile_expr(object)?;
                                let prop_idx = self.constant_idx(Constant::String(prop));
                                self.emit(Opcode::ArraySortByProperty);
                                self.chunk.write_u16(prop_idx);
                                self.chunk.write_u16(if ascending { 0 } else { 1 });
                                return Ok(());
                            }
                        }
                        if key.as_ref() == "map" {
                            if let Some(simple) = Self::detect_simple_map_callback(cmp_expr) {
                                self.compile_expr(object)?;
                                match simple {
                                    SimpleMapResult::Identity => {
                                        self.emit(Opcode::ArrayMapIdentity);
                                    }
                                    SimpleMapResult::BinOp(op, c, param_left) => {
                                        let const_idx = self.constant_idx(c);
                                        self.emit(Opcode::ArrayMapBinOp);
                                        self.chunk.write_u8(binop_to_u8(op));
                                        self.chunk.write_u16(const_idx);
                                        self.chunk.write_u8(if param_left { 0 } else { 1 });
                                    }
                                }
                                return Ok(());
                            }
                        }
                        if key.as_ref() == "filter" {
                            if let Some((op, const_val, param_left)) =
                                Self::detect_simple_filter_callback(cmp_expr)
                            {
                                self.compile_expr(object)?;
                                let const_idx = self.constant_idx(const_val);
                                self.emit(Opcode::ArrayFilterBinOp);
                                self.chunk.write_u8(binop_to_u8(op));
                                self.chunk.write_u16(const_idx);
                                self.chunk.write_u8(if param_left { 0 } else { 1 });
                                return Ok(());
                            }
                        }
                    }
                }
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if has_spread {
                    // Build args array [a, ...b, c], then callee, then CallSpread
                    self.emit_u16(Opcode::NewArray, 0);
                    for arg in args {
                        match arg {
                            CallArg::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit_u16(Opcode::NewArray, 1);
                                self.emit(Opcode::ConcatArray);
                            }
                            CallArg::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::ConcatArray);
                            }
                        }
                    }
                    self.compile_expr(callee)?;
                    self.emit(Opcode::CallSpread);
                } else {
                    self.compile_expr(callee)?;
                    for arg in args {
                        if let CallArg::Expr(e) = arg {
                            self.compile_expr(e)?;
                        }
                    }
                    self.emit_u16(Opcode::Call, args.len() as u16);
                }
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                self.compile_expr(object)?;
                if *optional {
                    self.emit(Opcode::Dup);
                    let null_idx = self.constant_idx(Constant::Null);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(null_idx);
                    self.emit_u8(Opcode::BinOp, 8);
                    let jump_to_null = self.emit_jump(Opcode::JumpIfFalse);
                    let jump_to_get_instr = self.chunk.code.len();
                    let jump_to_get = self.emit_jump(Opcode::Jump);
                    self.patch_jump(jump_to_null, jump_to_get_instr);
                    self.emit(Opcode::Pop);
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(null_idx);
                    let jump_end = self.emit_jump(Opcode::Jump);
                    self.patch_jump(jump_to_get, self.chunk.code.len());
                    match prop {
                        MemberProp::Name(key) => {
                            let idx = self.name_idx(key);
                            self.emit_u16(Opcode::GetMemberOptional, idx);
                        }
                        MemberProp::Expr(e) => {
                            self.compile_expr(e)?;
                            self.emit(Opcode::GetIndex);
                        }
                    }
                    self.patch_jump(jump_end, self.chunk.code.len());
                } else {
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
                // JumpIfFalse pops condition when taking then; when taking else it also pops
                self.compile_expr(then_branch)?;
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_else, self.chunk.code.len());
                // no Pop: condition was already popped by JumpIfFalse
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
                let jump_to_right = self.emit_jump(Opcode::JumpIfFalse);
                let jump_end = self.emit_jump(Opcode::Jump);
                self.patch_jump(jump_to_right, self.chunk.code.len());
                self.emit(Opcode::Pop);
                self.compile_expr(right)?;
                self.patch_jump(jump_end, self.chunk.code.len());
            }
            Expr::Array { elements, .. } => {
                let has_spread = elements.iter().any(|e| matches!(e, ArrayElement::Spread(_)));
                if has_spread {
                    // Build array incrementally: start with [], concat each element
                    self.emit_u16(Opcode::NewArray, 0);
                    for elem in elements {
                        match elem {
                            ArrayElement::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit_u16(Opcode::NewArray, 1);
                                self.emit(Opcode::ConcatArray);
                            }
                            ArrayElement::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::ConcatArray);
                            }
                        }
                    }
                } else {
                    for elem in elements {
                        if let ArrayElement::Expr(e) = elem {
                            self.compile_expr(e)?;
                        }
                    }
                    self.emit_u16(Opcode::NewArray, elements.len() as u16);
                }
            }
            Expr::Object { props, .. } => {
                let has_spread = props.iter().any(|p| matches!(p, ObjectProp::Spread(_)));
                if has_spread {
                    self.emit_u16(Opcode::NewObject, 0); // start with {}
                    for prop in props {
                        match prop {
                            ObjectProp::KeyValue(k, v) => {
                                let idx = self.constant_idx(Constant::String(Arc::clone(k)));
                                self.emit(Opcode::LoadConst);
                                self.chunk.write_u16(idx);
                                self.compile_expr(v)?;
                                self.emit_u16(Opcode::NewObject, 1);
                                self.emit(Opcode::MergeObject);
                            }
                            ObjectProp::Spread(expr) => {
                                self.compile_expr(expr)?;
                                self.emit(Opcode::MergeObject);
                            }
                        }
                    }
                } else {
                    for prop in props {
                        if let ObjectProp::KeyValue(k, v) = prop {
                            let idx = self.constant_idx(Constant::String(Arc::clone(k)));
                            self.emit(Opcode::LoadConst);
                            self.chunk.write_u16(idx);
                            self.compile_expr(v)?;
                        }
                    }
                    self.emit_u16(Opcode::NewObject, props.len() as u16);
                }
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
                inner.param_count = param_names.len() as u16;
                let mut inner_comp = Compiler::new(&mut inner);
                inner_comp.scope = vec![param_names
                    .iter()
                    .map(|n| (Arc::clone(n), false))
                    .collect::<HashMap<_, _>>()];
                match body {
                    ArrowBody::Expr(e) => {
                        inner_comp.compile_expr(e)?;
                        inner_comp.emit(Opcode::Return);
                    }
                    ArrowBody::Block(s) => {
                        inner_comp.compile_statement(s)?;
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
                    // Interleave quasis and exprs: quasi[0] + expr[0] + quasi[1] + expr[1] + ... + quasi[n]
                    let first = quasis[0].to_string();
                    let idx = self.constant_idx(Constant::String(Arc::from(first)));
                    self.emit(Opcode::LoadConst);
                    self.chunk.write_u16(idx);
                    for (i, expr) in exprs.iter().enumerate() {
                        self.compile_expr(expr)?;
                        self.emit_u8(Opcode::BinOp, 0); // Add (string concat)
                        let quasi_s = quasis[i + 1].to_string();
                        let qidx = self.constant_idx(Constant::String(Arc::from(quasi_s)));
                        self.emit(Opcode::LoadConst);
                        self.chunk.write_u16(qidx);
                        self.emit_u8(Opcode::BinOp, 0); // Add
                    }
                }
            }
            Expr::PostfixInc { name, .. } => {
                let idx = self.name_idx(name);
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_u16(Opcode::LoadVar, idx);
                self.emit(Opcode::Dup);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit_u16(Opcode::StoreVar, idx);
            }
            Expr::PostfixDec { name, .. } => {
                let idx = self.name_idx(name);
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_u16(Opcode::LoadVar, idx);
                self.emit(Opcode::Dup);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 1);
                self.emit_u16(Opcode::StoreVar, idx);
            }
            Expr::PrefixInc { name, .. } => {
                let idx = self.name_idx(name);
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_u16(Opcode::LoadVar, idx);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 0);
                self.emit(Opcode::Dup);
                self.emit_u16(Opcode::StoreVar, idx);
            }
            Expr::PrefixDec { name, .. } => {
                let idx = self.name_idx(name);
                let one = self.constant_idx(Constant::Number(1.0));
                self.emit_u16(Opcode::LoadVar, idx);
                self.emit(Opcode::LoadConst);
                self.chunk.write_u16(one);
                self.emit_u8(Opcode::BinOp, 1);
                self.emit(Opcode::Dup);
                self.emit_u16(Opcode::StoreVar, idx);
            }
            Expr::CompoundAssign { name, op, value, .. } => {
                let idx = self.name_idx(name);
                self.emit_u16(Opcode::LoadVar, idx);
                self.compile_expr(value)?;
                self.emit_u8(Opcode::BinOp, compound_op_to_u8(*op));
                self.emit(Opcode::Dup);
                self.emit_u16(Opcode::StoreVar, idx);
            }
            Expr::MemberAssign { object, prop, value, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                let idx = self.name_idx(prop);
                self.emit_u16(Opcode::SetMember, idx); // SetMember pops obj, val and pushes val back
            }
            Expr::IndexAssign { object, index, value, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.compile_expr(value)?;
                self.emit(Opcode::Dup); // leave copy for assignment expression result
                self.emit(Opcode::SetIndex);
            }
            Expr::LogicalAssign { .. }
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

/// Compile a Tish program to bytecode (with peephole optimizations).
pub fn compile(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, true)
}

/// Compile without peephole optimizations (for --no-optimize).
pub fn compile_unoptimized(program: &Program) -> Result<Chunk, CompileError> {
    compile_internal(program, false)
}

fn compile_internal(program: &Program, peephole: bool) -> Result<Chunk, CompileError> {
    let mut chunk = Chunk::new();
    let mut compiler = Compiler::new(&mut chunk);
    compiler.compile_program(program)?;
    if peephole {
        crate::peephole::optimize(&mut chunk);
    }
    Ok(chunk)
}
