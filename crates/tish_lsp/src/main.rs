//! Tish Language Server — diagnostics, symbols, completion, format, go-to-definition, workspace symbols.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use regex::Regex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
    CompletionTriggerKind, Diagnostic, DiagnosticSeverity, DiagnosticTag, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
    Hover, HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    Location, MarkupContent, MarkupKind, MessageType, NumberOrString, OneOf, Position, Range,
    ReferenceParams, RenameOptions, RenameParams, ServerCapabilities, ServerInfo,
    WorkDoneProgressOptions,
    SymbolInformation, SymbolKind, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::lsp_types::{PrepareRenameResponse, TextEdit};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use walkdir::WalkDir;

mod builtin_goto;
mod import_goto;

#[derive(Debug)]
struct Backend {
    client: Client,
    docs: Arc<RwLock<HashMap<Url, String>>>,
    roots: Arc<RwLock<Vec<PathBuf>>>,
    /// `(project_root, cargo:spec)` → resolved dependency source root (for `cargo metadata` / registry).
    cargo_src_cache: Arc<RwLock<HashMap<(PathBuf, String), PathBuf>>>,
    /// Root of the `tishlang/tish` checkout (parent of `crates/`), for built-in / JSX goto-definition.
    tishlang_source_root: Arc<RwLock<Option<PathBuf>>>,
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Arc::new(RwLock::new(HashMap::new())),
        roots: Arc::new(RwLock::new(Vec::new())),
        cargo_src_cache: Arc::new(RwLock::new(HashMap::new())),
        tishlang_source_root: Arc::new(RwLock::new(None)),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

fn parse_error_pos(err: &str) -> (u32, u32) {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"start: \((\d+), (\d+)\)").unwrap());
    if let Some(c) = re.captures(err) {
        let line: u32 = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        let col: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        return (line.saturating_sub(1), col.saturating_sub(1));
    }
    (0, 0)
}

fn pos(line: u32, col: u32) -> Position {
    Position {
        line,
        character: col,
    }
}

fn diag_range(line: u32, col: u32, text: &str) -> Range {
    let line_str = text.lines().nth(line as usize).unwrap_or("");
    let end_char = line_str.len().max(col as usize + 1) as u32;
    Range {
        start: pos(line, col),
        end: pos(line, end_char.min(col + 80)),
    }
}

