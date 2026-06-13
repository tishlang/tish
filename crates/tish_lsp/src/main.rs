//! Tish Language Server — diagnostics, symbols, completion, format, go-to-definition, workspace symbols.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use regex::Regex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
    CompletionTriggerKind, Diagnostic, DiagnosticSeverity, DiagnosticTag,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, Location, MarkupContent,
    MarkupKind, MessageType, NumberOrString, OneOf, Position, Range, ReferenceParams,
    RenameOptions, RenameParams, ServerCapabilities, ServerInfo, SymbolInformation, SymbolKind,
    SymbolTag, TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    WorkDoneProgressOptions, WorkspaceEdit, WorkspaceSymbolParams,
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
    /// Monotonic per-document edit counter. did_change bumps it and a debounced task only
    /// publishes diagnostics if its edit is still the latest — so rapid keystrokes coalesce and
    /// superseded recomputes are dropped (the analysis pipeline is comparatively expensive).
    edit_seq: Arc<RwLock<HashMap<Url, u64>>>,
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
        edit_seq: Arc::new(RwLock::new(HashMap::new())),
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

/// End position of a full-document range. Splits on '\n' (not `str::lines`, which drops a trailing
/// newline) so the range reaches *past* the document's final newline; the last segment is counted in
/// UTF-16 code units, as the LSP position encoding requires.
fn full_doc_end(text: &str) -> (u32, u32) {
    let line = text.matches('\n').count() as u32;
    let last_seg = text.rsplit('\n').next().unwrap_or("");
    let col = last_seg.encode_utf16().count() as u32;
    (line, col)
}

fn diag_range(line: u32, col: u32, text: &str) -> Range {
    let line_str = text.lines().nth(line as usize).unwrap_or("");
    let end_char = line_str.len().max(col as usize + 1) as u32;
    Range {
        start: pos(line, col),
        end: pos(line, end_char.min(col + 80)),
    }
}

/// `lsp-types` still requires the `deprecated` field on these structs, but marks it
/// `#[deprecated(note = "Use tags instead")]`. Use `tags` with [`SymbolTag::Deprecated`] when a
/// symbol is actually deprecated; this helper keeps a single `#[allow(deprecated)]` boundary.
#[allow(deprecated)]
fn symbol_information(
    name: String,
    kind: SymbolKind,
    tags: Option<Vec<SymbolTag>>,
    location: Location,
    container_name: Option<String>,
) -> SymbolInformation {
    SymbolInformation {
        name,
        kind,
        tags,
        deprecated: None,
        location,
        container_name,
    }
}

#[allow(deprecated)]
fn document_symbol(
    name: String,
    detail: Option<String>,
    kind: SymbolKind,
    tags: Option<Vec<SymbolTag>>,
    range: Range,
    selection_range: Range,
    children: Option<Vec<DocumentSymbol>>,
) -> DocumentSymbol {
    // LSP spec: selectionRange must be contained in range, or VS Code rejects the whole document
    // outline ("selectionRange must be contained in fullRange"). Some declaration spans are
    // unset/degenerate (e.g. a top-level VarDecl's span defaults to an empty range) and so do not
    // enclose the name span; fall back to the name range to keep the invariant.
    let contains = (range.start.line, range.start.character)
        <= (selection_range.start.line, selection_range.start.character)
        && (selection_range.end.line, selection_range.end.character)
            <= (range.end.line, range.end.character);
    let range = if contains { range } else { selection_range };
    DocumentSymbol {
        name,
        detail,
        kind,
        tags,
        deprecated: None,
        range,
        selection_range,
        children,
    }
}

