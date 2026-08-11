//! Line coverage collection for `tish test --coverage` (lives entirely in this crate).
//!
//! Hits are recorded by AST-injected calls to `__tish_cov_hit__(fileId, line)`, which works
//! for every execution backend that runs the instrumented program (vm, interp, and later
//! native/js emit of the same AST).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use std::sync::LazyLock;

use tishlang_core::Value;

static STATE: LazyLock<Mutex<CoverageState>> =
    LazyLock::new(|| Mutex::new(CoverageState::empty()));

struct CoverageState {
    enabled: bool,
    files: Vec<String>,
    file_index: HashMap<String, u32>,
    /// file → executable lines (from instrumentation sites)
    executable: BTreeMap<String, BTreeSet<u32>>,
    /// file → line → hits
    hits: BTreeMap<String, BTreeMap<u32, u64>>,
}

impl CoverageState {
    fn empty() -> Self {
        Self {
            enabled: false,
            files: Vec::new(),
            file_index: HashMap::new(),
            executable: BTreeMap::new(),
            hits: BTreeMap::new(),
        }
    }
}

/// Enable coverage collection and clear prior data.
pub fn begin() {
    let mut s = STATE.lock().unwrap();
    *s = CoverageState {
        enabled: true,
        files: Vec::new(),
        file_index: HashMap::new(),
        executable: BTreeMap::new(),
        hits: BTreeMap::new(),
    };
}

pub fn is_enabled() -> bool {
    STATE.lock().unwrap().enabled
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
    id
}

pub fn mark_executable(file_id: u32, line: u32) {
    if line == 0 {
        return;
    }
    let mut s = STATE.lock().unwrap();
    if let Some(path) = s.files.get(file_id as usize).cloned() {
        s.executable.entry(path).or_default().insert(line);
    }
}

fn record_hit(file_id: u32, line: u32) {
    if line == 0 {
        return;
    }
    let mut s = STATE.lock().unwrap();
    if !s.enabled {
        return;
    }
    let Some(path) = s.files.get(file_id as usize).cloned() else {
        return;
    };
    *s.hits.entry(path).or_default().entry(line).or_insert(0) += 1;
}

/// Core native installed as global `__tish_cov_hit__` for VM / via CoreFn for interp.
pub fn hit_native(args: &[Value]) -> Value {
    let file_id = args
        .first()
        .and_then(|v| v.as_number())
        .unwrap_or(0.0) as u32;
    let line = args
        .get(1)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0) as u32;
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
    let is_test = |p: &str| {
        let name = Path::new(p)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        name.ends_with(".test.tish")
            || name.ends_with("_test.tish")
            || name.ends_with(".spec.tish")
            || name.ends_with("_spec.tish")
    };
    let mut s = STATE.lock().unwrap();
    s.executable.retain(|k, _| !is_test(k));
    s.hits.retain(|k, _| !is_test(k));
}

pub fn summary() -> (usize, usize) {
    let s = STATE.lock().unwrap();
    let mut total = 0usize;
    let mut covered = 0usize;
    for (file, lines) in &s.executable {
        for &line in lines {
            total += 1;
            let h = s
                .hits
                .get(file)
                .and_then(|m| m.get(&line))
                .copied()
                .unwrap_or(0);
            if h > 0 {
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
    for (file, lines) in &s.executable {
        writeln!(out, "TN:")?;
        writeln!(out, "SF:{file}")?;
        let file_hits = s.hits.get(file);
        let mut lf = 0u32;
        let mut lh = 0u32;
        for &line in lines {
            let count = file_hits.and_then(|m| m.get(&line)).copied().unwrap_or(0);
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
    let pct = if total == 0 {
        100.0
    } else {
        (covered as f64) * 100.0 / (total as f64)
    };
    writeln!(w, "\nCoverage: {covered}/{total} lines ({pct:.1}%)")?;
    let s = STATE.lock().unwrap();
    for (file, lines) in &s.executable {
        let mut c = 0usize;
        let t = lines.len();
        for &line in lines {
            let h = s
                .hits
                .get(file)
                .and_then(|m| m.get(&line))
                .copied()
                .unwrap_or(0);
            if h > 0 {
                c += 1;
            }
        }
        let p = if t == 0 {
            100.0
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
