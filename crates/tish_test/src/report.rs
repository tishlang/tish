//! Console / dots / JUnit reporters for `tish test`.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStatus {
    Pass,
    Fail,
    Skip,
    Todo,
    /// Excluded by name/tag/`--only` filters — not printed or counted.
    Filtered,
}

#[derive(Clone, Debug)]
pub struct TestResultRecord {
    pub full_name: String,
    pub file: String,
    pub status: TestStatus,
    pub duration: Duration,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RunSummary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub todo: usize,
    pub results: Vec<TestResultRecord>,
}

impl RunSummary {
    pub fn record(&mut self, r: TestResultRecord) {
        match r.status {
            TestStatus::Pass => self.passed += 1,
            TestStatus::Fail => self.failed += 1,
            TestStatus::Skip => self.skipped += 1,
            TestStatus::Todo => self.todo += 1,
            TestStatus::Filtered => {
                // Omitted from counts and reporters.
                return;
            }
        }
        self.results.push(r);
    }

    pub fn total(&self) -> usize {
        self.passed + self.failed + self.skipped + self.todo
    }

    pub fn success(&self) -> bool {
        self.failed == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReporterKind {
    Console,
    Dots,
}

pub struct ConsoleReporter {
    kind: ReporterKind,
    out: io::Stdout,
    /// File header already printed for the current file, so console output groups by file.
    current_file: Option<String>,
}

impl ConsoleReporter {
    pub fn new(kind: ReporterKind) -> Self {
        Self {
            kind,
            out: io::stdout(),
            current_file: None,
        }
    }

    pub fn on_result(&mut self, r: &TestResultRecord) {
        match self.kind {
            ReporterKind::Dots => {
                let ch = match r.status {
                    TestStatus::Pass => '.',
                    TestStatus::Fail => 'F',
                    TestStatus::Skip => 's',
                    TestStatus::Todo => 't',
                    TestStatus::Filtered => return,
                };
                let _ = write!(self.out, "{}", ch);
                let _ = self.out.flush();
            }
            ReporterKind::Console => {
                let label = match r.status {
                    TestStatus::Pass => "PASS",
                    TestStatus::Fail => "FAIL",
                    TestStatus::Skip => "SKIP",
                    TestStatus::Todo => "TODO",
                    TestStatus::Filtered => return,
                };
                // Compare the *displayed* path: a collection error carries the path as given on
                // the command line, while test records carry the canonicalized one.
                let short = short_path(&r.file);
                if !short.is_empty() && self.current_file.as_deref() != Some(short.as_str()) {
                    let _ = writeln!(self.out, "\n{short}");
                    self.current_file = Some(short);
                }
                let ms = r.duration.as_secs_f64() * 1000.0;
                let _ = writeln!(self.out, "  {} {} ({:.1}ms)", label, r.full_name, ms);
                if let Some(msg) = &r.message {
                    for line in format_failure(msg).lines() {
                        let _ = writeln!(self.out, "    {}", line);
                    }
                }
            }
        }
    }

    pub fn finish(&mut self, summary: &RunSummary) {
        if self.kind == ReporterKind::Dots {
            let _ = writeln!(self.out);
        }
        // Every reporter replays failures at the end. `dots` previously printed a bare `F`
        // and no diagnostics at all.
        let failures: Vec<&TestResultRecord> = summary
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Fail)
            .collect();
        if !failures.is_empty() {
            let _ = writeln!(self.out, "\nFailures:\n");
            for (i, r) in failures.iter().enumerate() {
                let _ = writeln!(self.out, "  {}) {}", i + 1, r.full_name);
                if !r.file.is_empty() {
                    let _ = writeln!(self.out, "     {}", short_path(&r.file));
                }
                if let Some(msg) = &r.message {
                    for line in format_failure(msg).lines() {
                        let _ = writeln!(self.out, "     {}", line);
                    }
                }
                let _ = writeln!(self.out);
            }
        }
        let _ = writeln!(
            self.out,
            "Tests: {} passed, {} failed, {} skipped, {} todo ({})",
            summary.passed,
            summary.failed,
            summary.skipped,
            summary.todo,
            summary.total()
        );
    }
}

/// Render a thrown value's display string readably.
///
/// `Value::to_display_string` on an `AssertionError` yields the whole object literal
/// (`{code: …, message: expect(…)…, actual: …}`) with the useful part buried mid-line. Pull the
/// message out and lead with it.
fn format_failure(msg: &str) -> String {
    let trimmed = msg.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return msg.to_string();
    }
    let Some(message) = extract_field(trimmed, "message") else {
        return msg.to_string();
    };
    let name = extract_field(trimmed, "name").unwrap_or_else(|| "Error".to_string());
    let mut out = format!("{name}: {message}");
    // Keep the structured comparison fields, one per line, after the message.
    for field in ["operator", "expected", "actual"] {
        if let Some(v) = extract_field(trimmed, field) {
            if !v.contains('\n') {
                out.push_str(&format!("\n  {field}: {v}"));
            }
        }
    }
    out
}

