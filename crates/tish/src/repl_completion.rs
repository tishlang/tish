//! REPL tab completion: dotted (e.g. `a.` -> properties/methods) and bare-word (e.g. `con` -> console).
//! Grey preview hint below the line (like Node) and Tab for full list.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::Context;
use rustyline::Helper;

use tishlang_bytecode::{compile_for_repl, compile_for_repl_unoptimized};
use tishlang_parser;
use tishlang_vm::Vm;

/// Tish keywords for bare-word completion (Python-style).
const KEYWORDS: &[&str] = &[
    "async", "await", "break", "case", "catch", "const", "continue", "default", "do", "else",
    "export", "false", "finally", "for", "fn", "function", "if", "import", "in", "let", "null",
    "of", "return", "switch", "throw", "true", "try", "typeof", "void", "while",
];

/// ANSI dim (grey) for hint preview; reset after.
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_RESET: &str = "\x1b[0m";

/// Tab completer that evaluates the expression before the last `.` and suggests property/method names.
pub struct ReplCompleter {
    pub vm: Rc<RefCell<Vm>>,
    pub no_optimize: bool,
}

impl ReplCompleter {
    /// Find the start of the identifier (or keyword) being typed before the cursor.
    /// Returns (byte_start, prefix_string).
    fn ident_prefix_at_cursor<'a>(&self, line_before_cursor: &'a str) -> (usize, &'a str) {
        let s = line_before_cursor;
        if s.is_empty() {
            return (0, "");
        }
        let mut start = s.len();
        for (i, c) in s.char_indices().rev() {
            if c.is_alphanumeric() || c == '_' {
                start = i;
            } else {
                break;
            }
        }
        (start, &s[start..])
    }

    /// Bare-word completions: globals + keywords that start with prefix (Python-style).
    /// When nothing is typed (empty prefix), return no completions so we don't show a hint at the prompt.
    fn get_bare_completions(&self, line_before_cursor: &str) -> (usize, Vec<String>) {
        let (start, prefix) = self.ident_prefix_at_cursor(line_before_cursor);
        if prefix.is_empty() {
            return (start, vec![]);
        }
        let mut names: Vec<String> = self.vm.borrow().global_names();
        names.extend(KEYWORDS.iter().map(|s| (*s).to_string()));
        names.sort();
        names.dedup();
        let filtered: Vec<String> = names
            .into_iter()
            .filter(|k| k.starts_with(prefix))
            .collect();
        (start, filtered)
    }

    /// Get completions for dotted expr (e.g. `a.` -> member names). Returns (start_offset, list).
    fn get_dotted_completions(&self, line_before_cursor: &str) -> Option<(usize, Vec<String>)> {
        let last_dot = line_before_cursor.rfind('.')?;
        let prefix_expr = line_before_cursor[..last_dot].trim();
        if prefix_expr.is_empty() {
            return None;
        }
        let member_prefix = line_before_cursor[last_dot + 1..].trim();

        let program = tishlang_parser::parse(prefix_expr).ok()?;
        let compile_fn = if self.no_optimize {
            compile_for_repl_unoptimized
        } else {
            compile_for_repl
        };
        let chunk = compile_fn(&program).ok()?;
        let value = self.vm.borrow_mut().run(&chunk).ok()?;

        let keys = value.completion_keys();
        let filtered: Vec<String> = keys
            .into_iter()
            .filter(|k| k.starts_with(member_prefix))
            .collect();
        Some((last_dot + 1, filtered))
    }

    /// Unified: dotted if we have a dot and it works, else bare-word.
    fn get_completions(&self, line_before_cursor: &str) -> (usize, Vec<String>) {
        if line_before_cursor.contains('.') {
            if let Some((start, filtered)) = self.get_dotted_completions(line_before_cursor) {
                if !filtered.is_empty() || line_before_cursor.trim_end().ends_with('.') {
                    return (start, filtered);
                }
            }
        }
        self.get_bare_completions(line_before_cursor)
    }

    /// Longest common prefix of a list of strings.
    fn longest_common_prefix(items: &[String]) -> Option<String> {
        let first = items.first()?;
        let mut len = first.len();
        for item in items.iter().skip(1) {
            len = first
                .bytes()
                .zip(item.bytes())
                .take_while(|(a, b)| a == b)
                .count()
                .min(len);
        }
        if len == 0 {
            None
        } else {
            Some(first[..len].to_string())
        }
    }
}

impl Completer for ReplCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let line_before_cursor = &line[..pos];
        let (start, filtered) = self.get_completions(line_before_cursor);
        let pairs: Vec<Pair> = filtered
            .into_iter()
            .map(|k| Pair {
                display: k.clone(),
                replacement: k,
            })
            .collect();
        Ok((start, pairs))
    }
}

impl Hinter for ReplCompleter {
    type Hint = String;

    /// Grey preview: show first completion or common prefix below the line (Node-style).
    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        let line_before_cursor = &line[..pos];
        let (start, filtered) = self.get_completions(line_before_cursor);
        if filtered.is_empty() {
            return None;
        }
        let member_prefix = line_before_cursor.get(start..).unwrap_or("").trim();
        // Hint = text to show after cursor (grey). Single match: full name; multiple: common prefix, or first suggestion.
        let hint = if filtered.len() == 1 {
            filtered[0].clone()
        } else if let Some(lcp) = Self::longest_common_prefix(&filtered) {
            if lcp.len() > member_prefix.len() {
                lcp
            } else {
                // No useful common prefix (e.g. "a." -> many methods); show first as preview like Node.
                filtered[0].clone()
            }
        } else {
            filtered[0].clone()
        };
        // Only show the part not yet typed.
        if hint.starts_with(member_prefix) && hint.len() > member_prefix.len() {
            Some(hint[member_prefix.len()..].to_string())
        } else if hint == member_prefix {
            None
        } else {
            Some(hint)
        }
    }
}

impl Highlighter for ReplCompleter {
    /// Show hint in dim grey (Node-style preview).
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if hint.is_empty() {
            return Cow::Borrowed(hint);
        }
        Cow::Owned(format!("{ANSI_DIM}{hint}{ANSI_RESET}"))
    }
}

impl Validator for ReplCompleter {}

impl Helper for ReplCompleter {}
