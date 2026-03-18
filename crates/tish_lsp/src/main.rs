//! Tish Language Server — diagnostics, symbols, completion, format, go-to-definition, workspace symbols.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use regex::Regex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, NumberOrString,
    CompletionParams, CompletionResponse, CompletionTriggerKind, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, InitializeParams, InitializeResult, Location, MessageType, OneOf,
    Position, Range, ServerCapabilities, ServerInfo, SymbolInformation, SymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use walkdir::WalkDir;

#[derive(Debug)]
struct Backend {
    client: Client,
    docs: Arc<RwLock<HashMap<Url, String>>>,
    roots: Arc<RwLock<Vec<PathBuf>>>,
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Arc::new(RwLock::new(HashMap::new())),
        roots: Arc::new(RwLock::new(Vec::new())),
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
    match tish_parser::parse(text) {
        Ok(program) => {
            for d in tish_lint::lint_program(&program) {
                let sev = match d.severity {
                    tish_lint::Severity::Error => DiagnosticSeverity::ERROR,
                    tish_lint::Severity::Warning => DiagnosticSeverity::WARNING,
                };
                diags.push(Diagnostic {
                    range: diag_range(d.line.saturating_sub(1), d.col.saturating_sub(1), text),
                    severity: Some(sev),
                    code: Some(NumberOrString::String(d.code.to_string())),
                    message: d.message,
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
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(tower_lsp::lsp_types::CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
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
        let uri = params
            .text_document_position
            .text_document
            .uri
            .clone();
        let _pos = params.text_document_position.position;
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

        if let Ok(program) = tish_parser::parse(&text) {
            for s in &program.statements {
                match s {
                    tish_ast::Statement::FunDecl { name, .. } => {
                        items.push(CompletionItem {
                            label: name.to_string(),
                            kind: Some(CompletionItemKind::FUNCTION),
                            ..Default::default()
                        });
                    }
                    tish_ast::Statement::VarDecl { name, .. } => {
                        items.push(CompletionItem {
                            label: name.to_string(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
        }

        if let Some(ctx) = params.context {
            if matches!(
                ctx.trigger_kind,
                CompletionTriggerKind::TRIGGER_CHARACTER
            ) && ctx.trigger_character.as_deref() == Some(".")
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
        let Ok(program) = tish_parser::parse(&text) else {
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
        let Ok(program) = tish_parser::parse(&text) else {
            return Ok(None);
        };

        let word = word_at_position(&text, position);
        if word.is_empty() {
            return Ok(None);
        }

        let path = uri.to_file_path().ok();

        for s in &program.statements {
            if let Some(loc) = find_decl_in_stmt(s, &word, &uri, &text) {
                return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
            }
        }

        if let Some(ref base) = path {
            for s in &program.statements {
                if let tish_ast::Statement::Import {
                    specifiers,
                    from,
                    ..
                } = s
                {
                    for sp in specifiers {
                        let (imported, local) = match sp {
                            tish_ast::ImportSpecifier::Named { name, alias } => {
                                (name.as_ref(), alias.as_ref().map(|a| a.as_ref()).unwrap_or(name.as_ref()))
                            }
                            tish_ast::ImportSpecifier::Default(n) => (n.as_ref(), n.as_ref()),
                            _ => continue,
                        };
                        if local != word.as_str() {
                            continue;
                        }
                        let from_s = from.as_ref();
                        if !from_s.starts_with("./") && !from_s.starts_with("../") {
                            continue;
                        }
                        let dir = base.parent().unwrap_or(Path::new(""));
                        let target = dir.join(from_s.trim_start_matches("./"));
                        let target = if target.extension().is_none() {
                            target.with_extension("tish")
                        } else {
                            target
                        };
                        if let Ok(can) = target.canonicalize() {
                            if let Ok(u) = Url::from_file_path(&can) {
                                if let Ok(src) = std::fs::read_to_string(&can) {
                                    if let Ok(prog) = tish_parser::parse(&src) {
                                        if let Some(loc) =
                                            find_export(&prog, imported, &u, &src)
                                        {
                                            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<tower_lsp::lsp_types::TextEdit>>> {
        let uri = params.text_document.uri;
        let text = {
            let g = self.docs.read().unwrap();
            g.get(&uri).cloned()
        };
        let Some(text) = text else {
            return Ok(None);
        };
        match tish_fmt::format_source(&text) {
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
                    .show_message(
                        MessageType::ERROR,
                        format!("tish-fmt (formatter): {}", e),
                    )
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
                let Ok(program) = tish_parser::parse(&src) else {
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
    s: &tish_ast::Statement,
    text: &str,
    uri: &Url,
    query: &str,
    out: &mut Vec<SymbolInformation>,
) {
    match s {
        tish_ast::Statement::FunDecl { name, span, .. } => {
            if name.to_lowercase().contains(query) {
                out.push(SymbolInformation {
                    name: name.to_string(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: span_to_range(span, text),
                    },
                    container_name: None,
                });
            }
        }
        tish_ast::Statement::VarDecl { name, span, .. } => {
            if name.to_lowercase().contains(query) {
                out.push(SymbolInformation {
                    name: name.to_string(),
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: span_to_range(span, text),
                    },
                    container_name: None,
                });
            }
        }
        tish_ast::Statement::Block { statements, .. } => {
            for x in statements {
                collect_workspace_syms(x, text, uri, query, out);
            }
        }
        _ => {}
    }
}

fn find_export(
    program: &tish_ast::Program,
    name: &str,
    uri: &Url,
    text: &str,
) -> Option<Location> {
    for s in &program.statements {
        match s {
            tish_ast::Statement::FunDecl { name: n, span, .. } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(span, text),
                });
            }
            tish_ast::Statement::VarDecl { name: n, span, .. } if n.as_ref() == name => {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(span, text),
                });
            }
            tish_ast::Statement::Export { declaration, .. } => match declaration.as_ref() {
                tish_ast::ExportDeclaration::Named(inner) => {
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
    s: &tish_ast::Statement,
    word: &str,
    uri: &Url,
    text: &str,
) -> Option<Location> {
    match s {
        tish_ast::Statement::FunDecl { name, span, .. } if name.as_ref() == word => Some(Location {
            uri: uri.clone(),
            range: span_to_range(span, text),
        }),
        tish_ast::Statement::VarDecl { name, span, .. } if name.as_ref() == word => Some(Location {
            uri: uri.clone(),
            range: span_to_range(span, text),
        }),
        tish_ast::Statement::Block { statements, .. } => {
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

fn span_to_range(span: &tish_ast::Span, _text: &str) -> Range {
    Range {
        start: pos(span.start.0.saturating_sub(1) as u32, span.start.1.saturating_sub(1) as u32),
        end: pos(span.end.0.saturating_sub(1) as u32, span.end.1.saturating_sub(1) as u32),
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

fn doc_symbol_stmt(
    s: &tish_ast::Statement,
    text: &str,
    out: &mut Vec<tower_lsp::lsp_types::DocumentSymbol>,
) {
    match s {
        tish_ast::Statement::FunDecl {
            name,
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
                selection_range: span_to_range(span, text),
                children: if children.is_empty() {
                    None
                } else {
                    Some(children)
                },
            });
        }
        tish_ast::Statement::VarDecl { name, span, .. } => {
            out.push(tower_lsp::lsp_types::DocumentSymbol {
                name: name.to_string(),
                detail: None,
                kind: tower_lsp::lsp_types::SymbolKind::VARIABLE,
                tags: None,
                deprecated: None,
                range: span_to_range(span, text),
                selection_range: span_to_range(span, text),
                children: None,
            });
        }
        tish_ast::Statement::Block { statements, .. } => {
            for x in statements {
                doc_symbol_stmt(x, text, out);
            }
        }
        _ => {}
    }
}

fn collect_child_syms(
    s: &tish_ast::Statement,
    text: &str,
    out: &mut Vec<tower_lsp::lsp_types::DocumentSymbol>,
) {
    match s {
        tish_ast::Statement::Block { statements, .. } => {
            for x in statements {
                doc_symbol_stmt(x, text, out);
            }
        }
        _ => doc_symbol_stmt(s, text, out),
    }
}
