//! Code generation: AST -> JavaScript source.

use std::path::{Path, PathBuf};

use sourcemap::SourceMapBuilder;
use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    ExportDeclaration, Expr, FunParam, ImportSpecifier, Literal, LogicalAssignOp, MemberProp,
    ObjectProp, Program, Statement, UnaryOp,
};

use crate::error::CompileError;

/// Default module specifier the JSX runtime (`h` / `Fragment`) is auto-imported from (issue #291).
/// Matches the repo convention (`import { h } from "lattish"`); overridable via `--jsx-import-source`.
pub const DEFAULT_JSX_IMPORT_SOURCE: &str = "lattish";

/// JS output mode. `Bundle` flattens every module into one file (imports/exports are resolved away
/// by `merge_modules`). `Esm` emits one file per module with real ES `import`/`export` statements so
/// a bundler (Vite/Rollup) can tree-shake and code-split. See issue #177's sibling, #282.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmitMode {
    Bundle,
    Esm,
}

/// How ESM import specifiers are rewritten. `Disk` (production `--format esm`) rewrites `.tish`
/// specifiers to their sibling `.js` output paths and resolves bare specifiers to relative paths
/// into the emitted module tree. `ViteDev` keeps relative `.tish` specifiers and bare specifiers
/// as-is so Vite's `resolveId`/`load` re-enters the plugin per module (in-graph HMR, issue #284).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRewrite {
    Disk,
    ViteDev,
}

struct Codegen {
    output: String,
    indent: usize,
    in_async: bool,
    emit_mode: EmitMode,
    /// ESM only: how import specifiers are rewritten (disk `.js` paths vs Vite-dev `.tish`).
    import_rewrite: ImportRewrite,
    /// ESM only: the absolute path of the module being emitted (the importer), used to rewrite
    /// relative/bare import specifiers to sibling `.js` output paths.
    module_path: PathBuf,
    /// ESM only: the project root, so import targets can be located relative to the output tree.
    project_root: PathBuf,
    /// ESM only: the module specifier the JSX runtime (`h` / `Fragment`) is auto-imported from when a
    /// module uses JSX but doesn't import them itself (issue #291). Defaults to `lattish`.
    jsx_import_source: String,
}

fn stmt_terminates_switch(stmt: Option<&Statement>) -> bool {
    matches!(
        stmt,
        Some(Statement::Break { .. })
            | Some(Statement::Return { .. })
            | Some(Statement::Throw { .. })
    )
}