/// Read `<key>: <value>` out of a displayed object literal. Values run to the next `, <key>: `
/// at brace depth 0, which is how `to_display_string` separates them.
fn extract_field(s: &str, key: &str) -> Option<String> {
    let body = s.strip_prefix('{')?.strip_suffix('}')?;
    let needle = format!("{key}: ");
    let start = if body.starts_with(&needle) {
        0
    } else {
        body.find(&format!(", {needle}"))? + 2
    };
    let rest = &body[start + needle.len()..];
    let bytes = rest.as_bytes();
    let (mut depth, mut i) = (0i32, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            // A comma ends the value only when the next token looks like `key: ` — message
            // text is full of commas.
            b',' if depth == 0 && starts_with_key(&rest[i + 1..]) => break,
            _ => {}
        }
        i += 1;
    }
    Some(rest[..i].trim().to_string())
}

/// Does `s` begin with `<identifier>: `?
fn starts_with_key(s: &str) -> bool {
    let s = s.trim_start();
    match s.split_once(": ") {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        }
        None => false,
    }
}

fn short_path(p: &str) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(rel) = Path::new(p).strip_prefix(&cwd) {
            if let Some(s) = rel.to_str() {
                return s.to_string();
            }
        }
    }
    p.to_string()
}

/// Write a minimal JUnit XML report.
pub fn write_junit_xml(path: &Path, summary: &RunSummary) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    xml.push_str(&format!(
        r#"<testsuites tests="{}" failures="{}" skipped="{}">"#,
        summary.total(),
        summary.failed,
        summary.skipped + summary.todo
    ));
    xml.push('\n');
    xml.push_str(r#"  <testsuite name="tish test">"#);
    xml.push('\n');
    for r in &summary.results {
        let time = r.duration.as_secs_f64();
        xml.push_str(&format!(
            r#"    <testcase name="{}" classname="{}" time="{:.6}">"#,
            xml_escape(&r.full_name),
            xml_escape(&r.file),
            time
        ));
        xml.push('\n');
        match r.status {
            TestStatus::Fail => {
                let msg = r.message.as_deref().unwrap_or("failed");
                xml.push_str(&format!(
                    "      <failure message=\"{}\">{}</failure>\n",
                    xml_escape(msg),
                    xml_escape(msg)
                ));
            }
            TestStatus::Skip | TestStatus::Todo => {
                xml.push_str("      <skipped/>\n");
            }
            TestStatus::Pass | TestStatus::Filtered => {}
        }
        xml.push_str("    </testcase>\n");
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    fs::write(path, xml)
}

/// Escape for XML, dropping control characters. A failure message carrying an ANSI escape or a
/// NUL would otherwise produce a JUnit file no parser accepts.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => out.push(' '),
            c => out.push(c),
        }
    }
    out
}