async fn publish_parse_and_lint(client: &Client, uri: Url, text: &str) {
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
                        format!("`{}` is declared but its value is never read", ub.name),
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
            // Gradual type checker (Phase 2): surface provable annotation violations as warnings.
            for d in tishlang_compile::check_program(&program) {
                diags.push(Diagnostic {
                    range: span_to_range(&d.span, text),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("tish-type".into())),
                    message: d.message,
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
    // MUST be awaited — `publish_diagnostics` is async; a bare `let _ = …` drops the future
    // unsent, which silently disables ALL LSP diagnostics (parse errors, lints, unused bindings).
    client.publish_diagnostics(uri, diags, None).await;
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
        publish_parse_and_lint(&self.client, uri, &text).await;
    }

    async fn did_change(&self, p: DidChangeTextDocumentParams) {
        let uri = p.text_document.uri;
        if let Some(chg) = p.content_changes.into_iter().last() {
            self.docs
                .write()
                .unwrap()
                .insert(uri.clone(), chg.text.clone());
            // Bump this document's edit sequence and debounce the (expensive) analysis: only the
            // task whose sequence is still current after the delay publishes, so a burst of
            // keystrokes coalesces into one recompute instead of one-per-change.
            let seq = {
                let mut g = self.edit_seq.write().unwrap();
                let n = g.entry(uri.clone()).or_insert(0);
                *n += 1;
                *n
            };
            let client = self.client.clone();
            let docs = Arc::clone(&self.docs);
            let edit_seq = Arc::clone(&self.edit_seq);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                // Superseded by a newer edit while we waited — drop this stale recompute.
                if edit_seq.read().unwrap().get(&uri).copied() != Some(seq) {
                    return;
                }
                let text = docs.read().unwrap().get(&uri).cloned();
                if let Some(text) = text {
                    publish_parse_and_lint(&client, uri, &text).await;
                }
            });
        }
    }

    async fn did_close(&self, p: DidCloseTextDocumentParams) {
        self.docs.write().unwrap().remove(&p.text_document.uri);
        self.edit_seq.write().unwrap().remove(&p.text_document.uri);
        self.client
            .publish_diagnostics(p.text_document.uri, vec![], None)
            .await;
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

        let mut syms: Vec<DocumentSymbol> = Vec::new();
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
            // If the use resolves to an import specifier, jump THROUGH the import into the source
            // module (a relative .tish file) instead of to the local import line. Falls back to the
            // specifier span when the source module can't be located.
            if is_import_specifier_span(&program, &def) {
                if let Ok(ref file_path) = uri.to_file_path() {
                    let word = word_at_position(&text, position);
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
                }
            }
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

        // Type reference (`: SomeType`, `extends SomeType`, `as SomeType`) → jump to its
        // `type`/`interface` declaration. Value bindings are resolved above, so this only
        // fires for genuine type names.
        if let Some(sp) = type_decl_span(&program, word.as_str()) {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: span_to_range(&sp, &text),
            })));
        }

        if let Ok(ref file_path) = uri.to_file_path() {
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
            // Not a value name at the cursor — it may be a type reference (`: SomeType`,
            // `extends SomeType`). Type annotations carry no spans, so match by word.
            let word = word_at_position(&text, pos);
            if let Some(ty) = type_alias_body(&program, &word) {
                let value = format!("**`{}`**{}", word, code_hint(&format!("type {} = {}", word, ty)));
                return Ok(Some(Hover {
                    range: None,
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                }));
            }
            return Ok(None);
        };
        let def = tishlang_resolve::definition_span(&program, &text, pos.line, pos.character);
        let mut md = format!("**`{}`**", use_site.name);
        // Type-aware hover: show the declared (or simply-inferred) type / fn signature.
        if let Some(ref dspan) = def {
            if let Some(hint) = type_hint_at_def(&program, dspan) {
                md.push_str(&hint);
            }
        }
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
                    md.push_str(
                        "\n\n_Interpreter root global (no lexical declaration in this file)._",
                    );
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
        // Honor the client's includeDeclaration flag: reference_spans_for_def returns the definition
        // span plus the use spans, so drop the definition when only uses were requested.
        let include_decl = params.context.include_declaration;
        let locs: Vec<Location> = spans
            .into_iter()
            .filter(|sp| include_decl || *sp != def)
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
        // Only offer a rename box for a symbol rename() can actually act on — not a member property
        // or other non-binding token, which would silently no-op (#145).
        match rename_target(&program, &text, pos.line, pos.character) {
            Some((range, placeholder)) => Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                range,
                placeholder,
            })),
            None => Ok(None),
        }
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
        // Type-alias rename: the value resolver can't see `: T` annotation uses (type names live
        // in a separate namespace), so handle a cursor on a type-alias declaration/use here,
        // editing the declaration and every annotation site together.
        if let Some(spans) =
            tishlang_resolve::type_alias_rename_spans(&program, &text, pos.line, pos.character)
        {
            let mut edits: Vec<TextEdit> = spans
                .into_iter()
                .map(|sp| TextEdit {
                    range: span_to_range(&sp, &text),
                    new_text: new_name.clone(),
                })
                .collect();
            edits.sort_by(|a, b| {
                (b.range.start.line, b.range.start.character)
                    .cmp(&(a.range.start.line, a.range.start.character))
            });
            let mut m = HashMap::new();
            m.insert(uri.clone(), edits);
            return Ok(Some(WorkspaceEdit {
                changes: Some(m),
                ..Default::default()
            }));
        }
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
        let mut edits: Vec<TextEdit> = spans
            .into_iter()
            .map(|sp| TextEdit {
                range: span_to_range(&sp, &text),
                new_text: new_name.clone(),
            })
            .collect();
        // Apply from end of document so earlier ranges stay valid when lengths change.
        edits.sort_by(|a, b| {
            (b.range.start.line, b.range.start.character)
                .cmp(&(a.range.start.line, a.range.start.character))
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
                // Replace the WHOLE document. Using a range that stops before the document's final
                // newline appends the formatter's own trailing newline on top of it, adding a blank
                // line on every format (see full_doc_end).
                let (end_line, end_char) = full_doc_end(&text);
                Ok(Some(vec![tower_lsp::lsp_types::TextEdit {
                    range: Range {
                        start: pos(0, 0),
                        end: pos(end_line, end_char),
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
            name, name_span, ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(symbol_information(
                    name.to_string(),
                    SymbolKind::FUNCTION,
                    None,
                    Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    None,
                ));
            }
        }
        tishlang_ast::Statement::VarDecl {
            name, name_span, ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(symbol_information(
                    name.to_string(),
                    SymbolKind::VARIABLE,
                    None,
                    Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    None,
                ));
            }
        }
        tishlang_ast::Statement::TypeAlias {
            name, name_span, ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(symbol_information(
                    name.to_string(),
                    SymbolKind::INTERFACE,
                    None,
                    Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    None,
                ));
            }
        }
        tishlang_ast::Statement::DeclareFun {
            name, name_span, ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(symbol_information(
                    name.to_string(),
                    SymbolKind::FUNCTION,
                    None,
                    Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    None,
                ));
            }
        }
        tishlang_ast::Statement::DeclareVar {
            name, name_span, ..
        } => {
            if name.to_lowercase().contains(query) {
                out.push(symbol_information(
                    name.to_string(),
                    SymbolKind::VARIABLE,
                    None,
                    Location {
                        uri: uri.clone(),
                        range: span_to_range(name_span, text),
                    },
                    None,
                ));
            }
        }
        tishlang_ast::Statement::Export { declaration, .. } => {
            if let tishlang_ast::ExportDeclaration::Named(inner) = declaration.as_ref() {
                collect_workspace_syms(inner, text, uri, query, out);
            }
        }
        tishlang_ast::Statement::Block { statements, .. }
        | tishlang_ast::Statement::Multi { statements, .. } => {
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
                name: n, name_span, ..
            } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(name_span, text),
                });
            }
            tishlang_ast::Statement::VarDecl {
                name: n, name_span, ..
            } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(name_span, text),
                });
            }
            tishlang_ast::Statement::Export { declaration, .. } => if let tishlang_ast::ExportDeclaration::Named(inner) = declaration.as_ref() {
                if let Some(loc) = find_decl_in_stmt(inner, name, uri, text) {
                    return Some(loc);
                }
            },
            _ => {}
        }
    }
    None
}