impl Codegen {
    fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            in_async: false,
            emit_mode: EmitMode::Bundle,
            import_rewrite: ImportRewrite::Disk,
            module_path: PathBuf::new(),
            project_root: PathBuf::new(),
            jsx_import_source: DEFAULT_JSX_IMPORT_SOURCE.to_string(),
        }
    }

    fn new_esm(
        module_path: PathBuf,
        project_root: PathBuf,
        import_rewrite: ImportRewrite,
        jsx_import_source: String,
    ) -> Self {
        Self {
            output: String::new(),
            indent: 0,
            in_async: false,
            emit_mode: EmitMode::Esm,
            import_rewrite,
            module_path,
            project_root,
            jsx_import_source,
        }
    }

    /// ECMAScript does not allow `if (c) const x = 1` / `while (c) let y = 2` without a block.
    fn stmt_needs_braces_in_js_control_head(stmt: &Statement) -> bool {
        matches!(
            stmt,
            Statement::VarDecl { .. } | Statement::VarDeclDestructure { .. }
        )
    }

    fn emit_js_control_body(&mut self, body: &Statement) -> Result<(), CompileError> {
        if Self::stmt_needs_braces_in_js_control_head(body) {
            self.writeln("{");
            self.indent += 1;
            self.emit_statement(body)?;
            self.indent -= 1;
            self.writeln("}");
        } else {
            self.indent += 1;
            self.emit_statement(body)?;
            self.indent -= 1;
        }
        Ok(())
    }

    fn indent_str(&self) -> String {
        "  ".repeat(self.indent)
    }

    fn write(&mut self, s: &str) {
        self.output.push_str(s);
    }

    fn writeln(&mut self, s: &str) {
        self.output.push_str(&self.indent_str());
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn output_line(&self) -> u32 {
        self.output
            .as_bytes()
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32
    }

    fn escape_ident(s: &str) -> String {
        let s = s.to_string();
        if s == "await" || s == "default" {
            format!("_{}", s)
        } else {
            s
        }
    }

    fn emit_program(
        &mut self,
        program: &Program,
        map_sources: Option<(&[PathBuf], &Path)>,
        map_builder: Option<&mut SourceMapBuilder>,
    ) -> Result<(), CompileError> {
        self.write("// Generated by tishlang_compile_js\n");
        self.emit_jsx_runtime_auto_import(program)?;
        match (map_sources, map_builder) {
            (Some((srcs, root)), Some(sm)) => {
                for (i, stmt) in program.statements.iter().enumerate() {
                    if i < srcs.len() {
                        let dst_line = self.output_line();
                        let sp = stmt.span();
                        let src_line = sp.start.0.saturating_sub(1) as u32;
                        let src_col = sp.start.1.saturating_sub(1) as u32;
                        let abs = srcs[i].as_path();
                        let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                        let abs_canon = abs.canonicalize().unwrap_or_else(|_| abs.to_path_buf());
                        let rel = abs_canon
                            .strip_prefix(&root_canon)
                            .unwrap_or(abs_canon.as_path());
                        let rel_str = rel.to_string_lossy();
                        sm.add(
                            dst_line,
                            0,
                            src_line,
                            src_col,
                            Some(rel_str.as_ref()),
                            None,
                            false,
                        );
                    }
                    self.emit_statement(stmt)?;
                }
            }
            _ => {
                for stmt in &program.statements {
                    self.emit_statement(stmt)?;
                }
            }
        }
        Ok(())
    }

    /// Auto-import the JSX runtime (`h` / `Fragment`) for ESM modules that use JSX but don't bring
    /// those bindings into scope themselves (issue #291). In `--format bundle` the merged scope makes
    /// a single `lattish` import visible everywhere, but each ES module is its own scope, so a JSX
    /// module that didn't `import { h }` throws `ReferenceError` at load. We inject only the missing
    /// names (so an explicit `import { h }` isn't duplicated) from `jsx_import_source`, rewritten the
    /// same way as user imports (disk `.js` path vs. Vite-dev bare specifier). No-op outside ESM.
    fn emit_jsx_runtime_auto_import(&mut self, program: &Program) -> Result<(), CompileError> {
        if self.emit_mode != EmitMode::Esm {
            return Ok(());
        }
        let needed = tishlang_ui::jsx::jsx_runtime_imports_needed(program);
        if needed.is_empty() {
            return Ok(());
        }
        let spec = rewrite_import_to_js(
            &self.jsx_import_source,
            &self.module_path,
            &self.project_root,
            self.import_rewrite,
        )?;
        self.writeln(&format!(
            "import {{ {} }} from \"{}\";",
            needed.join(", "),
            spec
        ));
        Ok(())
    }

    fn emit_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match stmt {
            Statement::Block { statements, .. } => {
                self.writeln("{");
                self.indent += 1;
                for s in statements {
                    self.emit_statement(s)?;
                }
                self.indent -= 1;
                self.writeln("}");
            }
            // Comma-declarators: emit each declarator as its own JS statement in the
            // current scope (no wrapping braces).
            Statement::Multi { statements, .. } => {
                for s in statements {
                    self.emit_statement(s)?;
                }
            }
            Statement::VarDecl {
                name,
                mutable,
                type_ann: _,
                init,
                ..
            } => {
                let decl = if *mutable { "let" } else { "const" };
                let escaped = Self::escape_ident(name.as_ref());
                if let Some(expr) = init {
                    let e = self.emit_expr(expr)?;
                    self.writeln(&format!("{} {} = {};", decl, escaped, e));
                } else {
                    self.writeln(&format!("{} {};", decl, escaped));
                }
            }
            Statement::VarDeclDestructure {
                pattern,
                mutable,
                init,
                ..
            } => {
                let decl = if *mutable { "let" } else { "const" };
                let rhs = self.emit_expr(init)?;
                let pat = self.emit_destruct_pattern(pattern)?;
                self.writeln(&format!("{} {} = {};", decl, pat, rhs));
            }
            Statement::ExprStmt { expr, .. } => {
                let e = self.emit_expr(expr)?;
                self.writeln(&format!("{};", e));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_expr(cond)?;
                self.writeln(&format!("if ({})", c));
                self.emit_js_control_body(then_branch)?;
                if let Some(eb) = else_branch {
                    self.writeln("else");
                    self.emit_js_control_body(eb)?;
                }
            }
            Statement::While { cond, body, .. } => {
                let c = self.emit_expr(cond)?;
                self.writeln(&format!("while ({})", c));
                self.emit_js_control_body(body)?;
            }
            Statement::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                // Keep the whole `for (...)` on one line with normal statement indentation (do not
                // mix bare `write("for (")` with `writeln(")")`, which indents `)` on a new line).
                let mut header = self.indent_str();
                header.push_str("for (");
                if let Some(i) = init {
                    match i.as_ref() {
                        Statement::VarDecl {
                            name,
                            mutable,
                            init: opt_init,
                            ..
                        } => {
                            let decl = if *mutable { "let" } else { "const" };
                            let escaped = Self::escape_ident(name.as_ref());
                            if let Some(e) = opt_init {
                                let ex = self.emit_expr(e)?;
                                header.push_str(&format!("{} {} = {}", decl, escaped, ex));
                            } else {
                                header.push_str(&format!("{} {}", decl, escaped));
                            }
                        }
                        Statement::ExprStmt { expr, .. } => {
                            let ex = self.emit_expr(expr)?;
                            header.push_str(&ex);
                        }
                        // Comma-declarators (`for (let i = 0, n = len; …)`): emit JS-native
                        // `let i = 0, n = len` — one keyword, declarators joined by `, `.
                        Statement::Multi { statements, .. } => {
                            let mut decl_kw = "let";
                            let mut parts: Vec<String> = Vec::new();
                            for (idx, st) in statements.iter().enumerate() {
                                let Statement::VarDecl {
                                    name,
                                    mutable,
                                    init: opt_init,
                                    ..
                                } = st
                                else {
                                    return Err(CompileError::new("Unsupported for init"));
                                };
                                if idx == 0 {
                                    decl_kw = if *mutable { "let" } else { "const" };
                                }
                                let escaped = Self::escape_ident(name.as_ref());
                                if let Some(e) = opt_init {
                                    let ex = self.emit_expr(e)?;
                                    parts.push(format!("{} = {}", escaped, ex));
                                } else {
                                    parts.push(escaped);
                                }
                            }
                            header.push_str(&format!("{} {}", decl_kw, parts.join(", ")));
                        }
                        _ => return Err(CompileError::new("Unsupported for init")),
                    }
                }
                header.push_str("; ");
                if let Some(c) = cond {
                    let ce = self.emit_expr(c)?;
                    header.push_str(&ce);
                }
                header.push_str("; ");
                if let Some(u) = update {
                    let ue = self.emit_expr(u)?;
                    header.push_str(&ue);
                }
                header.push(')');
                header.push('\n');
                self.output.push_str(&header);
                self.emit_js_control_body(body)?;
            }
            Statement::ForOf {
                name,
                iterable,
                body,
                ..
            } => {
                let escaped = Self::escape_ident(name.as_ref());
                let it = self.emit_expr(iterable)?;
                self.writeln(&format!("for (const {} of {})", escaped, it));
                self.emit_js_control_body(body)?;
            }
            Statement::Return { value, .. } => {
                if let Some(v) = value {
                    let e = self.emit_expr(v)?;
                    self.writeln(&format!("return {};", e));
                } else {
                    self.writeln("return;");
                }
            }
            Statement::Break { .. } => self.writeln("break;"),
            Statement::Continue { .. } => self.writeln("continue;"),
            Statement::FunDecl {
                async_,
                name,
                params,
                rest_param,
                return_type: _,
                body,
                ..
            } => {
                let async_prefix = if *async_ { "async " } else { "" };
                let escaped = Self::escape_ident(name.as_ref());
                let params_str = self.emit_params(params, rest_param.as_ref())?;
                self.writeln(&format!(
                    "{}function {} ({}) {{",
                    async_prefix, escaped, params_str
                ));
                self.indent += 1;
                if *async_ {
                    self.in_async = true;
                }
                self.emit_statement(body)?;
                if *async_ {
                    self.in_async = false;
                }
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::Switch {
                expr,
                cases,
                default_body,
                ..
            } => {
                let e = self.emit_expr(expr)?;
                self.writeln(&format!("switch ({}) {{", e));
                self.indent += 1;
                for (case_expr, stmts) in cases {
                    if let Some(ce) = case_expr {
                        let c = self.emit_expr(ce)?;
                        self.writeln(&format!("case {}:", c));
                    }
                    for s in stmts {
                        self.emit_statement(s)?;
                    }
                    // Tish has no fall-through; add break unless case ends with break/return/throw
                    if !stmt_terminates_switch(stmts.last()) {
                        self.writeln("break;");
                    }
                }
                if let Some(stmts) = default_body {
                    self.writeln("default:");
                    for s in stmts {
                        self.emit_statement(s)?;
                    }
                    if !stmt_terminates_switch(stmts.last()) {
                        self.writeln("break;");
                    }
                }
                self.indent -= 1;
                self.writeln("}");
            }
            Statement::DoWhile { body, cond, .. } => {
                self.writeln("do {");
                self.indent += 1;
                self.emit_statement(body)?;
                self.indent -= 1;
                let c = self.emit_expr(cond)?;
                self.writeln(&format!("}} while ({});", c));
            }
            Statement::Throw { value, .. } => {
                let v = self.emit_expr(value)?;
                self.writeln(&format!("throw {};", v));
            }
            Statement::Try {
                body,
                catch_param,
                catch_body,
                finally_body,
                ..
            } => {
                self.writeln("try {");
                self.indent += 1;
                self.emit_statement(body)?;
                self.indent -= 1;
                if let (Some(param), Some(cb)) = (catch_param, catch_body) {
                    let p = Self::escape_ident(param.as_ref());
                    self.writeln(&format!("}} catch ({}) {{", p));
                    self.indent += 1;
                    self.emit_statement(cb)?;
                    self.indent -= 1;
                }
                if let Some(fb) = finally_body {
                    self.writeln("} finally {");
                    self.indent += 1;
                    self.emit_statement(fb)?;
                    self.indent -= 1;
                }
                self.writeln("}");
            }
            Statement::Import {
                specifiers, from, ..
            } => match self.emit_mode {
                // Bundle: resolved away by merge_modules (the dep bindings are already inlined).
                EmitMode::Bundle => {}
                EmitMode::Esm => self.emit_esm_import(specifiers, from.as_ref())?,
            },
            Statement::Export { declaration, .. } => match self.emit_mode {
                // Bundle: merge_modules unwrapped exports into plain top-level declarations.
                EmitMode::Bundle => {}
                EmitMode::Esm => self.emit_esm_export(declaration)?,
            },
            Statement::TypeAlias { .. }
            | Statement::DeclareVar { .. }
            | Statement::DeclareFun { .. } => {}
        }
        Ok(())
    }

    fn emit_params(
        &mut self,
        params: &[FunParam],
        rest_param: Option<&tishlang_ast::TypedParam>,
    ) -> Result<String, CompileError> {
        let mut parts: Vec<String> = Vec::new();
        for p in params {
            match p {
                FunParam::Simple(tp) => {
                    let n = Self::escape_ident(tp.name.as_ref());
                    let s = if let Some(ref d) = tp.default {
                        format!("{} = {}", n, self.emit_expr(d)?)
                    } else {
                        n
                    };
                    parts.push(s);
                }
                FunParam::Destructure {
                    pattern,
                    type_ann: _,
                    default,
                } => {
                    let mut s = self.emit_destruct_pattern(pattern)?;
                    if let Some(ref d) = default {
                        s = format!("{} = {}", s, self.emit_expr(d)?);
                    }
                    parts.push(s);
                }
            }
        }
        if let Some(rest) = rest_param {
            parts.push(format!("...{}", Self::escape_ident(rest.name.as_ref())));
        }
        Ok(parts.join(", "))
    }

    fn emit_destruct_pattern(&mut self, pattern: &DestructPattern) -> Result<String, CompileError> {
        match pattern {
            DestructPattern::Array(elements) => {
                let parts: Vec<String> = elements
                    .iter()
                    .map(|el| match el {
                        Some(DestructElement::Ident(n, _)) => Ok(Self::escape_ident(n.as_ref())),
                        Some(DestructElement::Pattern(p)) => self.emit_destruct_pattern(p),
                        Some(DestructElement::Rest(n, _)) => {
                            Ok(format!("...{}", Self::escape_ident(n.as_ref())))
                        }
                        None => Ok("".to_string()),
                    })
                    .collect::<Result<_, _>>()?;
                Ok(format!("[{}]", parts.join(", ")))
            }
            DestructPattern::Object(props) => {
                let parts: Vec<String> = props
                    .iter()
                    .map(|p| {
                        let k = p.key.as_ref();
                        match &p.value {
                            DestructElement::Ident(n, _) => {
                                if k == n.as_ref() {
                                    Ok(k.to_string())
                                } else {
                                    Ok(format!("{}: {}", k, Self::escape_ident(n.as_ref())))
                                }
                            }
                            DestructElement::Pattern(pat) => {
                                Ok(format!("{}: {}", k, self.emit_destruct_pattern(pat)?))
                            }
                            DestructElement::Rest(n, _) => {
                                Ok(format!("...{}", Self::escape_ident(n.as_ref())))
                            }
                        }
                    })
                    .collect::<Result<_, _>>()?;
                Ok(format!("{{ {} }}", parts.join(", ")))
            }
        }
    }

    /// Is `expr` the `null` literal? Used to lower `x === null` / `x !== null` to JS `== null` /
    /// `!= null` so the check catches `undefined` too (tish treats a missing/undefined value as
    /// null on every other backend) — see the StrictEq/StrictNe arms below.
    fn is_null_literal(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Literal {
                value: Literal::Null,
                ..
            }
        )
    }

    fn emit_expr(&mut self, expr: &Expr) -> Result<String, CompileError> {
        Ok(match expr {
            Expr::Literal { value, .. } => match value {
                // Rust's `{}` prints non-finite f64 as `inf` / `-inf` / `NaN`; only `NaN` is valid JS.
                // Emit the JS spellings so a folded `1/0` / `-1/0` doesn't become an undefined `inf`
                // identifier in the output.
                Literal::Number(n) if n.is_nan() => "NaN".to_string(),
                Literal::Number(n) if n.is_infinite() => {
                    if *n < 0.0 { "-Infinity".to_string() } else { "Infinity".to_string() }
                }
                Literal::Number(n) => format!("{}", n),
                Literal::String(s) => format!("{:?}", s.as_ref()),
                Literal::Bool(b) => format!("{}", b),
                Literal::Null => "null".to_string(),
            },
            Expr::Ident { name, .. } => Self::escape_ident(name.as_ref()),
            Expr::Binary {
                left, op, right, ..
            } => {
                let l = self.emit_expr(left)?;
                let r = self.emit_expr(right)?;
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Pow => "**",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    // tish has no `undefined`: a missing/absent value reads back as `null`, so
                    // `x === null` means "is nullish" — exactly how interp/vm/native behave (a
                    // missing property is null there). In the JS runtime a missing property is
                    // `undefined`, so lower `=== null` / `!== null` to loose `== null` / `!= null`,
                    // which match BOTH null and undefined — keeping `=== null` mean the same thing on
                    // every target. Strict equality between non-null operands is unaffected.
                    BinOp::StrictEq => {
                        if Self::is_null_literal(left) || Self::is_null_literal(right) {
                            "=="
                        } else {
                            "==="
                        }
                    }
                    BinOp::StrictNe => {
                        if Self::is_null_literal(left) || Self::is_null_literal(right) {
                            "!="
                        } else {
                            "!=="
                        }
                    }
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
                    BinOp::UShr => ">>>",
                    BinOp::In => {
                        // key in object (property/index existence check)
                        return Ok(format!("({} in {})", l, r));
                    }
                };
                format!("({} {} {})", l, op_str, r)
            }
            Expr::Unary { op, operand, .. } => {
                let o = self.emit_expr(operand)?;
                match op {
                    UnaryOp::Not => format!("!{}", o),
                    UnaryOp::Neg => format!("(-{})", o),
                    UnaryOp::Pos => format!("(+{})", o),
                    UnaryOp::BitNot => format!("(~{})", o),
                    UnaryOp::Void => format!("((void {}), null)", o), // Tish void returns null, not undefined
                }
            }
            Expr::Call { callee, args, .. } => {
                let c = self.emit_expr(callee)?;
                let arg_strs: Result<Vec<_>, _> =
                    args.iter().map(|a| self.emit_call_arg(a)).collect();
                let arg_strs = arg_strs?.join(", ");
                // Tish uses null for undefined (e.g. empty array pop/shift)
                format!("({}({}) ?? null)", c, arg_strs)
            }
            Expr::New { callee, args, .. } => {
                let c = self.emit_expr(callee)?;
                let arg_strs: Result<Vec<_>, _> =
                    args.iter().map(|a| self.emit_call_arg(a)).collect();
                let arg_strs = arg_strs?.join(", ");
                format!("(new {}({}) ?? null)", c, arg_strs)
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                // `255.toString()` is a JS syntax error — the lexer reads `255.` as a float and
                // then chokes on the method name. Parenthesize a numeric-literal object so member
                // access / method calls stay valid: `(255).toString()`. (Folded constants reach
                // codegen as number literals too, so this covers e.g. `(100 * 2).toString()`.)
                let obj = if matches!(&**object, Expr::Literal { value: Literal::Number(_), .. }) {
                    format!("({})", obj)
                } else {
                    obj
                };
                let expr = match prop {
                    MemberProp::Name { name, .. } => {
                        if name.parse::<u32>().is_ok()
                            || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        {
                            format!("{}[{:?}]", obj, name.as_ref())
                        } else {
                            let sep = if *optional { "?." } else { "." };
                            format!("{}{}{}", obj, sep, name.as_ref())
                        }
                    }
                    MemberProp::Expr(e) => {
                        let idx = self.emit_expr(e)?;
                        format!("{}[{}]", obj, idx)
                    }
                };
                // Tish uses null where JS uses undefined for optional chaining short-circuit only
                if *optional {
                    format!("({} ?? null)", expr)
                } else {
                    expr
                }
            }
            Expr::Index {
                object,
                index,
                optional,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let sep = if *optional { "?." } else { "" };
                let expr = format!("{}{}[{}]", obj, sep, idx);
                // Tish uses null for array holes / missing indices (JS returns undefined)
                format!("({} ?? null)", expr)
            }
            Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                let c = self.emit_expr(cond)?;
                let t = self.emit_expr(then_branch)?;
                let e = self.emit_expr(else_branch)?;
                format!("({} ? {} : {})", c, t, e)
            }
            Expr::NullishCoalesce { left, right, .. } => {
                let l = self.emit_expr(left)?;
                let r = self.emit_expr(right)?;
                format!("({} ?? {})", l, r)
            }
            Expr::Array { elements, .. } => {
                let parts: Result<Vec<_>, _> = elements
                    .iter()
                    .map(|el| match el {
                        ArrayElement::Expr(e) => self.emit_expr(e),
                        ArrayElement::Spread(e) => Ok(format!("...{}", self.emit_expr(e)?)),
                    })
                    .collect();
                format!("[{}]", parts?.join(", "))
            }
            Expr::Object { props, .. } => {
                let parts: Result<Vec<_>, _> = props
                    .iter()
                    .map(|p| match p {
                        ObjectProp::KeyValue(k, v, _) => {
                            let key = k.as_ref();
                            let val = self.emit_expr(v)?;
                            Ok(if key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                format!("{}: {}", key, val)
                            } else {
                                format!("{:?}: {}", key, val)
                            })
                        }
                        ObjectProp::Spread(e) => Ok(format!("...{}", self.emit_expr(e)?)),
                    })
                    .collect();
                format!("{{ {} }}", parts?.join(", "))
            }
            Expr::Assign { name, value, .. } => {
                let n = Self::escape_ident(name.as_ref());
                let v = self.emit_expr(value)?;
                format!("({} = {})", n, v)
            }
            Expr::TypeOf { operand, .. } => {
                let o = self.emit_expr(operand)?;
                // tish `typeof null` is "null" (interp/vm/native all agree — null is a first-class
                // type, not JS's `typeof null === "object"` wart). tish has no `undefined`, so any
                // nullish operand (incl. a JS-runtime `undefined`) maps to "null". Evaluate the
                // operand once via the arrow arg so side effects don't run twice.
                format!("((__v) => __v == null ? \"null\" : typeof __v)({})", o)
            }
            Expr::Delete { target, .. } => {
                // Emit the raw property *reference*, not a value: `emit_expr` wraps Index /
                // optional reads in `(… ?? null)`, and `delete (x ?? null)` is a no-op. So
                // reconstruct `obj.name` / `obj[key]` directly here.
                match target.as_ref() {
                    Expr::Member { object, prop: MemberProp::Name { name, .. }, .. } => {
                        let obj = self.emit_expr(object)?;
                        if name.parse::<u32>().is_ok()
                            || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        {
                            format!("(delete {}[{:?}])", obj, name.as_ref())
                        } else {
                            format!("(delete {}.{})", obj, name.as_ref())
                        }
                    }
                    Expr::Member { object, prop: MemberProp::Expr(key), .. } => {
                        let obj = self.emit_expr(object)?;
                        let k = self.emit_expr(key)?;
                        format!("(delete {}[{}])", obj, k)
                    }
                    Expr::Index { object, index, .. } => {
                        let obj = self.emit_expr(object)?;
                        let idx = self.emit_expr(index)?;
                        format!("(delete {}[{}])", obj, idx)
                    }
                    _ => {
                        let t = self.emit_expr(target)?;
                        format!("(delete {})", t)
                    }
                }
            }
            Expr::PostfixInc { name, .. } => {
                format!("{}++", Self::escape_ident(name.as_ref()))
            }
            Expr::PostfixDec { name, .. } => {
                format!("{}--", Self::escape_ident(name.as_ref()))
            }
            Expr::PrefixInc { name, .. } => {
                format!("++{}", Self::escape_ident(name.as_ref()))
            }
            Expr::PrefixDec { name, .. } => {
                format!("--{}", Self::escape_ident(name.as_ref()))
            }
            Expr::CompoundAssign {
                name, op, value, ..
            } => {
                let n = Self::escape_ident(name.as_ref());
                let v = self.emit_expr(value)?;
                let op_str = match op {
                    CompoundOp::Add => "+=",
                    CompoundOp::Sub => "-=",
                    CompoundOp::Mul => "*=",
                    CompoundOp::Div => "/=",
                    CompoundOp::Mod => "%=",
                };
                format!("({} {} {})", n, op_str, v)
            }
            Expr::LogicalAssign {
                name, op, value, ..
            } => {
                let n = Self::escape_ident(name.as_ref());
                let v = self.emit_expr(value)?;
                let op_str = match op {
                    LogicalAssignOp::AndAnd => "&&=",
                    LogicalAssignOp::OrOr => "||=",
                    LogicalAssignOp::Nullish => "??=",
                };
                format!("({} {} {})", n, op_str, v)
            }
            Expr::MemberAssign {
                object,
                prop,
                value,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                let val = self.emit_expr(value)?;
                format!("({}.{} = {})", obj, prop.as_ref(), val)
            }
            Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                let obj = self.emit_expr(object)?;
                let idx = self.emit_expr(index)?;
                let val = self.emit_expr(value)?;
                format!("({}[{}] = {})", obj, idx, val)
            }
            Expr::ArrowFunction { params, body, .. } => {
                let ps = self.emit_params(params, None)?;
                let body_str = match body {
                    ArrowBody::Expr(e) => self.emit_expr(e)?,
                    ArrowBody::Block(s) => {
                        let saved = std::mem::take(&mut self.output);
                        self.writeln("{");
                        self.indent += 1;
                        self.emit_statement(s)?;
                        self.indent -= 1;
                        self.writeln("}");
                        let block = self.output.trim().to_string();
                        self.output = saved;
                        block
                    }
                };
                if matches!(body, ArrowBody::Expr(_)) {
                    format!("({}) => ({})", ps, body_str)
                } else {
                    format!("({}) => {}", ps, body_str)
                }
            }
            Expr::TemplateLiteral { quasis, exprs, .. } => {
                let mut s = String::from('`');
                for (i, q) in quasis.iter().enumerate() {
                    let escaped = q
                        .replace('\\', "\\\\")
                        .replace('`', "\\`")
                        .replace('$', "\\$");
                    s.push_str(&escaped);
                    if i < exprs.len() {
                        s.push_str("${");
                        s.push_str(&self.emit_expr(&exprs[i])?);
                        s.push('}');
                    }
                }
                s.push('`');
                s
            }
            Expr::Await { operand, .. } => {
                let o = self.emit_expr(operand)?;
                format!("(await {})", o)
            }
            Expr::JsxElement { .. } | Expr::JsxFragment { .. } => {
                tishlang_ui::jsx::emit_jsx_js(expr, &mut |e| {
                    self.emit_expr(e).map_err(|ce| ce.message)
                })
                .map_err(|m| CompileError { message: m })?
            }
            Expr::NativeModuleLoad { spec, .. } => {
                return Err(CompileError {
                    message: format!(
                        "Native module imports ({}) are only supported when compiling to Rust. Omit --target js.",
                        spec.as_ref()
                    ),
                });
            }
        })
    }

    fn emit_call_arg(&mut self, arg: &CallArg) -> Result<String, CompileError> {
        match arg {
            CallArg::Expr(e) => self.emit_expr(e),
            CallArg::Spread(e) => Ok(format!("...{}", self.emit_expr(e)?)),
        }
    }

    /// Emit a real ES `import` statement (ESM mode), rewriting the `.tish` specifier to its sibling
    /// `.js` output path. Named and namespace specifiers can't share one statement, so a module that
    /// imports both is split across two lines.
    fn emit_esm_import(
        &mut self,
        specifiers: &[ImportSpecifier],
        from: &str,
    ) -> Result<(), CompileError> {
        let spec = rewrite_import_to_js(
            from,
            &self.module_path,
            &self.project_root,
            self.import_rewrite,
        )?;
        let mut named: Vec<String> = Vec::new();
        let mut default_local: Option<String> = None;
        let mut namespace_local: Option<String> = None;
        for s in specifiers {
            match s {
                ImportSpecifier::Named { name, alias, .. } => match alias {
                    Some(a) => named.push(format!(
                        "{} as {}",
                        Self::escape_ident(name.as_ref()),
                        Self::escape_ident(a.as_ref())
                    )),
                    None => named.push(Self::escape_ident(name.as_ref())),
                },
                ImportSpecifier::Default { name, .. } => {
                    default_local = Some(Self::escape_ident(name.as_ref()))
                }
                ImportSpecifier::Namespace { name, .. } => {
                    namespace_local = Some(Self::escape_ident(name.as_ref()))
                }
            }
        }
        if let Some(ns) = namespace_local {
            match &default_local {
                Some(def) => self.writeln(&format!("import {}, * as {} from \"{}\";", def, ns, spec)),
                None => self.writeln(&format!("import * as {} from \"{}\";", ns, spec)),
            }
            if !named.is_empty() {
                self.writeln(&format!("import {{ {} }} from \"{}\";", named.join(", "), spec));
            }
        } else if !named.is_empty() {
            match &default_local {
                Some(def) => self.writeln(&format!(
                    "import {}, {{ {} }} from \"{}\";",
                    def,
                    named.join(", "),
                    spec
                )),
                None => self.writeln(&format!("import {{ {} }} from \"{}\";", named.join(", "), spec)),
            }
        } else if let Some(def) = &default_local {
            self.writeln(&format!("import {} from \"{}\";", def, spec));
        } else {
            self.writeln(&format!("import \"{}\";", spec));
        }
        Ok(())
    }

    /// Emit a real ES `export` (ESM mode). Named exports prefix the inner declaration with the
    /// `export` keyword; default exports become `export default <expr>;`.
    fn emit_esm_export(&mut self, declaration: &ExportDeclaration) -> Result<(), CompileError> {
        match declaration {
            ExportDeclaration::Named(inner) => {
                // Emit the inner declaration, then splice the `export ` keyword in front of it. The
                // declaration's first line is `<indent><keyword> …`, so the keyword goes right after
                // the leading indent.
                let start = self.output.len();
                self.emit_statement(inner)?;
                let insert_at = start + self.indent_str().len();
                if insert_at <= self.output.len() {
                    self.output.insert_str(insert_at, "export ");
                }
            }
            ExportDeclaration::Default(e) => {
                let v = self.emit_expr(e)?;
                self.writeln(&format!("export default {};", v));
            }
            // #305: re-export in ESM mode — emit the native `export { … } from` / `export * from`,
            // rewriting the `.tish` specifier to the emitted `.js` module path.
            ExportDeclaration::ReExport {
                specifiers,
                all,
                from,
                ..
            } => {
                let spec = rewrite_import_to_js(
                    from.as_ref(),
                    &self.module_path,
                    &self.project_root,
                    self.import_rewrite,
                )?;
                if *all {
                    self.writeln(&format!("export * from {:?};", spec));
                } else {
                    let mut parts: Vec<String> = Vec::new();
                    for s in specifiers {
                        if let ImportSpecifier::Named { name, alias, .. } = s {
                            match alias {
                                Some(a) => parts.push(format!(
                                    "{} as {}",
                                    Self::escape_ident(name.as_ref()),
                                    Self::escape_ident(a.as_ref())
                                )),
                                None => parts.push(Self::escape_ident(name.as_ref()).to_string()),
                            }
                        }
                    }
                    self.writeln(&format!("export {{ {} }} from {:?};", parts.join(", "), spec));
                }
            }
        }
        Ok(())
    }
}

