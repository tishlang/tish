//! Line coverage collection for `tish test --coverage` (lives entirely in this crate).
//!
//! Hits are recorded by AST-injected calls to `__tish_cov_hit__(fileId, line)`, which works
//! for every execution backend that runs the instrumented program (vm, interp, and later
//! native/js emit of the same AST).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use tishlang_core::Value;

/// Read on every instrumented line, so it stays out of the mutex.
static ENABLED: AtomicBool = AtomicBool::new(false);

static STATE: LazyLock<Mutex<CoverageState>> = LazyLock::new(|| Mutex::new(CoverageState::empty()));

struct CoverageState {
    files: Vec<String>,
    file_index: HashMap<String, u32>,
    /// Modules already rewritten. A module imported by several test files must only be
    /// instrumented once, or its statements would be wrapped (and counted) twice.
    instrumented: HashSet<String>,
    /// file id → executable lines (from instrumentation sites)
    executable: Vec<BTreeSet<u32>>,
    /// file id → line → hits. Indexed by id so the hot path never hashes or clones a path.
    hits: Vec<BTreeMap<u32, u64>>,
}

impl CoverageState {
    fn empty() -> Self {
        Self {
            files: Vec::new(),
            file_index: HashMap::new(),
            instrumented: HashSet::new(),
            executable: Vec::new(),
            hits: Vec::new(),
        }
    }
}

/// Enable coverage collection and clear prior data.
pub fn begin() {
    let mut s = STATE.lock().unwrap();
    *s = CoverageState::empty();
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Intern a source path; returns the numeric id passed to `__tish_cov_hit__`.
pub fn intern_file(path: &Path) -> u32 {
    let key = normalize_path(path);
    let mut s = STATE.lock().unwrap();
    if let Some(&id) = s.file_index.get(&key) {
        return id;
    }
    let id = s.files.len() as u32;
    s.files.push(key.clone());
    s.file_index.insert(key, id);
    s.executable.push(BTreeSet::new());
    s.hits.push(BTreeMap::new());
    id
}

/// Claim `path` for instrumentation. Returns false if it was already rewritten.
pub fn mark_instrumented(path: &Path) -> bool {
    let key = normalize_path(path);
    let mut s = STATE.lock().unwrap();
    s.instrumented.insert(key)
}

pub fn mark_executable(file_id: u32, line: u32) {
    if line == 0 {
        return;
    }
    let mut s = STATE.lock().unwrap();
    if let Some(lines) = s.executable.get_mut(file_id as usize) {
        lines.insert(line);
    }
}

fn record_hit(file_id: u32, line: u32) {
    if line == 0 || !is_enabled() {
        return;
    }
    let mut s = STATE.lock().unwrap();
    if let Some(m) = s.hits.get_mut(file_id as usize) {
        *m.entry(line).or_insert(0) += 1;
    }
}

/// Core native installed as global `__tish_cov_hit__` for VM / via CoreFn for interp.
pub fn hit_native(args: &[Value]) -> Value {
    let file_id = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let line = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    record_hit(file_id, line);
    Value::Null
}

pub fn hit_value() -> Value {
    Value::native(hit_native)
}

const HIT_GLOBAL: &str = "__tish_cov_hit__";

pub fn hit_global_name() -> &'static str {
    HIT_GLOBAL
}

/// Drop test/spec files from the report (coverage of code under test).
pub fn retain_non_test_files() {
    let mut s = STATE.lock().unwrap();
    for id in 0..s.files.len() {
        if crate::discovery::is_test_file_name(
            Path::new(&s.files[id])
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(""),
        ) {
            s.executable[id].clear();
            s.hits[id].clear();
        }
    }
}

/// `(file, executable lines, hits)` for every file with instrumented lines, sorted by path.
fn reportable(s: &CoverageState) -> Vec<(&String, &BTreeSet<u32>, &BTreeMap<u32, u64>)> {
    let mut rows: Vec<_> = s
        .files
        .iter()
        .enumerate()
        .filter(|(id, _)| !s.executable[*id].is_empty())
        .map(|(id, f)| (f, &s.executable[id], &s.hits[id]))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    rows
}

pub fn summary() -> (usize, usize) {
    let s = STATE.lock().unwrap();
    let mut total = 0usize;
    let mut covered = 0usize;
    for (_, lines, hits) in reportable(&s) {
        for line in lines {
            total += 1;
            if hits.get(line).copied().unwrap_or(0) > 0 {
                covered += 1;
            }
        }
    }
    (covered, total)
}

pub fn write_lcov(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let s = STATE.lock().unwrap();
    let mut out = fs::File::create(path)?;
    for (file, lines, file_hits) in reportable(&s) {
        writeln!(out, "TN:")?;
        writeln!(out, "SF:{file}")?;
        let mut lf = 0u32;
        let mut lh = 0u32;
        for &line in lines {
            let count = file_hits.get(&line).copied().unwrap_or(0);
            writeln!(out, "DA:{line},{count}")?;
            lf += 1;
            if count > 0 {
                lh += 1;
            }
        }
        writeln!(out, "LF:{lf}")?;
        writeln!(out, "LH:{lh}")?;
        writeln!(out, "end_of_record")?;
    }
    Ok(())
}

pub fn print_summary<W: Write>(mut w: W) -> io::Result<()> {
    let (covered, total) = summary();
    if total == 0 {
        // "100%" of nothing reads as a passing gate. Say what actually happened.
        writeln!(
            w,
            "\nCoverage: no non-test source was executed — nothing to report."
        )?;
        return Ok(());
    }
    let pct = (covered as f64) * 100.0 / (total as f64);
    writeln!(w, "\nCoverage: {covered}/{total} lines ({pct:.1}%)")?;
    let s = STATE.lock().unwrap();
    for (file, lines, hits) in reportable(&s) {
        let t = lines.len();
        let c = lines
            .iter()
            .filter(|l| hits.get(l).copied().unwrap_or(0) > 0)
            .count();
        let p = if t == 0 {
            0.0
        } else {
            (c as f64) * 100.0 / (t as f64)
        };
        writeln!(w, "  {p:5.1}%  {c}/{t}  {}", short_path(file))?;
    }
    Ok(())
}

pub fn default_dir() -> PathBuf {
    PathBuf::from("coverage")
}

fn normalize_path(p: &Path) -> String {
    p.canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn short_path(p: &str) -> &str {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(rel) = Path::new(p).strip_prefix(&cwd) {
            if let Some(s) = rel.to_str() {
                return s;
            }
        }
    }
    p
}