/// Locate the `export default …` statement in a module (the target of a default import).
pub(crate) fn find_default_export(
    program: &tishlang_ast::Program,
    uri: &Url,
    text: &str,
) -> Option<Location> {
    for s in &program.statements {
        if let tishlang_ast::Statement::Export { declaration, span } = s {
            if matches!(
                declaration.as_ref(),
                tishlang_ast::ExportDeclaration::Default(_)
            ) {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(span, text),
                });
            }
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
            name, name_span, ..
        } if name.as_ref() == word => Some(Location {
            uri: uri.clone(),
            range: span_to_range(name_span, text),
        }),
        tishlang_ast::Statement::VarDecl {
            name, name_span, ..
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

/// The (range, placeholder) a rename should offer, or `None` when the symbol under the cursor isn't
/// renameable — so `prepare_rename` doesn't pop a rename box that `rename()` then silently no-ops
/// (#145, e.g. a cursor on a member property `obj.foo`). Mirrors exactly what `rename()` can act on:
/// a value binding (`definition_span` resolves) or a type alias (`type_alias_rename_spans`).
fn rename_target(
    program: &tishlang_ast::Program,
    text: &str,
    line: u32,
    character: u32,
) -> Option<(Range, String)> {
    let nu = tishlang_resolve::name_at_cursor(program, text, line, character)?;
    let renameable = tishlang_resolve::definition_span(program, text, line, character).is_some()
        || tishlang_resolve::type_alias_rename_spans(program, text, line, character).is_some();
    if !renameable {
        return None;
    }
    Some((span_to_range(&nu.span, text), nu.name.to_string()))
}

/// Whether `span` is the local-name span of an import specifier — i.e. `definition_span` resolved a
/// use to an `import { … }` line. Go-to-definition should follow such a result through to the source
/// module rather than jumping to the import line itself.
fn is_import_specifier_span(program: &tishlang_ast::Program, span: &tishlang_ast::Span) -> bool {
    use tishlang_ast::{ImportSpecifier, Statement};
    program.statements.iter().any(|s| {
        if let Statement::Import { specifiers, .. } = s {
            specifiers.iter().any(|sp| {
                let local = match sp {
                    ImportSpecifier::Named {
                        name_span,
                        alias_span,
                        ..
                    } => alias_span.as_ref().unwrap_or(name_span),
                    ImportSpecifier::Namespace { name_span, .. }
                    | ImportSpecifier::Default { name_span, .. } => name_span,
                };
                local == span
            })
        } else {
            false
        }
    })
}

fn word_at_position(text: &str, position: Position) -> String {
    let line = text.lines().nth(position.line as usize).unwrap_or("");
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    // `position.character` is a UTF-16 code-unit offset (the LSP position encoding), not a char
    // index — map it to one so astral chars (2 UTF-16 units each, e.g. emoji) earlier on the line
    // don't shift the cursor off the intended word (#133).
    let target_u16 = position.character as usize;
    let col = {
        let mut idx = 0usize;
        let mut acc = 0usize;
        for (_, c) in &chars {
            if acc >= target_u16 {
                break;
            }
            acc += c.len_utf16();
            idx += 1;
        }
        idx.min(chars.len())
    };
    // Pick the identifier the cursor is on. If the cursor sits just past a word's end
    // (on whitespace/punct or EOL), fall back to the identifier immediately to its left.
    let mut start = col;
    if start >= chars.len() || !is_ident_char(chars[start].1) {
        if start == 0 || !is_ident_char(chars[start - 1].1) {
            return String::new();
        }
        start -= 1;
    }
    // Scan left to the word start, then right to the word end (the original missed the prefix
    // when the cursor landed in the middle of a word).
    while start > 0 && is_ident_char(chars[start - 1].1) {
        start -= 1;
    }
    let mut end = start;
    while end < chars.len() && is_ident_char(chars[end].1) {
        end += 1;
    }
    let s = chars[start].0;
    let e = chars.get(end).map(|(p, _)| *p).unwrap_or(line.len());
    line[s..e].to_string()
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// ── Type-aware hover ─────────────────────────────────────────────────────────

/// Render a `TypeAnnotation` to a readable, TypeScript-ish string for hover.
fn render_type(t: &tishlang_ast::TypeAnnotation) -> String {
    use tishlang_ast::{TypeAnnotation as T, TypeLiteral as L};
    match t {
        T::Simple(s, _) => s.to_string(),
        T::Array(inner) => {
            // Parenthesize composite element types so `(A | B)[]` reads unambiguously.
            if matches!(
                inner.as_ref(),
                T::Union(_) | T::Intersection(_) | T::Function { .. }
            ) {
                format!("({})[]", render_type(inner))
            } else {
                format!("{}[]", render_type(inner))
            }
        }
        T::Object(fields) => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, render_type(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        T::Function { params, returns } => format!(
            "({}) => {}",
            params.iter().map(render_type).collect::<Vec<_>>().join(", "),
            render_type(returns)
        ),
        T::Union(ts) => ts.iter().map(render_type).collect::<Vec<_>>().join(" | "),
        T::Tuple(ts) => format!(
            "[{}]",
            ts.iter().map(render_type).collect::<Vec<_>>().join(", ")
        ),
        T::Intersection(ts) => ts.iter().map(render_type).collect::<Vec<_>>().join(" & "),
        T::Literal(L::Str(s)) => format!("\"{}\"", s),
        T::Literal(L::Num(n)) => {
            if n.fract() == 0.0 && n.is_finite() {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        T::Literal(L::Bool(b)) => b.to_string(),
    }
}

/// Best-effort type of a simple initializer (literals only). Anything non-trivial returns `None`,
/// so hover omits the type rather than guessing wrong.
fn shallow_expr_type(e: &tishlang_ast::Expr) -> Option<tishlang_ast::TypeAnnotation> {
    use tishlang_ast::{Expr, Literal, TypeAnnotation as T};
    if let Expr::Literal { value, .. } = e {
        let name = match value {
            Literal::Number(_) => "number",
            Literal::String(_) => "string",
            Literal::Bool(_) => "boolean",
            Literal::Null => "null",
        };
        Some(T::Simple(Arc::from(name), tishlang_ast::Span::default()))
    } else {
        None
    }
}

/// Render a function parameter as `name: T` (or just `name` when unannotated).
fn render_param(p: &tishlang_ast::FunParam) -> String {
    use tishlang_ast::FunParam;
    match p {
        FunParam::Simple(tp) => match &tp.type_ann {
            Some(t) => format!("{}: {}", tp.name, render_type(t)),
            None => tp.name.to_string(),
        },
        FunParam::Destructure { type_ann, .. } => match type_ann {
            Some(t) => format!("{{…}}: {}", render_type(t)),
            None => "{…}".to_string(),
        },
    }
}

/// `fn name(params): R` signature line for a function declaration.
fn fn_signature(
    name: &str,
    params: &[tishlang_ast::FunParam],
    rest: &Option<tishlang_ast::TypedParam>,
    ret: &Option<tishlang_ast::TypeAnnotation>,
) -> String {
    let mut ps: Vec<String> = params.iter().map(render_param).collect();
    if let Some(r) = rest {
        let t = r
            .type_ann
            .as_ref()
            .map(|t| format!(": {}", render_type(t)))
            .unwrap_or_default();
        ps.push(format!("...{}{}", r.name, t));
    }
    let ret_s = ret
        .as_ref()
        .map(render_type)
        .unwrap_or_else(|| "void".to_string());
    format!("fn {}({}): {}", name, ps.join(", "), ret_s)
}

/// Definition spans are name spans; match on the start position.
fn same_start(a: &tishlang_ast::Span, b: &tishlang_ast::Span) -> bool {
    a.start == b.start
}

/// Wrap a one-line type hint in a tish code fence for hover.
fn code_hint(line: &str) -> String {
    format!("\n\n```tish\n{}\n```", line)
}

/// Find the declaration whose name is at `def` and produce a hover type line (markdown), if any.
fn type_hint_at_def(program: &tishlang_ast::Program, def: &tishlang_ast::Span) -> Option<String> {
    program.statements.iter().find_map(|s| hint_in_stmt(s, def))
}

/// `name_span` of a `type`/`interface` declaration named `name` (both parse to `TypeAlias`).
/// Used so cmd+click on a `: SomeType` reference jumps to its declaration. Type annotations
/// carry no spans, so this is a name match — sound because value bindings resolve first.
fn type_decl_span(program: &tishlang_ast::Program, name: &str) -> Option<tishlang_ast::Span> {
    program.statements.iter().find_map(|s| match s {
        tishlang_ast::Statement::TypeAlias {
            name: n, name_span, ..
        } if n.as_ref() == name => Some(*name_span),
        _ => None,
    })
}

/// Rendered body of a `type`/`interface` declaration named `name`, for hover.
fn type_alias_body(program: &tishlang_ast::Program, name: &str) -> Option<String> {
    program.statements.iter().find_map(|s| match s {
        tishlang_ast::Statement::TypeAlias { name: n, ty, .. } if n.as_ref() == name => {
            Some(render_type(ty))
        }
        _ => None,
    })
}

fn hint_in_stmt(s: &tishlang_ast::Statement, def: &tishlang_ast::Span) -> Option<String> {
    use tishlang_ast::{FunParam, Statement as St};
    match s {
        St::VarDecl {
            name,
            name_span,
            mutable,
            type_ann,
            init,
            ..
        } => {
            if same_start(name_span, def) {
                let ty = type_ann
                    .clone()
                    .or_else(|| init.as_ref().and_then(shallow_expr_type))?;
                let kw = if *mutable { "let" } else { "const" };
                return Some(code_hint(&format!(
                    "{} {}: {}",
                    kw,
                    name,
                    render_type(&ty)
                )));
            }
            None
        }
        St::FunDecl {
            name,
            name_span,
            params,
            rest_param,
            return_type,
            body,
            ..
        } => {
            if same_start(name_span, def) {
                return Some(code_hint(&fn_signature(
                    name,
                    params,
                    rest_param,
                    return_type,
                )));
            }
            for p in params {
                if let FunParam::Simple(tp) = p {
                    if same_start(&tp.name_span, def) {
                        let ty = tp
                            .type_ann
                            .as_ref()
                            .map(render_type)
                            .unwrap_or_else(|| "any".to_string());
                        return Some(code_hint(&format!("(parameter) {}: {}", tp.name, ty)));
                    }
                }
            }
            if let Some(r) = rest_param {
                if same_start(&r.name_span, def) {
                    let ty = r
                        .type_ann
                        .as_ref()
                        .map(render_type)
                        .unwrap_or_else(|| "any[]".to_string());
                    return Some(code_hint(&format!("(parameter) ...{}: {}", r.name, ty)));
                }
            }
            hint_in_stmt(body, def)
        }
        St::Block { statements, .. } | St::Multi { statements, .. } => {
            statements.iter().find_map(|s| hint_in_stmt(s, def))
        }
        St::If {
            then_branch,
            else_branch,
            ..
        } => hint_in_stmt(then_branch, def)
            .or_else(|| else_branch.as_ref().and_then(|e| hint_in_stmt(e, def))),
        St::For { init, body, .. } => init
            .as_ref()
            .and_then(|i| hint_in_stmt(i, def))
            .or_else(|| hint_in_stmt(body, def)),
        St::While { body, .. } | St::DoWhile { body, .. } | St::ForOf { body, .. } => {
            hint_in_stmt(body, def)
        }
        St::Try { body, .. } => hint_in_stmt(body, def),
        _ => None,
    }
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
            .or_else(|| {
                catch_body
                    .as_ref()
                    .and_then(|b| value_completion_kind_stmt(b, name))
            })
            .or_else(|| {
                finally_body
                    .as_ref()
                    .and_then(|b| value_completion_kind_stmt(b, name))
            }),
        tishlang_ast::Statement::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_e, stmts) in cases {
                if let Some(k) = stmts
                    .iter()
                    .find_map(|st| value_completion_kind_stmt(st, name))
                {
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
            tishlang_ast::ExportDeclaration::Named(inner) => {
                value_completion_kind_stmt(inner, name)
            }
            tishlang_ast::ExportDeclaration::Default(_) => None,
        },
        _ => None,
    }
}

fn doc_symbol_stmt(
    s: &tishlang_ast::Statement,
    text: &str,
    out: &mut Vec<DocumentSymbol>,
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
            out.push(document_symbol(
                name.to_string(),
                None,
                SymbolKind::FUNCTION,
                None,
                span_to_range(span, text),
                span_to_range(name_span, text),
                if children.is_empty() {
                    None
                } else {
                    Some(children)
                },
            ));
        }
        tishlang_ast::Statement::VarDecl {
            name,
            name_span,
            span,
            ..
        } => {
            out.push(document_symbol(
                name.to_string(),
                None,
                SymbolKind::VARIABLE,
                None,
                span_to_range(span, text),
                span_to_range(name_span, text),
                None,
            ));
        }
        tishlang_ast::Statement::TypeAlias {
            name,
            name_span,
            span,
            ..
        } => {
            out.push(document_symbol(
                name.to_string(),
                None,
                SymbolKind::INTERFACE,
                None,
                span_to_range(span, text),
                span_to_range(name_span, text),
                None,
            ));
        }
        tishlang_ast::Statement::DeclareFun {
            name,
            name_span,
            span,
            ..
        } => {
            out.push(document_symbol(
                name.to_string(),
                None,
                SymbolKind::FUNCTION,
                None,
                span_to_range(span, text),
                span_to_range(name_span, text),
                None,
            ));
        }
        tishlang_ast::Statement::DeclareVar {
            name,
            name_span,
            span,
            ..
        } => {
            out.push(document_symbol(
                name.to_string(),
                None,
                SymbolKind::VARIABLE,
                None,
                span_to_range(span, text),
                span_to_range(name_span, text),
                None,
            ));
        }
        // `export fn` / `export let` / `export type` wrap the declaration — descend into it so
        // exported symbols appear in the outline.
        tishlang_ast::Statement::Export { declaration, .. } => {
            if let tishlang_ast::ExportDeclaration::Named(inner) = declaration.as_ref() {
                doc_symbol_stmt(inner, text, out);
            }
        }
        // Block and the transparent comma-declarator group (`let a = 1, b = 2`).
        tishlang_ast::Statement::Block { statements, .. }
        | tishlang_ast::Statement::Multi { statements, .. } => {
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
    out: &mut Vec<DocumentSymbol>,
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

#[cfg(test)]
mod hover_tests {
    use super::*;
    use tishlang_ast::{FunParam, Span, Statement};

    fn parse(src: &str) -> tishlang_ast::Program {
        tishlang_parser::parse(src).expect("parse")
    }

    /// name_span of the first VarDecl/FunDecl named `name`, searched recursively.
    fn decl_span(s: &Statement, name: &str) -> Option<Span> {
        match s {
            Statement::VarDecl { name: n, name_span, .. } if n.as_ref() == name => Some(*name_span),
            Statement::FunDecl { name: n, name_span, body, .. } => {
                if n.as_ref() == name {
                    Some(*name_span)
                } else {
                    decl_span(body, name)
                }
            }
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                statements.iter().find_map(|x| decl_span(x, name))
            }
            Statement::If { then_branch, else_branch, .. } => decl_span(then_branch, name)
                .or_else(|| else_branch.as_ref().and_then(|e| decl_span(e, name))),
            Statement::For { body, .. }
            | Statement::While { body, .. }
            | Statement::DoWhile { body, .. }
            | Statement::ForOf { body, .. } => decl_span(body, name),
            _ => None,
        }
    }

    fn span_of(p: &tishlang_ast::Program, name: &str) -> Span {
        p.statements
            .iter()
            .find_map(|s| decl_span(s, name))
            .unwrap_or_else(|| panic!("decl `{name}` not found"))
    }

    fn param_span(p: &tishlang_ast::Program, fname: &str, pname: &str) -> Span {
        for s in &p.statements {
            if let Statement::FunDecl { name, params, .. } = s {
                if name.as_ref() == fname {
                    for fp in params {
                        if let FunParam::Simple(tp) = fp {
                            if tp.name.as_ref() == pname {
                                return tp.name_span;
                            }
                        }
                    }
                }
            }
        }
        panic!("param `{fname}.{pname}` not found")
    }

    fn hint(p: &tishlang_ast::Program, span: &Span) -> String {
        type_hint_at_def(p, span).expect("expected a type hint")
    }

    #[test]
    fn document_symbols_include_exported_type_and_comma_decls() {
        let src = "export fn foo() {}\ntype Status = number\nlet a = 1, b = 2\ndeclare fn ext(): void\nlet plain = 3\n";
        let program = tishlang_parser::parse(src).unwrap();
        let mut syms = Vec::new();
        for s in &program.statements {
            doc_symbol_stmt(s, src, &mut syms);
        }
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        for expected in ["foo", "Status", "a", "b", "ext", "plain"] {
            assert!(names.contains(&expected), "outline missing `{expected}`: {names:?}");
        }
    }

    #[test]
    fn is_import_specifier_span_detects_imports() {
        let src = "import { foo } from \"./m\"\nfoo()\nlet x = 1\nx\n";
        let program = tishlang_parser::parse(src).unwrap();
        // `foo()` (line 1) resolves to the import specifier → go-to-def should follow it cross-file.
        let foo_def = tishlang_resolve::definition_span(&program, src, 1, 0).expect("foo resolves");
        assert!(
            is_import_specifier_span(&program, &foo_def),
            "foo resolves to an import specifier"
        );
        // `x` (line 3) resolves to the local `let` → not an import, jump to the local def as usual.
        let x_def = tishlang_resolve::definition_span(&program, src, 3, 0).expect("x resolves");
        assert!(
            !is_import_specifier_span(&program, &x_def),
            "x is a local binding, not an import"
        );
    }

    #[test]
    fn find_default_export_locates_export_default() {
        let src = "export fn foo() {}\nexport default 42\n";
        let program = tishlang_parser::parse(src).unwrap();
        let uri = Url::parse("file:///m.tish").unwrap();
        let loc = find_default_export(&program, &uri, src).expect("default export found");
        assert_eq!(loc.range.start.line, 1, "export default is on line 1");
        let none_src = "export fn bar() {}\n";
        let p2 = tishlang_parser::parse(none_src).unwrap();
        assert!(find_default_export(&p2, &uri, none_src).is_none());
    }

    #[test]
    fn annotated_var() {
        let p = parse("let count: number = 0\n");
        assert!(hint(&p, &span_of(&p, "count")).contains("let count: number"));
    }

    #[test]
    fn inferred_var_and_const() {
        let p = parse("let x = 42\nconst label = \"hi\"\nlet ok = true\n");
        assert!(hint(&p, &span_of(&p, "x")).contains("let x: number"));
        assert!(hint(&p, &span_of(&p, "label")).contains("const label: string"));
        assert!(hint(&p, &span_of(&p, "ok")).contains("let ok: boolean"));
    }

    #[test]
    fn function_signature() {
        let p = parse("fn add(a: number, b: number): number { return a + b }\n");
        assert!(hint(&p, &span_of(&p, "add")).contains("fn add(a: number, b: number): number"));
    }

    #[test]
    fn parameter_hover() {
        let p = parse("fn f(p: string) { return p }\n");
        assert!(hint(&p, &param_span(&p, "f", "p")).contains("(parameter) p: string"));
    }

    #[test]
    fn nested_decl_resolves() {
        let p = parse("fn g() {\n  let inner: boolean = true\n  return inner\n}\n");
        assert!(hint(&p, &span_of(&p, "inner")).contains("let inner: boolean"));
    }

    #[test]
    fn composite_types_render() {
        use tishlang_ast::{TypeAnnotation as T, TypeLiteral as L};
        let arr = T::Array(Box::new(T::Simple("number".into(), tishlang_ast::Span::default())));
        assert_eq!(render_type(&arr), "number[]");
        let tup = T::Tuple(vec![T::Simple("number".into(), tishlang_ast::Span::default()), T::Simple("string".into(), tishlang_ast::Span::default())]);
        assert_eq!(render_type(&tup), "[number, string]");
        let uni = T::Union(vec![T::Simple("number".into(), tishlang_ast::Span::default()), T::Simple("null".into(), tishlang_ast::Span::default())]);
        assert_eq!(render_type(&uni), "number | null");
        assert_eq!(render_type(&T::Literal(L::Str("on".into()))), "\"on\"");
        let arr_of_union = T::Array(Box::new(uni));
        assert_eq!(render_type(&arr_of_union), "(number | null)[]");
    }

    #[test]
    fn full_doc_end_reaches_past_trailing_newline_in_utf16() {
        assert_eq!(full_doc_end("a\nb\n"), (2, 0)); // past the final newline (was the blank-line bug)
        assert_eq!(full_doc_end("a\nb"), (1, 1)); // no trailing newline
        assert_eq!(full_doc_end("x\n"), (1, 0));
        assert_eq!(full_doc_end(""), (0, 0));
        assert_eq!(full_doc_end("café"), (0, 4)); // UTF-16 units (é = 1), not bytes (5)
    }

    #[test]
    fn doc_symbols_satisfy_lsp_selection_containment() {
        // LSP requires every DocumentSymbol's selectionRange ⊆ range, or VS Code rejects the
        // whole outline ("selectionRange must be contained in fullRange"). Exercise the
        // declaration forms the outline emits.
        use tower_lsp::lsp_types::DocumentSymbol;
        fn check(syms: &[DocumentSymbol], src: &str) {
            for s in syms {
                let (r, sel) = (&s.range, &s.selection_range);
                let contained = (r.start.line, r.start.character)
                    <= (sel.start.line, sel.start.character)
                    && (sel.end.line, sel.end.character) <= (r.end.line, r.end.character);
                assert!(
                    contained,
                    "selectionRange {sel:?} not contained in range {r:?} for `{}` in:\n{src}",
                    s.name
                );
                if let Some(children) = &s.children {
                    check(children, src);
                }
            }
        }
        let sources = [
            "fn f(x) { return x }\n",
            "let a = 1\n",
            "let a = 1, b = 2\n",
            "export fn g() { return 1 }\n",
            "export let x = 1\n",
            "type T = number\n",
            "declare fn h(): void\n",
            "declare let y: number\n",
            "fn outer() {\n  fn inner() { return 1 }\n  return inner\n}\n",
            "export type Opts = { a: number }\n",
        ];
        for src in sources {
            let p = parse(src);
            let mut syms = Vec::new();
            for s in &p.statements {
                doc_symbol_stmt(s, src, &mut syms);
            }
            check(&syms, src);
        }
    }
}

#[cfg(test)]
mod type_ref_tests {
    use super::*;

    const SRC: &str =
        "interface Point { x: number, y: number }\ntype Status = \"on\" | \"off\"\nlet p: Point = { x: 1, y: 2 }\n";

    #[test]
    fn type_decl_lookup_and_body() {
        let p = tishlang_parser::parse(SRC).expect("parse");
        assert!(type_decl_span(&p, "Point").is_some());
        assert!(type_decl_span(&p, "Status").is_some());
        assert_eq!(type_alias_body(&p, "Point").as_deref(), Some("{ x: number, y: number }"));
        assert_eq!(type_alias_body(&p, "Status").as_deref(), Some("\"on\" | \"off\""));
        assert!(type_decl_span(&p, "Nope").is_none());
    }

    #[test]
    fn word_at_position_finds_whole_word() {
        // Cursor in the MIDDLE of `Point` (line 2, the `o`) must yield the whole word.
        assert_eq!(word_at_position(SRC, Position { line: 2, character: 8 }), "Point");
        // At the word start.
        assert_eq!(word_at_position(SRC, Position { line: 2, character: 7 }), "Point");
        // Just past the end (on the space) falls back to the word on the left.
        assert_eq!(word_at_position(SRC, Position { line: 2, character: 12 }), "Point");
        // On punctuation between words → empty.
        assert_eq!(word_at_position("a = b\n", Position { line: 0, character: 2 }), "");
    }

    #[test]
    fn word_at_position_handles_astral_chars() {
        // #133: three emoji (2 UTF-16 units each) precede `w`; the cursor's UTF-16 character offset
        // (6) must map to the char `w`, not be used directly as a char index (which lands in `foo`).
        assert_eq!(word_at_position("😀😀😀w foo", Position { line: 0, character: 6 }), "w");
    }
}

#[cfg(test)]
mod rename_target_tests {
    use super::*;
    fn parse(src: &str) -> tishlang_ast::Program {
        tishlang_parser::parse(src).expect("parse")
    }

    // #145: a member property is NOT renameable — prepare_rename must not offer a box rename() no-ops.
    #[test]
    fn member_property_not_offered() {
        let src = "let obj = { foo: 1 }\nlet z = obj.foo\n";
        let p = parse(src);
        assert!(
            rename_target(&p, src, 1, 12).is_none(),
            "member property `foo` must not be offered for rename"
        );
    }

    // a value-binding use IS renameable.
    #[test]
    fn value_binding_offered() {
        let src = "let count = 1\nlet z = count\n";
        let p = parse(src);
        let t = rename_target(&p, src, 1, 8); // cursor on the `count` use
        assert!(t.is_some(), "a value binding use must be renameable");
        assert_eq!(t.unwrap().1, "count");
    }

    // a type alias IS renameable (value resolver can't see it, but type_alias_rename_spans can).
    #[test]
    fn type_alias_offered() {
        let src = "type T = number\nfn f(x: T) { return x }\nf(1)\n";
        let p = parse(src);
        assert!(
            rename_target(&p, src, 0, 5).is_some(),
            "a type alias declaration must be renameable"
        );
    }
}
