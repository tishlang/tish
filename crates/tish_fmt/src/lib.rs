//! Pretty-print Tish AST to source. Style: 2-space indent, braces for blocks, trailing newline.

use std::collections::HashMap;

use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CallArg, CompoundOp, DestructElement, DestructPattern,
    ExportDeclaration, Expr, FunParam, ImportSpecifier, JsxAttrValue, JsxChild, JsxProp, Literal,
    LogicalAssignOp, MemberProp, ObjectProp, Program, Span, Statement, TypeAnnotation, TypedParam,
    UnaryOp,
};

/// A comment recovered from source by [`scan_comments`]. The lexer discards
/// comments, so the parsed AST has none; the formatter re-inserts these by source
/// position so they survive a format pass.
#[derive(Clone)]
struct CommentTok {
    /// 1-based (line, col) of the opening `/`, matching `ast::Span`.
    start: (usize, usize),
    /// Verbatim comment text, including the `//` or `/* */` delimiters.
    text: String,
    /// True when only whitespace precedes the comment on its line (a leading,
    /// own-line comment); false for a trailing `code // note` comment.
    own_line: bool,
}

/// Maps a `{`'s 1-based (line, col) to its matching `}`'s (line, col). Used to
/// bound a block's dangling-comment flush to the real closing brace, since the
/// parser's `Block` `span.end` overshoots to the next token.
type BraceMap = HashMap<(usize, usize), (usize, usize)>;

/// Format Tish source. On parse error, returns the parser message.
pub fn format_source(source: &str) -> Result<String, String> {
    let program = tishlang_parser::parse(source)?;
    let (comments, braces, bracket_spans) = scan_comments(source);
    Ok(format_with_comments(
        &program,
        comments,
        blank_line_map(source),
        braces,
        bracket_spans,
        source,
    ))
}

/// Format an already-parsed program. Comments and blank lines are unavailable
/// here (the AST carries neither), so the output is comment-free and dense and
/// `// tish-fmt-ignore` has no effect; use [`format_source`] when the original
/// text is available.
pub fn format_program(program: &Program) -> String {
    format_with_comments(
        program,
        Vec::new(),
        Vec::new(),
        BraceMap::new(),
        Vec::new(),
        "",
    )
}

fn format_with_comments(
    program: &Program,
    comments: Vec<CommentTok>,
    blank_lines: Vec<bool>,
    braces: BraceMap,
    bracket_spans: Vec<(usize, usize)>,
    source: &str,
) -> String {
    let mut p = Printer::new(comments, blank_lines, braces, bracket_spans, source);
    p.print_seq(&program.statements, 0);
    // Comments after the last statement (trailing file comments).
    p.emit_leading_comments((usize::MAX, usize::MAX), 0);
    // Exactly one trailing newline.
    while p.buf.ends_with('\n') {
        p.buf.pop();
    }
    p.buf.push('\n');
    p.buf
}

/// `out[line]` is true when 1-based source `line` is blank (empty or whitespace
/// only). Index 0 is unused so callers can index by 1-based line number.
fn blank_line_map(source: &str) -> Vec<bool> {
    let mut v = vec![false];
    v.extend(source.lines().map(|l| l.trim().is_empty()));
    v
}

struct Printer {
    buf: String,
    /// Comments in source order; `ci` is the next one not yet emitted.
    comments: Vec<CommentTok>,
    ci: usize,
    /// 1-indexed: `blank_lines[n]` is true when source line `n` is blank. Empty
    /// when no source is available (`format_program`).
    blank_lines: Vec<bool>,
    /// `{`→`}` position map for bounding block dangling-comment flushes.
    braces: BraceMap,
    /// Nonzero once anything has been emitted in the current sequence; used only
    /// to suppress a leading blank line at the very start of a file or block.
    emitted: usize,
    /// Current structural indentation level for expression layout — set to the
    /// enclosing statement's level and bumped as broken containers nest. Continuation
    /// lines of a broken object/array/argument-list indent to `depth + 1`.
    depth: usize,
    /// When set, containers render on one line regardless of width. Used to measure
    /// a container's flat width before deciding whether to break it.
    force_flat: bool,
    /// Source as chars, plus 1-based line → starting char-index, for slicing the
    /// original text of a `// tish-fmt-ignore`-d statement verbatim. Empty when no
    /// source is available (`format_program`).
    src: Vec<char>,
    line_start: Vec<usize>,
    /// Set by [`Printer::emit_leading_comments`] when the comment directly above the
    /// next statement is `// tish-fmt-ignore`; consumed by [`Printer::print_seq`].
    ignore_next: bool,
    /// `(open_line, close_line)` for every bracket pair, used to bound an ignored
    /// statement's verbatim slice to its own full extent.
    bracket_spans: Vec<(usize, usize)>,
}

/// Target line width: objects/arrays/argument lists that fit within this stay on
/// one line; longer ones break one item per line.
const WIDTH: usize = 100;

/// The escape-hatch marker (à la Prettier's `// prettier-ignore`): a comment with
/// exactly this content leaves the next statement's original source untouched.
const IGNORE_MARKER: &str = "tish-fmt-ignore";

impl Printer {
    fn new(
        comments: Vec<CommentTok>,
        blank_lines: Vec<bool>,
        braces: BraceMap,
        bracket_spans: Vec<(usize, usize)>,
        source: &str,
    ) -> Self {
        let src: Vec<char> = source.chars().collect();
        // line_start[L] = char index where 1-based line L begins (index 0 unused).
        let mut line_start = vec![0usize, 0usize];
        for (i, &c) in src.iter().enumerate() {
            if c == '\n' {
                line_start.push(i + 1);
            }
        }
        Self {
            buf: String::with_capacity(4096),
            comments,
            ci: 0,
            blank_lines,
            braces,
            emitted: 0,
            depth: 0,
            force_flat: false,
            src,
            line_start,
            ignore_next: false,
            bracket_spans,
        }
    }

    /// Char index of a 1-based (line, col) position, clamped to the source length.
    fn char_pos(&self, (line, col): (usize, usize)) -> usize {
        if line >= self.line_start.len() {
            return self.src.len();
        }
        (self.line_start[line] + col - 1).min(self.src.len())
    }

    /// The original source spanning `[from, to)`, with trailing whitespace trimmed.
    fn verbatim(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let a = self.char_pos(from);
        let b = self.char_pos(to).max(a);
        let s: String = self.src[a..b].iter().collect();
        s.trim_end().to_string()
    }

    fn indent(&mut self, level: usize) {
        for _ in 0..level {
            self.buf.push_str("  ");
        }
    }

    /// Current column (chars since the last newline) — the start position for the
    /// next thing to be printed.
    fn col(&self) -> usize {
        let line_start = self.buf.rfind('\n').map(|i| i + 1).unwrap_or(0);
        self.buf[line_start..].chars().count()
    }

    /// Render a container flat into `buf`; if it fits in the remaining width keep it,
    /// otherwise roll back and render the broken form. While measuring (and inside an
    /// already-flat context) nested containers stay inline too, so the measured width
    /// is the true single-line length. This is the layout decision in miniature — the
    /// `fits` check of a Wadler/Prettier-style pretty-printer, done by rendering.
    fn fit(&mut self, inline: impl Fn(&mut Self), broken: impl Fn(&mut Self)) {
        if self.force_flat {
            inline(self);
            return;
        }
        let mark = self.buf.len();
        let col = self.col();
        self.force_flat = true;
        inline(self);
        self.force_flat = false;
        if col + (self.buf.len() - mark) <= WIDTH {
            return;
        }
        self.buf.truncate(mark);
        broken(self);
    }