fn publish_parse_and_lint(client: &Client, uri: Url, text: &str) {
    let mut diags = Vec::new();
    match tishlang_parser::parse(text) {
        Ok(program) => {
            for d in tishlang_lint::lint_program(&program) {
                let sev = match d.severity {
                    tishlang_lint::Severity::Error => DiagnosticSeverity::ERROR,
                    tishlang_lint::Severity::Warning => DiagnosticSeverity::WARNING,
                };
                diags.push(Diagnostic {
                    range: diag_range(d.line.saturating_sub(1), d.col.saturating_sub(1), text),
                    severity: Some(sev),
                    code: Some(NumberOrString::String(d.code.to_string())),
                    message: d.message,
                    ..Default::default()
                });
            }
            for u in tishlang_resolve::collect_unresolved_identifiers(&program) {
                diags.push(Diagnostic {
                    range: span_to_range(&u.span, text),
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("tish-unresolved-name".into())),
                    message: format!("no binding in scope for `{}`", u.name),
                    ..Default::default()
                });
            }
            for ub in tishlang_resolve::collect_unused_bindings(&program, text) {
                let (message, code) = match ub.kind {
                    tishlang_resolve::UnusedBindingKind::Import => (
                        format!("`{}` is imported but never used", ub.name),
                        "tish-unused-import",
                    ),
                    tishlang_resolve::UnusedBindingKind::Parameter => (
                        format!("`{}` is declared but never read", ub.name),
                        "tish-unused-parameter",
                    ),
                    tishlang_resolve::UnusedBindingKind::Variable => (
                        format!(
                            "`{}` is declared but its value is never read",
                            ub.name
                        ),
                        "tish-unused-variable",
                    ),
                };
                diags.push(Diagnostic {
                    range: span_to_range(&ub.span, text),
                    severity: Some(DiagnosticSeverity::HINT),
                    code: Some(NumberOrString::String(code.into())),
                    message,
                    tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                    source: Some("tish".into()),
                    ..Default::default()
                });
            }
        }
        Err(e) => {
            let (l, c) = parse_error_pos(&e);
            diags.push(Diagnostic {
                range: diag_range(l, c, text),
                severity: Some(DiagnosticSeverity::ERROR),
                message: e,
                ..Default::default()
            });
        }
    }
    let _ = client.publish_diagnostics(uri, diags, None);
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut roots = self.roots.write().unwrap();
        roots.clear();
        if let Some(folders) = params.workspace_folders {
            for f in folders {
                if let Ok(p) = f.uri.to_file_path() {
                    roots.push(p);
                }
            }
        } else if let Some(uri) = params.root_uri {
            if let Ok(p) = uri.to_file_path() {
                roots.push(p);
            }
        }

        let mut src_root: Option<PathBuf> = None;
        if let Some(opts) = &params.initialization_options {
            if let Some(s) = opts
                .get("tishlangSourceRoot")
                .and_then(|v| v.as_str())
                .map(str::trim)
            {
                if !s.is_empty() {
                    src_root = Some(PathBuf::from(s));
                }
            }
        }
        if src_root.is_none() {
            if let Ok(s) = std::env::var("TISHLANG_SOURCE_ROOT") {
                let t = s.trim();
                if !t.is_empty() {
                    src_root = Some(PathBuf::from(t));
                }
            }
        }
        let mut g = self.tishlang_source_root.write().unwrap();
        *g = src_root.filter(|p| p.is_dir());

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(tower_lsp::lsp_types::CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "tish-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: tower_lsp::lsp_types::InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "tish-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, p: DidOpenTextDocumentParams) {
        let uri = p.text_document.uri;
        let text = p.text_document.text;
        self.docs.write().unwrap().insert(uri.clone(), text.clone());
        publish_parse_and_lint(&self.client, uri, &text);
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        let uri = p.text_document.uri;
        if let Some(chg) = p.content_changes.into_iter().last() {
            self.docs
                .write()
                .unwrap()
                .insert(uri.clone(), chg.text.clone());
            publish_parse_and_lint(&self.client, uri, &chg.text);
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        self.docs.write().unwrap().remove(&p.text_document.uri);
        let _ = self
            .client
            .publish_diagnostics(p.text_document.uri, vec![], None);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let pos = params.text_document_position.position;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let keywords = [
            "fn", "async", "let", "const", "if", "else", "while", "for", "return", "break",
            "continue", "switch", "case", "default", "try", "catch", "finally", "throw", "import",
            "export", "from", "typeof", "void", "await", "of", "in", "true", "false", "null",
            "function", "do",
        ];
        let mut items: Vec<CompletionItem> = keywords
            .iter()
            .map(|k| CompletionItem {
                label: (*k).to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            })
            .collect();

        if let Ok(program) = tishlang_parser::parse(&text) {
            for name in tishlang_resolve::completion_value_names_at_cursor(
                &program,
                &text,
                pos.line,
                pos.character,
            ) {
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(value_completion_kind(&program, name.as_ref())),
                    ..Default::default()
                });
            }
        }

        if let Some(ctx) = params.context {
            if matches!(ctx.trigger_kind, CompletionTriggerKind::TRIGGER_CHARACTER)
                && ctx.trigger_character.as_deref() == Some(".")
            {
                // After dot: could add member completion later
            }
        }

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };

        let mut syms: Vec<tower_lsp::lsp_types::DocumentSymbol> = Vec::new();
        for s in &program.statements {
            doc_symbol_stmt(s, &text, &mut syms);
        }
        Ok(Some(DocumentSymbolResponse::Nested(syms)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let TextDocumentPositionParams {
            text_document,
            position,
        } = params.text_document_position_params;
        let uri = text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };

        if let Some(def) =
            tishlang_resolve::definition_span(&program, &text, position.line, position.character)
        {
            let range = span_to_range(&def, &text);
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range,
            })));
        }

        let word = word_at_position(&text, position);
        if word.is_empty() {
            return Ok(None);
        }

        if let Some(ref file_path) = uri.to_file_path().ok() {
            let roots = self.roots.read().unwrap().clone();
            if let Some(loc) = import_goto::definition_for_import(
                &program,
                file_path,
                word.as_str(),
                &roots,
                self.cargo_src_cache.as_ref(),
            ) {
                return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
            }
            if let Some(loc) = import_goto::definition_for_native_receiver_member(
                &program,
                file_path,
                &text,
                &roots,
                self.cargo_src_cache.as_ref(),
                position.line,
                position.character,
                word.as_str(),
            ) {
                return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
            }
        }

        if let Some(root) = self.tishlang_source_root.read().unwrap().clone() {
            if let Some(bdef) = builtin_goto::definition_for_builtin(
                &text,
                position.line,
                position.character,
                word.as_str(),
            ) {
                if let Some(loc) = builtin_goto::to_file_location(&root, &bdef) {
                    return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
                }
            }
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let pos = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };
        let Some(use_site) =
            tishlang_resolve::name_at_cursor(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let def = tishlang_resolve::definition_span(&program, &text, pos.line, pos.character);
        let mut md = format!("**`{}`**", use_site.name);
        match def {
            Some(def) if def.start == use_site.span.start && def.end == use_site.span.end => {
                md.push_str("\n\n_(binding site)_");
            }
            Some(def) => {
                md.push_str(&format!(
                    "\n\nDefined at line {} col {}",
                    def.start.0, def.start.1
                ));
            }
            None => {
                if tishlang_resolve::is_runtime_global_ident(use_site.name.as_ref()) {
                    md.push_str("\n\n_Interpreter root global (no lexical declaration in this file)._");
                    let word = word_at_position(&text, pos);
                    if !word.is_empty() {
                        if let Some(root) = self.tishlang_source_root.read().unwrap().clone() {
                            if let Some(bdef) = builtin_goto::definition_for_builtin(
                                &text,
                                pos.line,
                                pos.character,
                                word.as_str(),
                            ) {
                                if let Some(loc) = builtin_goto::to_file_location(&root, &bdef) {
                                    // VS Code treats `#L<1-based-line>` on file URLs like "go to line".
                                    let line_1 = bdef.line.saturating_add(1);
                                    let href = loc.uri.as_str();
                                    md.push_str(&format!(
                                        "\n\n[Open in Tish sources]({href}#L{line_1}) (`{}`)",
                                        bdef.rel_path
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    let word = word_at_position(&text, pos);
                    if word.is_empty() {
                        md.push_str("\n\n_No binding in scope for this name._");
                    } else if let Ok(fp) = uri.to_file_path() {
                        let roots = self.roots.read().unwrap().clone();
                        if let Some(nmd) = import_goto::native_member_definition(
                            &program,
                            &fp,
                            &text,
                            &roots,
                            self.cargo_src_cache.as_ref(),
                            pos.line,
                            pos.character,
                            word.as_str(),
                        ) {
                            md.push_str(
                                "\n\n_Native host module member (e.g. `tish:macos`); implementation in Rust._",
                            );
                            if let Some(ref d) = nmd.doc {
                                md.push_str("\n\n");
                                md.push_str(d);
                            }
                            let loc = nmd.location;
                            let line_1 = loc.range.start.line.saturating_add(1);
                            let href = loc.uri.as_str();
                            md.push_str(&format!(
                                "\n\n[Open Rust implementation]({href}#L{line_1})"
                            ));
                        } else {
                            md.push_str("\n\n_No binding in scope for this name._");
                        }
                    } else {
                        md.push_str("\n\n_No binding in scope for this name._");
                    }
                }
            }
        }
        Ok(Some(Hover {
            range: Some(span_to_range(&use_site.span, &text)),
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
        }))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let pos = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };
        let Some(def) = tishlang_resolve::definition_span(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let Some(nu) = tishlang_resolve::name_at_cursor(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let spans =
            tishlang_resolve::reference_spans_for_def(&program, &text, nu.name.as_ref(), def);
        let locs: Vec<Location> = spans
            .into_iter()
            .map(|sp| Location {
                uri: uri.clone(),
                range: span_to_range(&sp, &text),
            })
            .collect();
        Ok(Some(locs))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let pos = params.position;
        let uri = params.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };
        let Some(nu) = tishlang_resolve::name_at_cursor(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let range = span_to_range(&nu.span, &text);
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: nu.name.to_string(),
        }))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let pos = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        let new_name = params.new_name;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(program) = tishlang_parser::parse(&text) else {
            return Ok(None);
        };
        let Some(def) = tishlang_resolve::definition_span(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let Some(nu) = tishlang_resolve::name_at_cursor(&program, &text, pos.line, pos.character)
        else {
            return Ok(None);
        };
        let spans = tishlang_resolve::reference_spans_for_def(
            &program,
            &text,
            nu.name.as_ref(),
            def,
        );
        let mut edits: Vec<TextEdit> = spans
            .into_iter()
            .map(|sp| TextEdit {
                range: span_to_range(&sp, &text),
                new_text: new_name.clone(),
            })
            .collect();
        // Apply from end of document so earlier ranges stay valid when lengths change.
        edits.sort_by(|a, b| {
            (b.range.start.line, b.range.start.character).cmp(&(
                a.range.start.line,
                a.range.start.character,
            ))
        });
        let mut m = HashMap::new();
        m.insert(uri, edits);
        Ok(Some(WorkspaceEdit {
            changes: Some(m),
            ..Default::default()
        }))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<tower_lsp::lsp_types::TextEdit>>> {
        let uri = params.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        match tishlang_fmt::format_source(&text) {
            Ok(formatted) => {
                let lines = text.lines().count() as u32;
                let last_line = text.lines().last().map(|l| l.len() as u32).unwrap_or(0);
                Ok(Some(vec![tower_lsp::lsp_types::TextEdit {
                    range: Range {
                        start: pos(0, 0),
                        end: pos(lines.saturating_sub(1), last_line),
                    },
                    new_text: formatted,
                }]))
            }
            Err(e) => {
                self.client
                    .show_message(MessageType::ERROR, format!("tish-fmt (formatter): {}", e))
                    .await;
                Ok(None)
            }
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();
        if query.is_empty() {
            return Ok(Some(vec![]));
        }
        let roots = self.roots.read().unwrap().clone();
        let mut out = Vec::new();

        for root in roots {
            for e in WalkDir::new(&root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "tish").unwrap_or(false))
            {
                let path = e.path();
                let Ok(src) = std::fs::read_to_string(path) else {
                    continue;
                };
                let Ok(program) = tishlang_parser::parse(&src) else {
                    continue;
                };
                let Ok(uri) = Url::from_file_path(path) else {
                    continue;
                };
                for s in &program.statements {
                    collect_workspace_syms(s, &src, &uri, &query, &mut out);
                }
            }
        }
        Ok(Some(out))
    }
}

fn collect_workspace_syms(
    s: &tishlang_ast::Statement,
    text: &str,
    uri: &Url,
    query: &str,
    out: &mut Vec<SymbolInformation>,
) {
    match s {
        tishlang_ast::Statement::FunDecl {
            name,
            name_span,
            ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(SymbolInformation {
                    name: name.to_string(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    container_name: None,
                });
            }
        }
        tishlang_ast::Statement::VarDecl {
            name,
            name_span,
            ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(SymbolInformation {
                    name: name.to_string(),
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    container_name: None,
                });
            }
        }
        tishlang_ast::Statement::Block { statements, .. } => {
            for x in statements {
                collect_workspace_syms(x, text, uri, query, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn find_export(
    program: &tishlang_ast::Program,
    name: &str,
    uri: &Url,
    text: &str,
) -> Option<Location> {
    for s in &program.statements {
        match s {
            tishlang_ast::Statement::FunDecl {
                name: n,
                name_span,
                ..
            } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(name_span, text),
                });
            }
            tishlang_ast::Statement::VarDecl {
                name: n,
                name_span,
                ..
            } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(name_span, text),
                });
            }
            tishlang_ast::Statement::Export { declaration, .. } => match declaration.as_ref() {
                tishlang_ast::ExportDeclaration::Named(inner) => {
                    if let Some(loc) = find_decl_in_stmt(inner, name, uri, text) {
                        return Some(loc);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    None
}

fn find_decl_in_stmt(
    s: &tishlang_ast::Statement,
    word: &str,
    uri: &Url,
    text: &str,
) -> Option<Location> {
    match s {
        tishlang_ast::Statement::FunDecl {
            name,
            name_span,
            ..
        } if name.as_ref() == word => Some(Location {
            uri: uri.clone(),
            range: span_to_range(name_span, text),
        }),
        tishlang_ast::Statement::VarDecl {
            name,
            name_span,
            ..
        } if name.as_ref() == word => Some(Location {
            uri: uri.clone(),
            range: span_to_range(name_span, text),
        }),
        tishlang_ast::Statement::Block { statements, .. } => {
            for x in statements {
                if let Some(l) = find_decl_in_stmt(x, word, uri, text) {
                    return Some(l);
                }
            }
            None
        }
        _ => None,
    }
}

fn span_to_range(span: &tishlang_ast::Span, text: &str) -> Range {
    if let Some(((sl, sc), (el, ec))) = tishlang_resolve::span_to_lsp_range_exclusive(text, span) {
        Range {
            start: pos(sl, sc),
            end: pos(el, ec),
        }
    } else {
        Range {
            start: pos(
                span.start.0.saturating_sub(1) as u32,
                span.start.1.saturating_sub(1) as u32,
            ),
            end: pos(
                span.end.0.saturating_sub(1) as u32,
                span.end.1.saturating_sub(1) as u32,
            ),
        }
    }
}

fn word_at_position(text: &str, position: Position) -> String {
    let line = text.lines().nth(position.line as usize).unwrap_or("");
    let col = position.character as usize;
    let bytes: Vec<(usize, char)> = line.char_indices().collect();
    let mut start = col.min(bytes.len().saturating_sub(1));
    while start > 0 && !is_ident_char(bytes.get(start).map(|(_, c)| *c).unwrap_or(' ')) {
        start = start.saturating_sub(1);
    }
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i].1) {
        i += 1;
    }
    if start < bytes.len() {
        line[bytes[start].0..bytes.get(i).map(|(p, _)| *p).unwrap_or(line.len())].to_string()
    } else {
        String::new()
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn value_completion_kind(program: &tishlang_ast::Program, name: &str) -> CompletionItemKind {
    for s in &program.statements {
        if let Some(k) = value_completion_kind_stmt(s, name) {
            return k;
        }
    }
    CompletionItemKind::VARIABLE
}

fn value_completion_kind_stmt(
    s: &tishlang_ast::Statement,
    name: &str,
) -> Option<CompletionItemKind> {
    match s {
        tishlang_ast::Statement::FunDecl { name: n, .. } if n.as_ref() == name => {
            Some(CompletionItemKind::FUNCTION)
        }
        tishlang_ast::Statement::VarDecl { name: n, .. } if n.as_ref() == name => {
            Some(CompletionItemKind::VARIABLE)
        }
        tishlang_ast::Statement::Import { specifiers, .. } => {
            for sp in specifiers {
                let local = match sp {
                    tishlang_ast::ImportSpecifier::Named { name: n, alias, .. } => {
                        alias.as_ref().map(|a| a.as_ref()).unwrap_or(n.as_ref())
                    }
                    tishlang_ast::ImportSpecifier::Default { name: n, .. } => n.as_ref(),
                    tishlang_ast::ImportSpecifier::Namespace { name: n, .. } => n.as_ref(),
                };
                if local == name {
                    return Some(CompletionItemKind::VARIABLE);
                }
            }
            None
        }
        tishlang_ast::Statement::Block { statements, .. } => statements
            .iter()
            .find_map(|x| value_completion_kind_stmt(x, name)),
        tishlang_ast::Statement::If {
            then_branch,
            else_branch,
            ..
        } => value_completion_kind_stmt(then_branch, name).or_else(|| {
            else_branch
                .as_ref()
                .and_then(|b| value_completion_kind_stmt(b, name))
        }),
        tishlang_ast::Statement::While { body, .. }
        | tishlang_ast::Statement::ForOf { body, .. }
        | tishlang_ast::Statement::DoWhile { body, .. } => value_completion_kind_stmt(body, name),
        tishlang_ast::Statement::For { init, body, .. } => init
            .as_ref()
            .and_then(|i| value_completion_kind_stmt(i, name))
            .or_else(|| value_completion_kind_stmt(body, name)),
        tishlang_ast::Statement::Try {
            body,
            catch_body,
            finally_body,
            ..
        } => value_completion_kind_stmt(body, name)
            .or_else(|| catch_body.as_ref().and_then(|b| value_completion_kind_stmt(b, name)))
            .or_else(|| finally_body.as_ref().and_then(|b| value_completion_kind_stmt(b, name))),
        tishlang_ast::Statement::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_e, stmts) in cases {
                if let Some(k) = stmts.iter().find_map(|st| value_completion_kind_stmt(st, name)) {
                    return Some(k);
                }
            }
            default_body.as_ref().and_then(|stmts| {
                stmts
                    .iter()
                    .find_map(|st| value_completion_kind_stmt(st, name))
            })
        }
        tishlang_ast::Statement::Export { declaration, .. } => match declaration.as_ref() {
            tishlang_ast::ExportDeclaration::Named(inner) => value_completion_kind_stmt(inner, name),
            tishlang_ast::ExportDeclaration::Default(_) => None,
        },
        _ => None,
    }
}

fn doc_symbol_stmt(
    s: &tishlang_ast::Statement,
    text: &str,
    out: &mut Vec<tower_lsp::lsp_types::DocumentSymbol>,
) {
    match s {
        tishlang_ast::Statement::FunDecl {
            name,
            name_span,
            span,
            body,
            ..
        } => {
            let mut children = Vec::new();
            collect_child_syms(body, text, &mut children);
            out.push(tower_lsp::lsp_types::DocumentSymbol {
                name: name.to_string(),
                detail: None,
                kind: tower_lsp::lsp_types::SymbolKind::FUNCTION,
                tags: None,
                deprecated: None,
                range: span_to_range(span, text),
                selection_range: span_to_range(name_span, text),
                children: if children.is_empty() {
                    None
                } else {
                    Some(children)
                },
            });
        }
        tishlang_ast::Statement::VarDecl {
            name,
            name_span,
            span,
            ..
        } => {
            out.push(tower_lsp::lsp_types::DocumentSymbol {
                name: name.to_string(),
                detail: None,
                kind: tower_lsp::lsp_types::SymbolKind::VARIABLE,
                tags: None,
                deprecated: None,
                range: span_to_range(span, text),
                selection_range: span_to_range(name_span, text),
                children: None,
            });
        }
        tishlang_ast::Statement::Block { statements, .. } => {
            for x in statements {
                doc_symbol_stmt(x, text, out);
            }
        }
        _ => {}
    }
}

fn collect_child_syms(
    s: &tishlang_ast::Statement,
    text: &str,
    out: &mut Vec<tower_lsp::lsp_types::DocumentSymbol>,
) {
    match s {
        tishlang_ast::Statement::Block { statements, .. } => {
            for x in statements {
                doc_symbol_stmt(x, text, out);
            }
        }
        _ => doc_symbol_stmt(s, text, out),
    }
}