/// Rewrite a Tish import specifier to the ESM `.js` specifier that the emitted module tree uses.
/// Relative specifiers keep their shape (the output tree mirrors the source tree) with `.tish`
/// swapped to `.js`. Bare specifiers are resolved to a `.tish` file and re-expressed as a relative
/// path from the importer's output location. Native imports (`tish:*`, `cargo:*`, …) are rejected.
fn rewrite_import_to_js(
    spec: &str,
    importer_abs: &Path,
    project_root: &Path,
    rewrite: ImportRewrite,
) -> Result<String, CompileError> {
    if tishlang_compile::is_native_import(spec) {
        return Err(CompileError {
            message: format!(
                "Native module import '{}' is not supported with --target js --format esm (native modules require --target native).",
                spec
            ),
        });
    }
    // Vite dev keeps specifiers verbatim: relative `.tish` paths and bare packages are left for the
    // plugin's `resolveId`/`load` (relative) or Node/Vite resolution (bare) to handle per-module.
    if rewrite == ImportRewrite::ViteDev {
        return Ok(spec.to_string());
    }
    if spec.starts_with("./") || spec.starts_with("../") {
        return Ok(spec_ext_to_js(spec));
    }
    // Bare specifier (e.g. a package): resolve to its `.tish` file and express it relative to the
    // importer's output `.js`, so the emitted module tree stays self-contained.
    let from_dir = importer_abs.parent().unwrap_or_else(|| Path::new("."));
    let dep = tishlang_compile::resolve_bare_spec(spec, from_dir, project_root).ok_or_else(|| {
        CompileError {
            message: format!(
                "Cannot resolve package import '{}' for --format esm. Only relative imports and packages resolvable to a .tish entry are supported; use --format bundle otherwise.",
                spec
            ),
        }
    })?;
    // The emitted module tree mirrors the real source tree under a common base (see
    // `compile_project_esm`), so the relative path between two *absolute* canonical paths is the
    // same as the relative path between their emitted `.js` files. This holds whether the dependency
    // lives under the entry's project root, in a sibling package, or in `node_modules` (#282), so no
    // "outside the project root" rejection is needed.
    let dep_canon = dep.canonicalize().unwrap_or(dep);
    let importer_canon = importer_abs
        .canonicalize()
        .unwrap_or_else(|_| importer_abs.to_path_buf());
    let importer_dir = importer_canon.parent().unwrap_or_else(|| Path::new("/"));
    Ok(spec_ext_to_js(&relative_specifier(importer_dir, &dep_canon)))
}