    // ---- Width-aware containers: inline when they fit, else one item per line. ----

    fn emit_object(&mut self, props: &[ObjectProp]) {
        if props.is_empty() {
            self.buf.push_str("{}");
            return;
        }
        self.fit(|s| s.object_inline(props), |s| s.object_broken(props));
    }

    fn object_inline(&mut self, props: &[ObjectProp]) {
        self.buf.push_str("{ ");
        for (i, pr) in props.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.object_prop(pr);
        }
        self.buf.push_str(" }");
    }

    fn object_broken(&mut self, props: &[ObjectProp]) {
        self.buf.push_str("{\n");
        self.depth += 1;
        for (i, pr) in props.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(",\n");
            }
            self.indent(self.depth);
            self.object_prop(pr);
        }
        self.depth -= 1;
        self.buf.push('\n');
        self.indent(self.depth);
        self.buf.push('}');
    }

    fn object_prop(&mut self, pr: &ObjectProp) {
        match pr {
            ObjectProp::KeyValue(k, v, _) => {
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

    fn emit_array(&mut self, elems: &[ArrayElement]) {
        if elems.is_empty() {
            self.buf.push_str("[]");
            return;
        }
        self.fit(|s| s.array_inline(elems), |s| s.array_broken(elems));
    }

    fn array_inline(&mut self, elems: &[ArrayElement]) {
        self.buf.push('[');
        for (i, el) in elems.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.array_elem(el);
        }
        self.buf.push(']');
    }

    fn array_broken(&mut self, elems: &[ArrayElement]) {
        self.buf.push_str("[\n");
        self.depth += 1;
        for (i, el) in elems.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(",\n");
            }
            self.indent(self.depth);
            self.array_elem(el);
        }
        self.depth -= 1;
        self.buf.push('\n');
        self.indent(self.depth);
        self.buf.push(']');
    }

    fn array_elem(&mut self, el: &ArrayElement) {
        match el {
            ArrayElement::Expr(ex) => self.expr(ex),
            ArrayElement::Spread(ex) => {
                self.buf.push_str("...");
                self.expr(ex);
            }
        }
    }

    /// Argument list `(...)`. A sole/last object|array|arrow argument "hugs" the
    /// parens — it breaks itself rather than forcing the whole list to break, e.g.
    /// `f(layout, [\n  …\n])`. Otherwise the list breaks as a unit when too wide.
    fn emit_args(&mut self, args: &[CallArg]) {
        if args.is_empty() {
            self.buf.push_str("()");
            return;
        }
        if !self.force_flat && self.last_arg_huggable(args) {
            self.buf.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    self.buf.push_str(", ");
                }
                self.call_arg(a);
            }
            self.buf.push(')');
            return;
        }
        self.fit(|s| s.args_inline(args), |s| s.args_broken(args));
    }

    fn last_arg_huggable(&self, args: &[CallArg]) -> bool {
        let huggable = |a: &CallArg| {
            matches!(
                a,
                CallArg::Expr(Expr::Object { .. })
                    | CallArg::Expr(Expr::Array { .. })
                    | CallArg::Expr(Expr::ArrowFunction { .. })
            )
        };
        let collection =
            |a: &CallArg| matches!(a, CallArg::Expr(Expr::Object { .. } | Expr::Array { .. }));
        match args.split_last() {
            Some((last, rest)) => huggable(last) && !rest.iter().any(collection),
            None => false,
        }
    }

    fn args_inline(&mut self, args: &[CallArg]) {
        self.buf.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.call_arg(a);
        }
        self.buf.push(')');
    }

    fn args_broken(&mut self, args: &[CallArg]) {
        self.buf.push_str("(\n");
        self.depth += 1;
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(",\n");
            }
            self.indent(self.depth);
            self.call_arg(a);
        }
        self.depth -= 1;
        self.buf.push('\n');
        self.indent(self.depth);
        self.buf.push(')');
    }

    fn call_arg(&mut self, a: &CallArg) {
        match a {
            CallArg::Expr(ex) => self.expr(ex),
            CallArg::Spread(ex) => {
                self.buf.push_str("...");
                self.expr(ex);
            }
        }
    }

    fn is_blank(&self, line: usize) -> bool {
        self.blank_lines.get(line).copied().unwrap_or(false)
    }

    /// Preserve a single blank line before the item starting at `next_line` when
    /// the source line directly above it was blank. Statement `span.end` is
    /// unreliable (it points at the next token), so spacing keys off the reliable
    /// start line and the source blank-line map rather than on span ranges.
    fn vspace(&mut self, next_line: usize) {
        if self.emitted != 0
            && self.is_blank(next_line.saturating_sub(1))
            && !self.buf.ends_with("\n\n")
        {
            self.buf.push('\n');
        }
    }

    /// Emit every pending comment positioned before `before`, each on its own line
    /// at `level` indentation, preserving source blank-line separation. Used at
    /// statement-sequence boundaries and before a block's closing brace.
    fn emit_leading_comments(&mut self, before: (usize, usize), level: usize) {
        while self.ci < self.comments.len() && self.comments[self.ci].start < before {
            let c = self.comments[self.ci].clone();
            self.ci += 1;
            self.vspace(c.start.0);
            self.indent(level);
            self.buf.push_str(&c.text);
            self.buf.push('\n');
            self.emitted = c.start.0;
            // A marker anywhere in the statement's leading comment group ignores it.
            if is_ignore_marker(&c.text) {
                self.ignore_next = true;
            }
        }
    }

    /// Emit trailing (same-line) comments sitting on source line `line`, inline
    /// after the just-printed statement text.
    fn emit_trailing_comments(&mut self, line: usize) {
        while self.ci < self.comments.len() {
            let c = &self.comments[self.ci];
            if c.own_line || c.start.0 != line {
                break;
            }
            let text = c.text.clone();
            self.ci += 1;
            self.buf.push(' ');
            self.buf.push_str(&text);
        }
    }

    /// Print a run of statements (top level, block body, or switch case body),
    /// interleaving recovered comments and preserving blank-line grouping. Trailing
    /// comments attach by the statement's reliable start line (single-line case);
    /// a trailing comment on a multi-line statement's last line simply migrates to
    /// a leading comment of the next item — preserved, never dropped.
    fn print_seq(&mut self, stmts: &[Statement], level: usize) {
        for (i, s) in stmts.iter().enumerate() {
            let sp = s.span();
            self.ignore_next = false;
            self.emit_leading_comments(sp.start, level);
            self.vspace(sp.start.0);
            if self.ignore_next {
                self.emit_ignored(s, i, stmts, level);
            } else {
                self.stmt(s, level);
            }
            self.emitted = sp.start.0;
            self.emit_trailing_comments(sp.start.0);
            self.buf.push('\n');
        }
    }

    /// Last source line covered by the statement at `start` — its start line, grown
    /// over the full lines of any bracket (`{}`/`[]`/`()`) it transitively opens. This
    /// bounds an ignored statement to its own extent regardless of what follows (next
    /// sibling, a `switch` case label, the enclosing `}`), so verbatim never overruns.
    fn ignored_last_line(&self, start: (usize, usize)) -> usize {
        let mut last = start.0;
        loop {
            let mut grew = false;
            for &(open_line, close_line) in &self.bracket_spans {
                if open_line >= start.0 && open_line <= last && close_line > last {
                    last = close_line;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        last
    }

    /// Emit a `// tish-fmt-ignore`-d statement as its original source, verbatim. The
    /// slice ends at the smallest of: the statement's own bracket extent, the next
    /// sibling, and the next own-line comment — so a same-line trailing comment is
    /// kept but neither the next statement's leading comments nor anything past this
    /// statement leaks in. Captured comments are skipped so they aren't re-emitted.
    fn emit_ignored(&mut self, s: &Statement, i: usize, stmts: &[Statement], level: usize) {
        let start = s.span().start;
        let mut boundary = (self.ignored_last_line(start) + 1, 1);
        if let Some(next) = stmts.get(i + 1) {
            boundary = boundary.min(next.span().start);
        }
        let mut j = self.ci;
        while j < self.comments.len() && self.comments[j].start < boundary {
            let c = &self.comments[j];
            if c.start > start && c.own_line {
                boundary = c.start;
                break;
            }
            j += 1;
        }
        self.indent(level);
        let text = self.verbatim(start, boundary);
        self.buf.push_str(&text);
        while self.ci < self.comments.len() && self.comments[self.ci].start < boundary {
            self.ci += 1;
        }
    }

    fn stmt(&mut self, s: &Statement, level: usize) {
        // Expression layout indents relative to this statement's level.
        self.depth = level;
        match s {
            Statement::Block { statements, span } => self.block(statements, *span, level, true),
            // Comma-declarators: render each as its own statement line. The caller
            // (print_seq) emits the trailing newline, so only separate internally.
            Statement::Multi { statements, .. } => {
                for (i, st) in statements.iter().enumerate() {
                    if i > 0 {
                        self.buf.push('\n');
                    }
                    self.stmt(st, level);
                }
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
                pattern,
                mutable,
                init,
                ..
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
            Statement::ForIn {
                name,
                object,
                body,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("for (let ");
                self.buf.push_str(name);
                self.buf.push_str(" in ");
                self.expr(object);
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
                    self.stmt_inline_or_block(body, level);
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
                    self.emitted = 0;
                    self.print_seq(stmts, level + 2);
                }
                if let Some(def) = default_body {
                    self.indent(level + 1);
                    self.buf.push_str("default:\n");
                    self.emitted = 0;
                    self.print_seq(def, level + 2);
                }
                self.indent(level);
                self.buf.push('}');
            }
            Statement::DoWhile { body, cond, .. } => {
                self.indent(level);
                self.buf.push_str("do ");
                self.stmt_inline_or_block(body, level);
                self.depth = level; // body recursion moved depth; restore for cond
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
            Statement::Import {
                specifiers, from, ..
            } => {
                self.indent(level);
                self.buf.push_str("import ");
                self.import_specs(specifiers);
                self.buf.push_str(" from ");
                self.string_lit(from.as_ref());
            }
            Statement::TypeAlias { name, ty, .. } => {
                self.indent(level);
                self.buf.push_str("type ");
                self.buf.push_str(name);
                self.buf.push_str(" = ");
                self.type_ann(ty);
            }
            Statement::DeclareVar {
                name,
                type_ann,
                const_,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("declare ");
                self.buf.push_str(if *const_ { "const " } else { "let " });
                self.buf.push_str(name);
                if let Some(t) = type_ann {
                    self.buf.push_str(": ");
                    self.type_ann(t);
                }
            }
            Statement::DeclareFun {
                async_,
                name,
                params,
                rest_param,
                return_type,
                ..
            } => {
                self.indent(level);
                self.buf.push_str("declare ");
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
            }
            Statement::Export { declaration, .. } => {
                self.indent(level);
                self.buf.push_str("export ");
                match declaration.as_ref() {
                    ExportDeclaration::Named(inner) => {
                        if let Statement::FunDecl {
                            async_,
                            name,
                            params,
                            rest_param,
                            return_type,
                            body,
                            ..
                        } = inner.as_ref()
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
                            self.stmt_inline_or_block(body, level);
                        } else {
                            self.stmt(inner, level);
                        }
                    }
                    ExportDeclaration::Default(e) => {
                        self.buf.push_str("default ");
                        self.expr(e);
                    }
                    // #305: re-export round-trip — `export { a, b as c } from "m"` / `export * from "m"`
                    ExportDeclaration::ReExport {
                        specifiers,
                        all,
                        from,
                        ..
                    } => {
                        if *all {
                            // `export *` always has a source.
                            self.buf.push_str("* from \"");
                            self.buf.push_str(from.as_deref().unwrap_or(""));
                            self.buf.push('"');
                        } else {
                            self.buf.push_str("{ ");
                            for (i, spec) in specifiers.iter().enumerate() {
                                if i > 0 {
                                    self.buf.push_str(", ");
                                }
                                if let tishlang_ast::ImportSpecifier::Named { name, alias, .. } =
                                    spec
                                {
                                    self.buf.push_str(name);
                                    if let Some(a) = alias {
                                        self.buf.push_str(" as ");
                                        self.buf.push_str(a);
                                    }
                                }
                            }
                            self.buf.push_str(" }");
                            // #415: a local named export (`export { a }`) has no `from` clause.
                            if let Some(from) = from {
                                self.buf.push_str(" from \"");
                                self.buf.push_str(from);
                                self.buf.push('"');
                            }
                        }
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

    /// Print a `{ … }` block. `lead_indent` controls whether the opening brace is
    /// indented (true for a standalone block statement) or written at the current
    /// position (false when it follows `if (…) `, `fn f() `, `else `, etc.).
    fn block(&mut self, statements: &[Statement], span: Span, level: usize, lead_indent: bool) {
        if lead_indent {
            self.indent(level);
        }
        self.buf.push_str("{\n");
        // Fresh sequence: suppress any blank line immediately after `{`.
        self.emitted = 0;
        self.print_seq(statements, level + 1);
        // Dangling comments between the last statement and `}` (e.g. a note inside an
        // otherwise-empty block). Bound by the real closing brace — `span.end`
        // overshoots to the next token. Brace-less (indent) blocks have no map entry,
        // so `span.start` flushes nothing and the comment migrates to the next sibling.
        let bound = self.braces.get(&span.start).copied().unwrap_or(span.start);
        self.emit_leading_comments(bound, level + 1);
        self.indent(level);
        self.buf.push('}');
        self.emitted = span.start.0;
    }

    fn stmt_inline_or_block(&mut self, s: &Statement, level: usize) {
        if let Statement::Block { statements, span } = s {
            self.block(statements, *span, level, false);
        } else {
            let sp = s.span();
            self.buf.push_str("{\n");
            self.emitted = 0;
            self.emit_leading_comments(sp.start, level + 1);
            self.stmt(s, level + 1);
            self.emitted = sp.start.0;
            self.emit_trailing_comments(sp.start.0);
            self.buf.push('\n');
            self.indent(level);
            self.buf.push('}');
            self.emitted = sp.start.0;
        }
    }

    fn import_specs(&mut self, specs: &[ImportSpecifier]) {
        if specs.len() == 1 {
            match &specs[0] {
                ImportSpecifier::Default { name, .. } => self.buf.push_str(name.as_ref()),
                ImportSpecifier::Namespace { name, .. } => {
                    self.buf.push_str("* as ");
                    self.buf.push_str(name.as_ref());
                }
                ImportSpecifier::Named { name, alias, .. } => {
                    self.buf.push_str("{ ");
                    self.import_named(name.as_ref(), alias.as_deref());
                    self.buf.push_str(" }");
                }
            }
            return;
        }
        // A long named-import list wraps one name per line.
        self.fit(
            |s| s.import_list_inline(specs),
            |s| s.import_list_broken(specs),
        );
    }

    fn import_named(&mut self, name: &str, alias: Option<&str>) {
        self.buf.push_str(name);
        if let Some(a) = alias {
            self.buf.push_str(" as ");
            self.buf.push_str(a);
        }
    }

    fn import_list_inline(&mut self, specs: &[ImportSpecifier]) {
        self.buf.push_str("{ ");
        for (i, sp) in specs.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            if let ImportSpecifier::Named { name, alias, .. } = sp {
                self.import_named(name.as_ref(), alias.as_deref());
            }
        }
        self.buf.push_str(" }");
    }

    fn import_list_broken(&mut self, specs: &[ImportSpecifier]) {
        self.buf.push_str("{\n");
        self.depth += 1;
        for (i, sp) in specs.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(",\n");
            }
            self.indent(self.depth);
            if let ImportSpecifier::Named { name, alias, .. } = sp {
                self.import_named(name.as_ref(), alias.as_deref());
            }
        }
        self.depth -= 1;
        self.buf.push('\n');
        self.indent(self.depth);
        self.buf.push('}');
    }

    fn param_list(&mut self, params: &[FunParam], rest: &Option<TypedParam>) {
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            match p {
                FunParam::Simple(tp) => {
                    self.buf.push_str(tp.name.as_ref());
                    if let Some(t) = &tp.type_ann {
                        self.buf.push_str(": ");
                        self.type_ann(t);
                    }
                    if let Some(e) = &tp.default {
                        self.buf.push_str(" = ");
                        self.expr(e);
                    }
                }
                FunParam::Destructure {
                    pattern,
                    type_ann,
                    default,
                } => {
                    self.destruct_pat(pattern);
                    if let Some(t) = type_ann {
                        self.buf.push_str(": ");
                        self.type_ann(t);
                    }
                    if let Some(e) = default {
                        self.buf.push_str(" = ");
                        self.expr(e);
                    }
                }
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
                        Some(DestructElement::Ident(n, _)) => self.buf.push_str(n.as_ref()),
                        Some(DestructElement::Pattern(inner)) => self.destruct_pat(inner),
                        Some(DestructElement::Rest(n, _)) => {
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
                        DestructElement::Ident(n, _) if n.as_ref() != pr.key.as_ref() => {
                            self.buf.push_str(": ");
                            self.buf.push_str(n.as_ref());
                        }
                        DestructElement::Ident(_, _) => {}
                        DestructElement::Pattern(inner) => {
                            self.buf.push_str(": ");
                            self.destruct_pat(inner);
                        }
                        DestructElement::Rest(n, _) => {
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
            TypeAnnotation::Simple(s, _) => self.buf.push_str(s.as_ref()),
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
            TypeAnnotation::Tuple(elems) => {
                self.buf.push('[');
                for (i, x) in elems.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.type_ann(x);
                }
                self.buf.push(']');
            }
            TypeAnnotation::Literal(lit) => match lit {
                tishlang_ast::TypeLiteral::Str(s) => {
                    self.buf.push('"');
                    self.buf.push_str(s.as_ref());
                    self.buf.push('"');
                }
                tishlang_ast::TypeLiteral::Num(n) => self.buf.push_str(&n.to_string()),
                tishlang_ast::TypeLiteral::Bool(b) => self.buf.push_str(&b.to_string()),
            },
            TypeAnnotation::Intersection(parts) => {
                for (i, x) in parts.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(" & ");
                    }
                    self.type_ann(x);
                }
            }
        }
    }

    /// Print `e` as a sub-expression, wrapping it in parentheses when its operator
    /// binds looser than `min_prec` (so the printed form re-parses to the same AST).
    fn child(&mut self, e: &Expr, min_prec: u8) {
        if expr_prec(e) < min_prec {
            self.buf.push('(');
            self.expr(e);
            self.buf.push(')');
        } else {
            self.expr(e);
        }
    }

    /// Print JSX children inline and verbatim. JSX whitespace is significant in tish, so text is
    /// emitted exactly as written and there is no reflow/indentation. A nested element/fragment is
    /// printed bare (as a child element); any other expression child gets `{ }`.
    ///
    /// Do NOT re-indent children by structural depth here: any injected newline/space becomes a
    /// real `JsxChild::Text` node and changes the rendered output (and breaks idempotency). The
    /// source's own layout inside the children is preserved as-is via the verbatim text nodes.
    fn jsx_children(&mut self, children: &[JsxChild]) {
        for ch in children {
            match ch {
                JsxChild::Text(t) => self.buf.push_str(t.as_ref()),
                JsxChild::Expr(e) => {
                    if matches!(e, Expr::JsxElement { .. } | Expr::JsxFragment { .. }) {
                        self.expr(e);
                    } else {
                        self.buf.push('{');
                        self.expr(e);
                        self.buf.push('}');
                    }
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Literal { value, .. } => match value {
                Literal::Number(n) => {
                    if !n.is_finite() {
                        // f64 overflow / NaN: emit the tish globals, not Rust's bare `inf`/`-inf`/`NaN`
                        // (`inf` would re-parse as an undefined identifier).
                        self.buf.push_str(if n.is_nan() {
                            "NaN"
                        } else if *n > 0.0 {
                            "Infinity"
                        } else {
                            "-Infinity"
                        });
                    } else if n.fract() == 0.0 && n.abs() < 1e15 {
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
            Expr::Binary {
                left, op, right, ..
            } => {
                // Parenthesize operands by precedence/associativity so the printed
                // grouping re-parses to the same tree (the AST has no paren nodes).
                let p = binop_prec(*op);
                let right_assoc = matches!(op, BinOp::Pow);
                let (lmin, rmin) = if right_assoc { (p + 1, p) } else { (p, p + 1) };
                self.child(left, lmin);
                self.buf.push(' ');
                self.buf.push_str(binop(*op));
                self.buf.push(' ');
                self.child(right, rmin);
            }
            Expr::Unary { op, operand, .. } => {
                match op {
                    UnaryOp::Not => self.buf.push('!'),
                    UnaryOp::Neg => self.buf.push('-'),
                    UnaryOp::Pos => self.buf.push('+'),
                    UnaryOp::BitNot => self.buf.push('~'),
                    UnaryOp::Void => self.buf.push_str("void "),
                }
                self.child(operand, PREC_POSTFIX);
            }
            Expr::Call { callee, args, .. } => {
                self.child(callee, PREC_POSTFIX);
                self.emit_args(args);
            }
            Expr::New { callee, args, .. } => {
                self.buf.push_str("new ");
                // `new` parses its callee as a member expression WITHOUT a trailing call, so any call
                // in the callee's spine (`new (factory())()`, `new (factory().Cls)()`) must be
                // parenthesized — otherwise the printed `()` rebinds as the constructor's argument
                // list. child()'s precedence check can't catch this because a Call/Member already has
                // postfix precedence.
                if new_callee_has_call(callee) {
                    self.buf.push('(');
                    self.expr(callee);
                    self.buf.push(')');
                } else {
                    self.child(callee, PREC_POSTFIX);
                }
                if !args.is_empty() {
                    self.emit_args(args);
                }
            }
            Expr::Member {
                object,
                prop,
                optional,
                ..
            } => {
                self.child(object, PREC_POSTFIX);
                if *optional {
                    self.buf.push_str("?.");
                } else {
                    self.buf.push('.');
                }
                match prop {
                    MemberProp::Name { name, .. } => self.buf.push_str(name.as_ref()),
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
                self.child(object, PREC_POSTFIX);
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
                // cond binds tighter than `?:`; the else chains (right-assoc).
                self.child(cond, PREC_NULLISH);
                self.buf.push_str(" ? ");
                self.expr(then_branch);
                self.buf.push_str(" : ");
                self.child(else_branch, PREC_CONDITIONAL);
            }
            Expr::NullishCoalesce { left, right, .. } => {
                self.child(left, PREC_NULLISH);
                self.buf.push_str(" ?? ");
                self.child(right, PREC_NULLISH + 1);
            }
            Expr::Array { elements, .. } => self.emit_array(elements),
            Expr::Object { props, .. } => self.emit_object(props),
            Expr::Assign { name, value, .. } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str(" = ");
                self.expr(value);
            }
            Expr::TypeOf { operand, .. } => {
                self.buf.push_str("typeof ");
                self.child(operand, PREC_POSTFIX);
            }
            Expr::Delete { target, .. } => {
                self.buf.push_str("delete ");
                self.child(target, PREC_POSTFIX);
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
            Expr::CompoundAssign {
                name, op, value, ..
            } => {
                self.buf.push_str(name.as_ref());
                self.buf.push_str(compound(*op));
                self.expr(value);
            }
            Expr::LogicalAssign {
                name, op, value, ..
            } => {
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
                self.child(object, PREC_POSTFIX);
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
                self.child(object, PREC_POSTFIX);
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
                    // A bare object-literal body must be parenthesized, else `=> {`
                    // re-parses as a block.
                    ArrowBody::Expr(e) => {
                        if matches!(e.as_ref(), Expr::Object { .. }) {
                            self.buf.push('(');
                            self.expr(e);
                            self.buf.push(')');
                        } else {
                            self.expr(e);
                        }
                    }
                    // Indent the block body relative to the arrow's own line
                    // (`self.depth`), printed inline after `=> ` (no leading indent).
                    ArrowBody::Block(b) => self.stmt_inline_or_block(b, self.depth),
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
                self.child(operand, PREC_POSTFIX);
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
                            // A leading space separates this prop from the tag/prior prop; the
                            // closing brace must NOT carry a trailing space, or a following attr
                            // (which prepends its own space) would render as a double space.
                            self.buf.push_str(" {...");
                            self.expr(e);
                            self.buf.push('}');
                        }
                    }
                }
                if children.is_empty() {
                    self.buf.push_str(" />");
                } else {
                    // JSX whitespace is significant in tish (the lexer keeps it verbatim and codegen
                    // emits it as content), so children are printed inline and verbatim. Reflowing
                    // them onto indented lines injected newlines/spaces as real text nodes, which
                    // changed the rendered output and was non-idempotent.
                    self.buf.push('>');
                    self.jsx_children(children);
                    self.buf.push_str("</");
                    self.buf.push_str(tag.as_ref());
                    self.buf.push('>');
                }
            }
            Expr::JsxFragment { children, .. } => {
                self.buf.push_str("<>");
                self.jsx_children(children);
                self.buf.push_str("</>");
            }
            Expr::NativeModuleLoad {
                spec, export_name, ..
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
    // In a template literal only backslash, backtick, and a `$` that begins an interpolation (`${`)
    // need escaping. A bare `$` is literal, so escaping every `$` just adds spurious backslashes.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '`' => out.push_str("\\`"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            _ => out.push(c),
        }
    }
    out
}

// Expression precedence levels (higher binds tighter), mirroring the parser's
// descent chain (parse_conditional → … → parse_unary → primary). Used to decide
// when a sub-expression needs parentheses.
const PREC_CONDITIONAL: u8 = 1;
const PREC_NULLISH: u8 = 2;
const PREC_POSTFIX: u8 = 15; // call / member / index — tighter than any operator
const PREC_ATOM: u8 = 16; // literals, identifiers, array/object/template/jsx

/// True when a comment is exactly the ignore marker — `// tish-fmt-ignore` or the
/// `/* tish-fmt-ignore */` block form.
fn is_ignore_marker(text: &str) -> bool {
    let t = text.trim();
    let inner = if let Some(r) = t.strip_prefix("//") {
        r
    } else if let Some(r) = t.strip_prefix("/*").and_then(|x| x.strip_suffix("*/")) {
        r
    } else {
        return false;
    };
    inner.trim() == IGNORE_MARKER
}

/// Precedence of a binary operator, matching the parser (parser.rs `parse_*`).
fn binop_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 3,
        BinOp::And => 4,
        BinOp::BitOr => 5,
        BinOp::BitXor => 6,
        BinOp::BitAnd => 7,
        BinOp::Shl | BinOp::Shr | BinOp::UShr => 8,
        BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe => 9,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 10,
        BinOp::Add | BinOp::Sub => 11,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 12,
        BinOp::Pow => 13,
    }
}

/// Precedence of an expression's outermost operator (atoms bind tightest).
fn expr_prec(e: &Expr) -> u8 {
    match e {
        Expr::Assign { .. }
        | Expr::MemberAssign { .. }
        | Expr::IndexAssign { .. }
        | Expr::CompoundAssign { .. }
        | Expr::LogicalAssign { .. } => 0,
        Expr::Conditional { .. } => PREC_CONDITIONAL,
        Expr::NullishCoalesce { .. } => PREC_NULLISH,
        Expr::Binary { op, .. } => binop_prec(*op),
        Expr::Unary { .. }
        | Expr::TypeOf { .. }
        | Expr::Delete { .. }
        | Expr::Await { .. }
        | Expr::PrefixInc { .. }
        | Expr::PrefixDec { .. } => 14,
        Expr::Call { .. }
        | Expr::Member { .. }
        | Expr::Index { .. }
        | Expr::PostfixInc { .. }
        | Expr::PostfixDec { .. } => PREC_POSTFIX,
        // `new X` binds looser than member/call, so a New used as the object of `.`/`[]`/`()`
        // must be parenthesized (e.g. `(new Foo()).bar()`, not `new Foo().bar()`).
        Expr::New { .. } => PREC_POSTFIX - 1,
        Expr::ArrowFunction { .. } => PREC_ATOM,
        _ => PREC_ATOM,
    }
}

/// True when a `new` callee contains a call in its member/index spine. Such a callee must be
/// parenthesized, because `new` parses its callee without a trailing call and would otherwise bind
/// the first `()` as the constructor's argument list (`new (f())()`, `new (f().C)()`).
fn new_callee_has_call(e: &Expr) -> bool {
    match e {
        Expr::Call { .. } => true,
        Expr::Member { object, .. } | Expr::Index { object, .. } => new_callee_has_call(object),
        _ => false,
    }
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
        BinOp::UShr => ">>>",
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

/// Recover `//` and `/* */` comments from source, in order, with their 1-based
/// (line, col) positions — matching `ast::Span`'s convention so the printer can
/// re-insert them by position. The lexer discards comments, so this is a separate
/// pass; it skips string and template literals so a `//` inside `"…"` or `` `…` ``
/// is never mistaken for a comment. (Tish has no regex literals, so a bare `/` is
/// only division or a comment opener — no further disambiguation is needed.)
fn scan_comments(source: &str) -> (Vec<CommentTok>, BraceMap, Vec<(usize, usize)>) {
    let mut s = Scanner {
        chars: source.chars().collect(),
        i: 0,
        line: 1,
        col: 1,
        seen_nonws: false,
        out: Vec::new(),
        brace_stack: Vec::new(),
        braces: BraceMap::new(),
        bracket_open: Vec::new(),
        bracket_spans: Vec::new(),
    };
    s.scan_code(false);
    (s.out, s.braces, s.bracket_spans)
}

struct Scanner {
    chars: Vec<char>,
    i: usize,
    line: usize,
    col: usize,
    /// Whether any non-whitespace has appeared on the current line yet.
    seen_nonws: bool,
    out: Vec<CommentTok>,
    /// Open `{` positions awaiting their match, for building `braces`.
    brace_stack: Vec<(usize, usize)>,
    /// `{`→`}` position pairs (see [`BraceMap`]).
    braces: BraceMap,
    /// Open lines of every bracket (`{ [ (`) awaiting its match.
    bracket_open: Vec<usize>,
    /// `(open_line, close_line)` for every bracket pair — lets an ignored statement's
    /// verbatim slice extend over the full lines of any `{}`/`[]`/`()` it opens.
    bracket_spans: Vec<(usize, usize)>,
}

impl Scanner {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.i + 1).copied()
    }

    /// Consume one char, tracking line/col like the lexer (col resets after `\n`).
    fn bump(&mut self) -> Option<char> {
        let c = *self.chars.get(self.i)?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
            self.seen_nonws = false;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// Scan ordinary code, collecting comments. When `stop_at_close_brace` is set
    /// (inside a `${ … }` template interpolation), returns at the matching `}`
    /// without consuming it.
    fn scan_code(&mut self, stop_at_close_brace: bool) {
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            match c {
                '}' if stop_at_close_brace && depth == 0 => return,
                '{' => {
                    self.brace_stack.push((self.line, self.col));
                    self.bracket_open.push(self.line);
                    depth += 1;
                    self.bump();
                    self.seen_nonws = true;
                }
                '}' => {
                    let close = (self.line, self.col);
                    if let Some(open) = self.brace_stack.pop() {
                        self.braces.insert(open, close);
                    }
                    if let Some(open_line) = self.bracket_open.pop() {
                        self.bracket_spans.push((open_line, self.line));
                    }
                    depth = depth.saturating_sub(1);
                    self.bump();
                    self.seen_nonws = true;
                }
                '(' | '[' => {
                    self.bracket_open.push(self.line);
                    self.bump();
                    self.seen_nonws = true;
                }
                ')' | ']' => {
                    if let Some(open_line) = self.bracket_open.pop() {
                        self.bracket_spans.push((open_line, self.line));
                    }
                    self.bump();
                    self.seen_nonws = true;
                }
                '"' | '\'' => self.scan_string(c),
                '`' => {
                    self.bump();
                    self.seen_nonws = true;
                    self.scan_template();
                }
                '/' if self.peek2() == Some('/') => self.scan_line_comment(),
                '/' if self.peek2() == Some('*') => self.scan_block_comment(),
                ' ' | '\t' | '\r' | '\n' => {
                    self.bump();
                }
                _ => {
                    self.bump();
                    self.seen_nonws = true;
                }
            }
        }
    }

    fn scan_string(&mut self, quote: char) {
        self.bump(); // opening quote
        self.seen_nonws = true;
        while let Some(c) = self.bump() {
            if c == '\\' {
                self.bump(); // escaped char
            } else if c == quote {
                break;
            }
        }
    }

    /// Scan a template literal body (opening backtick already consumed), recursing
    /// into `${ … }` interpolations so their strings/comments are handled too.
    fn scan_template(&mut self) {
        while let Some(c) = self.peek() {
            match c {
                '`' => {
                    self.bump();
                    return;
                }
                '\\' => {
                    self.bump();
                    self.bump();
                }
                '$' if self.peek2() == Some('{') => {
                    self.bump();
                    self.bump();
                    self.scan_code(true);
                    if self.peek() == Some('}') {
                        self.bump();
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn scan_line_comment(&mut self) {
        let start = (self.line, self.col);
        let own_line = !self.seen_nonws;
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            text.push(c);
            self.bump();
        }
        self.out.push(CommentTok {
            start,
            text: text.trim_end().to_string(),
            own_line,
        });
    }

    fn scan_block_comment(&mut self) {
        let start = (self.line, self.col);
        let own_line = !self.seen_nonws;
        let mut text = String::new();
        let mut depth = 0usize;
        loop {
            if self.peek() == Some('/') && self.peek2() == Some('*') {
                text.push('/');
                text.push('*');
                self.bump();
                self.bump();
                depth += 1;
            } else if self.peek() == Some('*') && self.peek2() == Some('/') {
                text.push('*');
                text.push('/');
                self.bump();
                self.bump();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else {
                match self.bump() {
                    Some(c) => text.push(c),
                    None => break,
                }
            }
        }
        self.out.push(CommentTok {
            start,
            text,
            own_line,
        });
        self.seen_nonws = true;
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

    /// Debug-render the parsed AST with all `Span { … }` contents removed, so two programs compare
    /// equal iff they are structurally identical (reformatting legitimately changes spans).
    fn structure(src: &str) -> String {
        let dbg = format!("{:#?}", tishlang_parser::parse(src).unwrap().statements);
        let mut out = String::new();
        let mut rest = dbg.as_str();
        while let Some(i) = rest.find("Span {") {
            out.push_str(&rest[..i]);
            match rest[i..].find('}') {
                Some(j) => rest = &rest[i + j + 1..],
                None => {
                    rest = "";
                    break;
                }
            }
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn new_expr_preserves_structure_and_is_idempotent() {
        // Each input must format to a program with the SAME structure (modulo spans), and a second
        // format pass must be a no-op. These previously corrupted into structurally different
        // programs (Call<->New<->Member swaps) on format.
        for src in [
            "const d = (new Foo()).bar()\n",
            "const b = new (factory().Cls)()\n",
            "const x = new (getClass())(1)\n",
            "const e = new Foo()\n",
            "const f = new a.b.c()\n",
            "const g = new Foo(1, 2).baz\n",
        ] {
            let out = format_source(src).unwrap();
            let out2 = format_source(&out).unwrap();
            assert_eq!(structure(src), structure(&out), "structure changed: {src:?} -> {out:?}");
            assert_eq!(out, out2, "not idempotent: {src:?} -> {out:?} -> {out2:?}");
        }
    }

    #[test]
    fn jsx_preserves_structure_and_is_idempotent() {
        // JSX whitespace is significant; formatting must not inject newlines/indentation as text
        // (which changed rendered output and was non-idempotent). Structure must be preserved.
        for src in [
            "let x = <div>a{b}</div>\n",
            "const e = <div>Hello {name}!</div>\n",
            "const e2 = <div><span>a</span><span>b</span></div>\n",
            "let y = <div>  spaced  text  </div>\n",
            "let z = <>{a}{b}</>\n",
            "let s = <div a={1} b=\"x\">{c}</div>\n",
        ] {
            let out = format_source(src).unwrap();
            let out2 = format_source(&out).unwrap();
            assert_eq!(structure(src), structure(&out), "JSX structure changed: {src:?} -> {out:?}");
            assert_eq!(out, out2, "JSX not idempotent: {src:?} -> {out:?} -> {out2:?}");
        }
    }

    #[test]
    fn multiline_jsx_children_are_bare_and_verbatim() {
        // #157: multi-line JSX nested inside an arrow body must print element children BARE (not
        // `{<child>}`-wrapped), must NOT emit stray blank `  ` lines, and must NOT re-indent to a
        // hardcoded 2-space depth — JSX whitespace is significant, so the source layout is kept
        // verbatim. This golden string locks the corrected output.
        let src = "const x = () => {\n  return <div>\n    <span>hello</span>\n    <span>world</span>\n  </div>\n}\n";
        let out = format_source(src).unwrap();
        assert_eq!(
            out,
            "const x = () => {\n  return <div>\n    <span>hello</span>\n    <span>world</span>\n  </div>\n}\n",
            "multi-line JSX corrupted: {out:?}"
        );
        // No curly-wrapped element children, and no blank two-space line.
        assert!(!out.contains("{<"), "element child was brace-wrapped: {out:?}");
        assert!(!out.contains("\n  \n"), "stray blank two-space line: {out:?}");
        // Idempotent.
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent: {out:?}");
    }

    #[test]
    fn multiline_jsx_fragment_children_are_bare_and_verbatim() {
        // #157: the JsxFragment path shares the same layout logic as JsxElement; verify a
        // multi-line fragment keeps its children bare and its source layout verbatim.
        let src = "const y = () => {\n  return <>\n    <span>a</span>\n    <span>b</span>\n  </>\n}\n";
        let out = format_source(src).unwrap();
        assert_eq!(
            out,
            "const y = () => {\n  return <>\n    <span>a</span>\n    <span>b</span>\n  </>\n}\n",
            "multi-line JSX fragment corrupted: {out:?}"
        );
        assert!(!out.contains("{<"), "fragment element child was brace-wrapped: {out:?}");
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent: {out:?}");
    }

    #[test]
    fn non_finite_number_literal_emits_valid_token() {
        // 1e400 overflows f64 to infinity; it must format to the `Infinity` global, not Rust's bare
        // `inf` (which would re-parse as an undefined identifier).
        let src = "let x = 1e400\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("Infinity"), "expected Infinity, got {out:?}");
        tishlang_parser::parse(&out).expect("formatted output must parse");
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent: {out:?}");
    }

    #[test]
    fn template_literal_escapes_only_interpolation_dollar() {
        let src = "let p = `cost: $5 and ${x} done`\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("cost: $5"), "a bare $ must not be escaped: {out:?}");
        assert_eq!(structure(src), structure(&out), "structure changed: {src:?} -> {out:?}");
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent: {out:?}");
    }

    #[test]
    fn preserves_leading_and_section_comments() {
        let src = "\
// file header
// second line
let a = 1

// a section
let b = 2
";
        let out = format_source(src).unwrap();
        assert!(out.contains("// file header"), "{out:?}");
        assert!(out.contains("// second line"), "{out:?}");
        assert!(out.contains("// a section"), "{out:?}");
        // header hugs the statement it documents; blank line before the section.
        assert!(out.contains("// a section\nlet b = 2"), "{out:?}");
        let _ = tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn preserves_trailing_comment() {
        let src = "let a = 1 // inline note\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("let a = 1 // inline note"), "{out:?}");
        let _ = tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn preserves_comments_inside_block() {
        let src = "\
fn f() {
  // step one
  let a = 1
  // step two
  return a
}
";
        let out = format_source(src).unwrap();
        assert!(out.contains("  // step one"), "{out:?}");
        assert!(out.contains("  // step two"), "{out:?}");
        let _ = tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn preserves_dangling_comment_in_empty_block() {
        let src = "fn f() {\n  // nothing yet\n}\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("// nothing yet"), "{out:?}");
        let _ = tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn preserves_block_comment() {
        let src = "/* a block comment */\nlet a = 1\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("/* a block comment */"), "{out:?}");
        let _ = tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn double_slash_in_string_is_not_a_comment() {
        let src = "let url = \"http://example.com\"\n";
        let out = format_source(src).unwrap();
        assert_eq!(out, "let url = \"http://example.com\"\n", "{out:?}");
    }

    #[test]
    fn idempotent_with_comments() {
        let src = "\
// header
let a = 1

fn f() {
  // body note
  let b = 2 // trailing
  return b
}
";
        let once = format_source(src).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(
            once, twice,
            "formatting is not idempotent:\n{once}\n---\n{twice}"
        );
    }

    #[test]
    fn collapses_multiple_blank_lines_to_one() {
        let src = "let a = 1\n\n\n\nlet b = 2\n";
        let out = format_source(src).unwrap();
        assert_eq!(out, "let a = 1\n\nlet b = 2\n", "{out:?}");
    }

    #[test]
    fn comment_after_inner_block_stays_at_outer_level() {
        // Regression: a comment after an inner block's `}` belongs to the outer
        // scope, not inside the inner block (the parser's block span.end overshoots
        // past `}`, which previously pulled this comment in and broke idempotency).
        let src = "\
fn f() {
  if (x) {
    a()
  }
  // after the if
  b()
}
";
        let out = format_source(src).unwrap();
        assert!(out.contains("  // after the if\n  b()"), "{out:?}");
        let twice = format_source(&out).unwrap();
        assert_eq!(out, twice, "not idempotent:\n{out}\n---\n{twice}");
    }

    #[test]
    fn trailing_comment_inside_block_stays_inside() {
        let src = "\
fn f() {
  a()
  // last note
}
";
        let out = format_source(src).unwrap();
        assert!(out.contains("  // last note\n}"), "{out:?}");
        let twice = format_source(&out).unwrap();
        assert_eq!(out, twice, "{out:?}");
    }

    #[test]
    fn preserves_operator_grouping() {
        // The AST has no parenthesis nodes, so the printer must re-derive parens
        // from precedence. Each `want` must re-parse to the same tree.
        let cases = [
            ("let a = 1 / (b - c)\n", "1 / (b - c)"),
            ("let a = 0 - (x + y + z)\n", "0 - (x + y + z)"),
            ("let a = (1 - (p + q)) * s\n", "(1 - (p + q)) * s"),
            ("let a = b * c + d\n", "b * c + d"),
            ("let a = b + c * d\n", "b + c * d"),
            ("let a = (b + c) * d\n", "(b + c) * d"),
            ("let a = 1 - 2 - 3\n", "1 - 2 - 3"),
            ("let a = 1 - (2 - 3)\n", "1 - (2 - 3)"),
            ("let a = -(b + c)\n", "-(b + c)"),
            ("let a = !(b && c)\n", "!(b && c)"),
            ("let a = (a | b) & c\n", "(a | b) & c"),
        ];
        for (src, want) in cases {
            let out = format_source(src).unwrap();
            assert!(
                out.contains(want),
                "for {src:?} expected to contain {want:?}, got {out:?}"
            );
            let twice = format_source(&out).unwrap();
            assert_eq!(out, twice, "not idempotent for {src:?}: {out:?}");
        }
    }

    #[test]
    fn nested_control_flow_brace_spacing() {
        let src = "fn f() {\n  if (a) {\n    b()\n  }\n}\n";
        let out = format_source(src).unwrap();
        assert!(!out.contains(")   {"), "double-space before brace: {out:?}");
        assert!(out.contains("  if (a) {\n"), "{out:?}");
        let twice = format_source(&out).unwrap();
        assert_eq!(out, twice, "{out:?}");
    }

    #[test]
    fn short_object_stays_inline() {
        let src = "let a = { x: 1, y: 2 }\n";
        assert_eq!(format_source(src).unwrap(), src);
    }

    #[test]
    fn long_object_breaks_one_per_line() {
        let props: Vec<String> = (0..12).map(|i| format!("key{i}: {i}")).collect();
        let src = format!("let a = {{ {} }}\n", props.join(", "));
        let out = format_source(&src).unwrap();
        assert!(
            out.contains("let a = {\n  key0: 0,\n"),
            "expected broken object:\n{out}"
        );
        assert!(out.ends_with("\n}\n"), "{out:?}");
        // idempotent and re-parses
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent:\n{out}");
        tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn long_array_breaks_one_per_line() {
        let elems: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        let src = format!("let a = [{}]\n", elems.join(", "));
        let out = format_source(&src).unwrap();
        assert!(
            out.contains("[\n  0,\n  1,\n"),
            "expected broken array:\n{out}"
        );
        assert_eq!(format_source(&out).unwrap(), out);
    }

    #[test]
    fn last_arg_object_hugs_parens() {
        let props: Vec<String> = (0..20).map(|i| format!("k{i}: {i}")).collect();
        let src = format!("f(a, {{ {} }})\n", props.join(", "));
        let out = format_source(&src).unwrap();
        assert!(
            out.starts_with("f(a, {\n"),
            "expected hugged object:\n{out}"
        );
        assert!(out.contains("\n})\n"), "expected hugged close:\n{out}");
        assert_eq!(format_source(&out).unwrap(), out);
        tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn nested_containers_indent_progressively() {
        let inner: Vec<String> = (0..16).map(|i| format!("p{i}: {i}")).collect();
        let src = format!("let a = {{ outer: {{ {} }} }}\n", inner.join(", "));
        let out = format_source(&src).unwrap();
        // outer object at 2 spaces, inner props at 4 spaces
        assert!(
            out.contains("\n  outer: {\n    p0: 0,"),
            "expected nested indent:\n{out}"
        );
        assert_eq!(format_source(&out).unwrap(), out);
    }

    #[test]
    fn arrow_block_body_indents_to_context() {
        let src =
            "export fn make() {\n  let s = {}\n  s.go = (x) => {\n    foo(x)\n  }\n  return s\n}\n";
        let out = format_source(src).unwrap();
        // Arrow body one level past its `s.go` line (4 spaces); closing `}` at 2.
        assert!(
            out.contains("  s.go = (x) => {\n    foo(x)\n  }\n"),
            "arrow body mis-indented:\n{out}"
        );
        assert_eq!(format_source(&out).unwrap(), out);
    }

    #[test]
    fn ignore_marker_preserves_statement_verbatim() {
        let src = "// tish-fmt-ignore\nexport fn m(out) {\n  out[0]=1;  out[1]=0\n  out[2]=0; out[3]=1\n}\n\nlet x = {a:1,b:2}\n";
        let out = format_source(src).unwrap();
        // The ignored function keeps its exact source (aligned, no spaces around `=`).
        assert!(
            out.contains("export fn m(out) {\n  out[0]=1;  out[1]=0\n  out[2]=0; out[3]=1\n}"),
            "ignored block not verbatim:\n{out}"
        );
        // Surrounding code is still formatted normally.
        assert!(
            out.contains("let x = { a: 1, b: 2 }"),
            "neighbour not formatted:\n{out}"
        );
        // The marker itself is kept.
        assert!(out.contains("// tish-fmt-ignore\n"), "{out}");
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent:\n{out}");
        tishlang_parser::parse(&out).unwrap();
    }

    #[test]
    fn ignore_marker_block_comment_form() {
        let src = "/* tish-fmt-ignore */\nlet a = [1,2,   3]\n";
        let out = format_source(src).unwrap();
        assert!(
            out.contains("let a = [1,2,   3]"),
            "expected verbatim array:\n{out}"
        );
    }

    #[test]
    fn ignore_in_switch_case_does_not_overrun() {
        // Regression: an ignored last statement of a non-final case must not swallow
        // the following cases / closing brace / trailing code.
        let src = "switch (x) {\n  case 1:\n    // tish-fmt-ignore\n    foo( a,b )\n  case 2:\n    bar()\n}\nlet after = 1\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("foo( a,b )"), "ignored not verbatim:\n{out}");
        assert!(out.contains("case 2:"), "case 2 was swallowed:\n{out}");
        assert!(out.contains("bar()"), "case 2 body swallowed:\n{out}");
        assert!(
            out.contains("let after = 1"),
            "trailing code swallowed:\n{out}"
        );
        tishlang_parser::parse(&out).unwrap();
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent:\n{out}");
    }

    #[test]
    fn ignore_preserves_multiline_bracket_statement() {
        // The verbatim extent must follow `[]`/`()`, not just `{}`.
        let src = "// tish-fmt-ignore\nlet m = [\n  1,2,\n  3,4\n]\nlet n = 5\n";
        let out = format_source(src).unwrap();
        assert!(
            out.contains("let m = [\n  1,2,\n  3,4\n]"),
            "multiline array truncated:\n{out}"
        );
        assert!(out.contains("let n = 5"), "{out}");
        assert_eq!(format_source(&out).unwrap(), out);
    }

    #[test]
    fn without_marker_is_formatted_normally() {
        let src = "let a = [1,2,   3]\n";
        let out = format_source(src).unwrap();
        assert_eq!(out, "let a = [1, 2, 3]\n", "{out:?}");
    }

    #[test]
    fn no_comments_round_trips_without_loss() {
        let src = "\
fn add(a, b) {
  return a + b
}

let x = add(1, 2)
";
        let out = format_source(src).unwrap();
        assert_eq!(out, src, "{out:?}");
    }

    #[test]
    fn formats_delete_expression() {
        // Regression: Expr::Delete (the `delete` operator) must be handled by the formatter —
        // a non-exhaustive `match` here broke the `tish-format` build once the delete feature landed.
        let src = "fn f(o, k) {\ndelete o.a\ndelete o[\"b\"]\nlet x = delete o[k]\nreturn x\n}\n";
        let out = format_source(src).unwrap();
        assert!(out.contains("delete o.a"), "{out}");
        assert!(out.contains("delete o[\"b\"]"), "{out}");
        assert!(out.contains("delete o[k]"), "{out}");
        tishlang_parser::parse(&out).unwrap();
        assert_eq!(format_source(&out).unwrap(), out, "not idempotent:\n{out}");
    }

    /// Broad idempotence guard (#163): the existing tests targeted specific constructs (JSX, new-
    /// expr) that *had* regressed, leaving structural blind spots. This sweeps a diverse corpus and
    /// asserts, for every snippet, that (a) the formatted output re-parses and (b) re-formatting is
    /// a fixed point — so any future construct that formats non-idempotently is caught here.
    #[test]
    fn format_is_idempotent_over_a_corpus() {
        let corpus = [
            "let a = { x: 1, y: { z: [1, 2, 3] } }\n",
            "let f = (a, b) => a + b\n",
            "let g = (x) => { return x * 2 }\n",
            "const e = <div class=\"x\">{a}<span>b</span></div>\n",
            "let frag = <><a>1</a><b>2</b></>\n",
            "let n = new Foo(1).bar().baz\n",
            "let n2 = new (getCtor())(arg)\n",
            "let t = `a${b}c${d}e`\n",
            "let lit = \"has a literal $ and a ${notInterp}\"\n",
            "if (x) { return 1 } else if (y) { return 2 } else { return 3 }\n",
            "for (let i = 0; i < n; i = i + 1) { f(i) }\n",
            "for (const k of items) { use(k) }\n",
            "while (cond) { step() }\n",
            "do { once() } while (again)\n",
            "switch (x) { case 1: f(); break; default: g() }\n",
            "let u: number | string = 1\n",
            "type T = { a: number, b: string[] }\n",
            "export fn h(x: T): T { return x }\n",
            "let v = a ? b : c\n",
            "let w = a ?? b\n",
            "let arr = [{ k: 1 }, { k: 2 }, ...rest]\n",
            "let obj = { ...base, x: 1, y: 2 }\n",
            "let chain = a?.b?.c\n",
            "let big = 1e400\n",
            "try { risky() } catch (e) { handle(e) } finally { cleanup() }\n",
            "async fn af() { return await thing() }\n",
            "import { a, b as c } from \"./m\"\nexport default a\n",
        ];
        for src in corpus {
            let once = format_source(src).expect("format");
            tishlang_parser::parse(&once)
                .unwrap_or_else(|e| panic!("formatted output must re-parse for {src:?}: {e}\n{once}"));
            let twice = format_source(&once).expect("re-format");
            assert_eq!(once, twice, "non-idempotent for {src:?}:\n{once:?}\n  ->\n{twice:?}");
        }
    }
}
