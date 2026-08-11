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
}

impl ConsoleReporter {
    pub fn new(kind: ReporterKind) -> Self {
        Self {
            kind,
            out: io::stdout(),
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
                let ms = r.duration.as_secs_f64() * 1000.0;
                let _ = writeln!(self.out, "  {} {} ({:.1}ms)", label, r.full_name, ms);
                if let Some(msg) = &r.message {
                    for line in msg.lines() {
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
        let _ = writeln!(
            self.out,
            "\nTests: {} passed, {} failed, {} skipped, {} todo ({})",
            summary.passed,
            summary.failed,
            summary.skipped,
            summary.todo,
            summary.total()
        );
    }
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