/// Swap a relative import specifier's `.tish` extension for `.js` (or append `.js` when extensionless).
/// Already-`.js`/`.mjs` specifiers pass through unchanged.
fn spec_ext_to_js(spec: &str) -> String {
    if let Some(base) = spec.strip_suffix(".tish") {
        format!("{}.js", base)
    } else if spec.ends_with(".js") || spec.ends_with(".mjs") {
        spec.to_string()
    } else {
        format!("{}.js", spec)
    }
}

/// Build a `./`-prefixed relative module specifier from `from_dir` to `to_file`, compared
/// component-wise, using `/` separators as ESM requires. Both paths must be expressed the same way
/// (both absolute canonical, or both relative to the same base) so the shared prefix lines up.
fn relative_specifier(from_dir: &Path, to_file: &Path) -> String {
    let from: Vec<String> = from_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let to: Vec<String> = to_file
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in i..from.len() {
        parts.push("..".to_string());
    }
    for c in &to[i..] {
        parts.push(c.clone());
    }
    let joined = parts.join("/");
    if joined.starts_with("..") {
        joined
    } else {
        format!("./{}", joined)
    }
}

/// Compile a single program (no imports) to JavaScript. JSX lowers to `h` / `Fragment` (Lattish).
pub fn compile_with_jsx(program: &Program, optimize: bool) -> Result<String, CompileError> {
    let program = if optimize {
        tishlang_opt::optimize(program)
    } else {
        program.clone()
    };
    let mut g = Codegen::new();
    g.emit_program(&program, None, None)?;
    Ok(g.output)
}

/// JavaScript plus optional v3 source map JSON (for publishing Tish libraries consumed from JS/TS).
#[derive(Debug, Clone)]
pub struct JsBundle {
    pub js: String,
    pub source_map_json: Option<String>,
}

/// Same as [`compile_project_with_jsx`] plus a v3 source map pointing at merged statements’ original `.tish` files.
/// **Does not run AST optimization** (required so statement ↔ file alignment stays valid).
pub fn compile_project_with_jsx_and_source_map(
    entry_path: &Path,
    project_root: Option<&Path>,
    output_js_file_name: &str,
) -> Result<JsBundle, CompileError> {
    compile_project_js_inner(entry_path, project_root, false, true, output_js_file_name)
}

fn compile_project_js_inner(
    entry_path: &Path,
    project_root: Option<&Path>,
    optimize: bool,
    emit_source_map: bool,
    output_js_file_name: &str,
) -> Result<JsBundle, CompileError> {
    use tishlang_ast::Statement;
    let modules = tishlang_compile::resolve_project(entry_path, project_root)
        .map_err(|e| CompileError { message: e })?;
    tishlang_compile::detect_cycles(&modules).map_err(|e| CompileError { message: e })?;
    let merged =
        tishlang_compile::merge_modules(modules).map_err(|e| CompileError { message: e })?;
    let program = if optimize {
        tishlang_opt::optimize(&merged.program)
    } else {
        merged.program.clone()
    };
    let stmt_sources = merged.statement_sources;
    let default_export = program.statements.iter().find_map(|s| {
        if let Statement::VarDecl { name, .. } = s {
            let n = name.as_ref();
            if n.starts_with("__default_") {
                Some(n.to_string())
            } else {
                None
            }
        } else {
            None
        }
    });
    if emit_source_map && optimize {
        return Err(CompileError {
            message: "internal: source map requested with optimize".into(),
        });
    }
    let root = project_root
        .map(Path::to_path_buf)
        .or_else(|| entry_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut gen = Codegen::new();
    let mut map_builder = if emit_source_map {
        let mut b = SourceMapBuilder::new(Some(output_js_file_name));
        b.set_source_root(Some(""));
        Some(b)
    } else {
        None
    };
    if let Some(ref mut b) = map_builder {
        gen.emit_program(
            &program,
            Some((stmt_sources.as_slice(), root.as_path())),
            Some(b),
        )?;
    } else {
        gen.emit_program(&program, None, None)?;
    }
    let mut js = gen.output;
    if let Some(name) = default_export {
        js.push_str(&format!("\nexport default {};\n", name));
    }
    let map_json = if let Some(b) = map_builder {
        let sm = b.into_sourcemap();
        let mut v = Vec::new();
        sm.to_writer(&mut v).map_err(|e| CompileError {
            message: e.to_string(),
        })?;
        Some(String::from_utf8(v).map_err(|e| CompileError {
            message: e.to_string(),
        })?)
    } else {
        None
    };
    Ok(JsBundle {
        js,
        source_map_json: map_json,
    })
}

/// Compile a project from entry path, resolving and merging modules.
/// Uses shared resolve from tishlang_compile (same pipeline as native/WASM).
pub fn compile_project_with_jsx(
    entry_path: &std::path::Path,
    project_root: Option<&std::path::Path>,
    optimize: bool,
) -> Result<String, CompileError> {
    let stem = entry_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("out.js");
    let out_name = if stem.ends_with(".tish") {
        format!("{}.js", stem.trim_end_matches(".tish"))
    } else {
        format!("{stem}.js")
    };
    Ok(compile_project_js_inner(entry_path, project_root, optimize, false, &out_name)?.js)
}

/// One emitted ES module: where it goes (relative to the output directory, mirroring the source
/// tree) and its JavaScript. See [`compile_project_esm`].
#[derive(Debug, Clone)]
pub struct EmittedJsModule {
    pub relative_path: PathBuf,
    pub js: String,
}

/// Compile a project to **ES modules** — one `.js` file per `.tish` module, with real `import` /
/// `export` statements (issue #282). Unlike [`compile_project_with_jsx`], modules are NOT merged, so
/// each keeps its own scope (no exported-name collisions) and a bundler can tree-shake the graph.
/// Output paths mirror the source tree relative to the deepest directory common to every module in
/// the graph (so sibling-package / `node_modules` deps are included), with `.tish` swapped to `.js`.
pub fn compile_project_esm(
    entry_path: &Path,
    project_root: Option<&Path>,
    optimize: bool,
    jsx_import_source: &str,
) -> Result<Vec<EmittedJsModule>, CompileError> {
    let modules = tishlang_compile::resolve_project(entry_path, project_root)
        .map_err(|e| CompileError { message: e })?;
    tishlang_compile::detect_cycles(&modules).map_err(|e| CompileError { message: e })?;
    // Resolution root: where bare-specifier / `node_modules` lookups are anchored. Unchanged
    // semantics — only used for `resolve_bare_spec`, which itself walks upward from each importer.
    let res_root = project_root
        .map(Path::to_path_buf)
        .or_else(|| entry_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let res_root_canon = res_root.canonicalize().unwrap_or(res_root);

    // Output layout base: the deepest directory that is an ancestor of *every* module in the graph.
    // The output tree mirrors the real filesystem beneath this base, so a dependency in a sibling
    // package or `node_modules` (#282 follow-up) gets a stable home in the output instead of being
    // rejected for living "outside the project root". For a self-contained project this is just the
    // project root, so existing layouts are unchanged.
    let mod_canons: Vec<PathBuf> = modules
        .iter()
        .map(|m| m.path.canonicalize().unwrap_or_else(|_| m.path.clone()))
        .collect();
    let layout_base = common_ancestor_dir(&mod_canons);

    let mut out = Vec::with_capacity(modules.len());
    for (module, mod_canon) in modules.iter().zip(mod_canons.iter()) {
        let rel = mod_canon
            .strip_prefix(&layout_base)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| {
                // `layout_base` is a common ancestor of every module, so this is unreachable; fall
                // back to the bare file name rather than panicking if a path is unexpectedly absolute.
                PathBuf::from(mod_canon.file_name().unwrap_or(mod_canon.as_os_str()))
            });
        let rel_js = rel.with_extension("js");
        let program = if optimize {
            tishlang_opt::optimize(&module.program)
        } else {
            module.program.clone()
        };
        let mut gen = Codegen::new_esm(
            mod_canon.clone(),
            res_root_canon.clone(),
            ImportRewrite::Disk,
            jsx_import_source.to_string(),
        );
        gen.emit_program(&program, None, None)?;
        out.push(EmittedJsModule {
            relative_path: rel_js,
            js: gen.output,
        });
    }
    Ok(out)
}

/// Compile a **single** `.tish` module to one ES module (issue #284, Vite dev / HMR). Unlike
/// [`compile_project_esm`], the dependency graph is **not** resolved — only `module_path` is read
/// and parsed — so a Vite plugin can compile one file per `load()` and let Vite own the module
/// graph. With `ImportRewrite::ViteDev` relative `.tish` specifiers and bare packages are preserved
/// so Vite re-enters the plugin per dependency. When `source_map` is set, a v3 map back to the
/// `.tish` source is returned (requires `optimize == false`, matching the bundle source-map rule).
pub fn compile_module_esm(
    module_path: &Path,
    project_root: Option<&Path>,
    optimize: bool,
    import_rewrite: ImportRewrite,
    source_map: bool,
    jsx_import_source: &str,
) -> Result<JsBundle, CompileError> {
    if source_map && optimize {
        return Err(CompileError {
            message: "source map requires no optimization (mappings follow unmerged statement order)."
                .into(),
        });
    }
    let source = std::fs::read_to_string(module_path).map_err(|e| CompileError {
        message: format!("Cannot read {}: {}", module_path.display(), e),
    })?;
    let parsed = tishlang_parser::parse(&source).map_err(|e| CompileError {
        message: format!("Parse error in {}: {}", module_path.display(), e),
    })?;
    let program = if optimize {
        tishlang_opt::optimize(&parsed)
    } else {
        parsed
    };
    let module_canon = module_path
        .canonicalize()
        .unwrap_or_else(|_| module_path.to_path_buf());
    let root = project_root
        .map(Path::to_path_buf)
        .or_else(|| module_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let root_canon = root.canonicalize().unwrap_or(root);

    let mut gen = Codegen::new_esm(
        module_canon.clone(),
        root_canon.clone(),
        import_rewrite,
        jsx_import_source.to_string(),
    );
    if source_map {
        let stmt_sources = vec![module_canon.clone(); program.statements.len()];
        let mut builder = SourceMapBuilder::new(Some("module.js"));
        builder.set_source_root(Some(""));
        gen.emit_program(
            &program,
            Some((stmt_sources.as_slice(), root_canon.as_path())),
            Some(&mut builder),
        )?;
        let mut sm = builder.into_sourcemap();
        // Embed the original `.tish` so consumers (Vite, browser devtools) never resolve
        // `sources` from disk. The map's `sources` are project-root-relative, but Vite resolves
        // them against the module's own directory; without inline content it logs
        // "Sourcemap ... points to missing source files" and devtools can't show the original.
        let source_count = sm.get_source_count();
        for id in 0..source_count {
            sm.set_source_contents(id, Some(&source));
        }
        let mut v = Vec::new();
        sm.to_writer(&mut v).map_err(|e| CompileError {
            message: e.to_string(),
        })?;
        let map_json = String::from_utf8(v).map_err(|e| CompileError {
            message: e.to_string(),
        })?;
        Ok(JsBundle {
            js: gen.output,
            source_map_json: Some(map_json),
        })
    } else {
        gen.emit_program(&program, None, None)?;
        Ok(JsBundle {
            js: gen.output,
            source_map_json: None,
        })
    }
}

/// Deepest directory that is an ancestor of every given file path, compared component-wise. Used as
/// the base of the `--format esm` output tree so modules outside the entry's project root (sibling
/// packages, `node_modules`) still map to a stable, collision-free location. For a single file this
/// is its parent directory; for files that share only the filesystem root it is `/`.
fn common_ancestor_dir(files: &[PathBuf]) -> PathBuf {
    let parent_components = |p: &Path| -> Vec<std::ffi::OsString> {
        p.parent()
            .unwrap_or_else(|| Path::new("/"))
            .components()
            .map(|c| c.as_os_str().to_os_string())
            .collect()
    };
    let mut common: Vec<std::ffi::OsString> = match files.first() {
        Some(f) => parent_components(f),
        None => return PathBuf::from("/"),
    };
    for f in &files[1..] {
        let comps = parent_components(f);
        let mut i = 0;
        while i < common.len() && i < comps.len() && common[i] == comps[i] {
            i += 1;
        }
        common.truncate(i);
    }
    if common.is_empty() {
        return PathBuf::from("/");
    }
    let mut base = PathBuf::new();
    for c in &common {
        base.push(c);
    }
    base
}
