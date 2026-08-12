//! Execute a collected suite tree.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use regex::Regex;
use tishlang_core::{has_pending_throw, take_pending_throw, value_call, Value};

use crate::isolation::reset_between_files;
use crate::registry::{only_mode, SuiteNode, TestCase, TestMode};
use crate::report::{
    gh_annotation, write_junit_xml, ConsoleReporter, ReporterKind, RunSummary, TestResultRecord,
    TestStatus,
};
use crate::snapshots::{set_ci_mode, set_current_file, set_current_test, set_update_snapshots};

#[derive(Clone, Debug)]
pub struct TestOptions {
    pub roots: Vec<PathBuf>,
    pub filters: Vec<String>,
    pub name_pattern: Option<String>,
    pub timeout_ms: u64,
    pub bail: bool,
    pub retry: u32,
    pub update_snapshots: bool,
    pub reporter: ReporterKind,
    pub junit_path: Option<PathBuf>,
    /// When true, only run tests marked `.only` (also implied by any `.only` registration).
    pub only: bool,
    pub backend: String,
    pub no_optimize: bool,
    pub features: Vec<String>,
    /// `i/n` shard of discovered files (1-based index).
    pub shard: Option<(usize, usize)>,
    /// Shuffle file order.
    pub randomize: bool,
    /// Seed for `--randomize` (also implies randomize when set).
    pub seed: Option<u64>,
    /// Run the full suite this many times.
    pub rerun_each: u32,
    /// Only run tests that include at least one of these tags.
    pub tags: Vec<String>,
    /// Preload modules (absolute or cwd-relative) evaluated before each test file.
    pub preload: Vec<PathBuf>,
    /// Treat a missing snapshot as a failure instead of writing it (also on when `CI` is set).
    pub ci: bool,
    /// Collect line coverage via AST instrumentation (backend-agnostic).
    pub coverage: bool,
    /// Directory for `lcov.info` (implies coverage). Default: `coverage/`.
    pub coverage_dir: Option<PathBuf>,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            roots: vec![PathBuf::from(".")],
            filters: Vec::new(),
            name_pattern: None,
            timeout_ms: 5_000,
            bail: false,
            retry: 0,
            update_snapshots: false,
            reporter: ReporterKind::Console,
            junit_path: None,
            only: false,
            backend: "vm".into(),
            no_optimize: false,
            features: Vec::new(),
            shard: None,
            randomize: false,
            seed: None,
            rerun_each: 1,
            tags: Vec::new(),
            preload: Vec::new(),
            ci: false,
            coverage: false,
            coverage_dir: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestRunResult {
    pub summary: RunSummary,
    pub exit_code: i32,
}

/// Run an already-collected suite (typically one file's tree).
pub fn run_suite(
    suite: &SuiteNode,
    opts: &TestOptions,
    summary: &mut RunSummary,
    reporter: &mut ConsoleReporter,
) -> bool {
    let pattern = opts.name_pattern.as_ref().and_then(|p| Regex::new(p).ok());
    let enforce_only = opts.only || only_mode();
    run_suite_inner(
        suite,
        &[],
        &[],
        opts,
        pattern.as_ref(),
        enforce_only,
        false,
        summary,
        reporter,
    )
}

/// `ancestor_only` is true once any enclosing suite was declared with `describe.only`, so a
/// `.only` on an outer suite selects everything nested beneath it (Jest/Bun semantics) rather
/// than only its direct children.
#[allow(clippy::too_many_arguments)]
fn run_suite_inner(
    suite: &SuiteNode,
    parent_before: &[Value],
    parent_after: &[Value],
    opts: &TestOptions,
    pattern: Option<&Regex>,
    enforce_only: bool,
    ancestor_only: bool,
    summary: &mut RunSummary,
    reporter: &mut ConsoleReporter,
) -> bool {
    if suite.mode == TestMode::Skip {
        // Mark nested tests as skipped without running hooks.
        skip_tree(suite, summary, reporter);
        return true;
    }
    let ancestor_only = ancestor_only || suite.mode == TestMode::Only;

    let mut before_each = parent_before.to_vec();
    before_each.extend(suite.before_each.iter().cloned());
    let mut after_each = suite.after_each.clone();
    after_each.extend(parent_after.iter().cloned());

    if let Err(msg) = run_hooks(&suite.before_all) {
        let rec = TestResultRecord {
            full_name: format!("{} (beforeAll)", suite.name),
            file: first_file_in(suite),
            status: TestStatus::Fail,
            duration: Duration::ZERO,
            message: Some(msg.clone()),
        };
        reporter.on_result(&rec);
        summary.record(rec);
        // Every test below this suite is now unrunnable. Report each one rather than
        // dropping it, so the total count still reflects what the file declared.
        fail_tree(
            suite,
            &format!("suite setup failed (beforeAll): {msg}"),
            summary,
            reporter,
        );
        // `afterAll` still runs: `beforeAll` may have acquired resources before it threw.
        if let Err(after_msg) = run_hooks(&suite.after_all) {
            let rec = TestResultRecord {
                full_name: format!("{} (afterAll)", suite.name),
                file: String::new(),
                status: TestStatus::Fail,
                duration: Duration::ZERO,
                message: Some(after_msg),
            };
            reporter.on_result(&rec);
            summary.record(rec);
        }
        return false;
    }

    let mut ok = true;
    for test in &suite.tests {
        match classify_test(
            test,
            suite,
            pattern,
            enforce_only,
            ancestor_only,
            &opts.tags,
        ) {
            TestDisposition::Run => {}
            disposition => {
                let status = match disposition {
                    TestDisposition::Todo => TestStatus::Todo,
                    TestDisposition::Skip => TestStatus::Skip,
                    TestDisposition::Filtered => TestStatus::Filtered,
                    TestDisposition::Run => unreachable!(),
                };
                let rec = TestResultRecord {
                    full_name: test.full_name.clone(),
                    file: test.file.clone(),
                    status,
                    duration: Duration::ZERO,
                    message: None,
                };
                reporter.on_result(&rec);
                summary.record(rec);
                continue;
            }
        }

        set_current_file(&test.file);
        set_current_test(&test.full_name);
        let timeout = test.timeout_ms.unwrap_or(opts.timeout_ms);
        let mut attempts = 0;
        let max_attempts = opts.retry + 1;
        let mut last_fail: Option<(Duration, String)> = None;

        while attempts < max_attempts {
            attempts += 1;
            let start = Instant::now();
            let _ = take_pending_throw();

            if let Err(msg) = run_hooks(&before_each) {
                last_fail = Some((start.elapsed(), msg));
                let _ = run_hooks(&after_each);
                continue;
            }

            crate::expect::begin_test_assertions();
            let result = run_test_body(&test.callback, timeout);
            // An `expect.assertions(n)` contract turns a body that silently returned early
            // into a failure instead of a pass.
            let result = result.and_then(|()| crate::expect::check_test_assertions());
            let dur = start.elapsed();
            let _ = run_hooks(&after_each);

            match (result, test.mode) {
                (Ok(()), TestMode::Failing) => {
                    last_fail = Some((
                        dur,
                        "Expected test to fail (test.failing), but it passed".into(),
                    ));
                }
                (Ok(()), _) => {
                    let rec = TestResultRecord {
                        full_name: test.full_name.clone(),
                        file: test.file.clone(),
                        status: TestStatus::Pass,
                        duration: dur,
                        message: None,
                    };
                    reporter.on_result(&rec);
                    summary.record(rec);
                    last_fail = None;
                    break;
                }
                (Err(msg), TestMode::Failing) => {
                    let rec = TestResultRecord {
                        full_name: test.full_name.clone(),
                        file: test.file.clone(),
                        status: TestStatus::Pass,
                        duration: dur,
                        message: Some(format!("failed as expected: {msg}")),
                    };
                    reporter.on_result(&rec);
                    summary.record(rec);
                    last_fail = None;
                    break;
                }
                (Err(msg), _) => {
                    last_fail = Some((dur, msg));
                }
            }
        }

        if let Some((dur, msg)) = last_fail {
            if std::env::var_os("GITHUB_ACTIONS").is_some() {
                eprintln!(
                    "::error file={},title={}::{}",
                    test.file,
                    test.full_name,
                    gh_annotation(&msg)
                );
            }
            let rec = TestResultRecord {
                full_name: test.full_name.clone(),
                file: test.file.clone(),
                status: TestStatus::Fail,
                duration: dur,
                message: Some(msg),
            };
            reporter.on_result(&rec);
            summary.record(rec);
            ok = false;
            if opts.bail {
                let _ = run_hooks(&suite.after_all);
                return false;
            }
        }
    }

    for child in &suite.children {
        if !run_suite_inner(
            child,
            &before_each,
            &after_each,
            opts,
            pattern,
            enforce_only,
            ancestor_only,
            summary,
            reporter,
        ) {
            ok = false;
            if opts.bail {
                let _ = run_hooks(&suite.after_all);
                return false;
            }
        }
    }

    if let Err(msg) = run_hooks(&suite.after_all) {
        let rec = TestResultRecord {
            full_name: format!("{} (afterAll)", suite.name),
            file: String::new(),
            status: TestStatus::Fail,
            duration: Duration::ZERO,
            message: Some(msg),
        };
        reporter.on_result(&rec);
        summary.record(rec);
        ok = false;
    }
    ok
}

enum TestDisposition {
    Run,
    Skip,
    Todo,
    Filtered,
}

#[allow(clippy::too_many_arguments)]
fn classify_test(
    test: &TestCase,
    suite: &SuiteNode,
    pattern: Option<&Regex>,
    enforce_only: bool,
    ancestor_only: bool,
    tag_filter: &[String],
) -> TestDisposition {
    // Name / tag / only filters hide tests entirely (including skip/todo that don't match).
    // `ancestor_only` already folds in `suite.mode`, and every enclosing suite besides.
    let _ = suite;
    if enforce_only && test.mode != TestMode::Only && !ancestor_only {
        return TestDisposition::Filtered;
    }
    if let Some(re) = pattern {
        if !re.is_match(&test.full_name) && !re.is_match(&test.name) {
            return TestDisposition::Filtered;
        }
    }
    if !tag_filter.is_empty() {
        let hit = test.tags.iter().any(|t| tag_filter.iter().any(|f| f == t));
        if !hit {
            return TestDisposition::Filtered;
        }
    }
    if test.mode == TestMode::Todo {
        return TestDisposition::Todo;
    }
    if test.mode == TestMode::Skip || suite.mode == TestMode::Skip {
        return TestDisposition::Skip;
    }
    TestDisposition::Run
}

fn skip_tree(suite: &SuiteNode, summary: &mut RunSummary, reporter: &mut ConsoleReporter) {
    mark_tree(suite, TestStatus::Skip, None, summary, reporter);
}

/// Report every test under `suite` as failed — used when a `beforeAll` makes them unrunnable.
fn fail_tree(
    suite: &SuiteNode,
    message: &str,
    summary: &mut RunSummary,
    reporter: &mut ConsoleReporter,
) {
    mark_tree(suite, TestStatus::Fail, Some(message), summary, reporter);
}

fn mark_tree(
    suite: &SuiteNode,
    status: TestStatus,
    message: Option<&str>,
    summary: &mut RunSummary,
    reporter: &mut ConsoleReporter,
) {
    for test in &suite.tests {
        let rec = TestResultRecord {
            full_name: test.full_name.clone(),
            file: test.file.clone(),
            status,
            duration: Duration::ZERO,
            message: message.map(|m| m.to_string()),
        };
        reporter.on_result(&rec);
        summary.record(rec);
    }
    for child in &suite.children {
        mark_tree(child, status, message, summary, reporter);
    }
}

/// First test file path found in the subtree — gives hook failures a file to report against.
fn first_file_in(suite: &SuiteNode) -> String {
    if let Some(t) = suite.tests.first() {
        return t.file.clone();
    }
    for child in &suite.children {
        let f = first_file_in(child);
        if !f.is_empty() {
            return f;
        }
    }
    String::new()
}

fn run_hooks(hooks: &[Value]) -> Result<(), String> {
    for h in hooks {
        let _ = take_pending_throw();
        let result = value_call(h, &[]);
        if has_pending_throw() {
            let err = take_pending_throw().unwrap_or(Value::Null);
            return Err(err.to_display_string());
        }
        #[cfg(feature = "promise")]
        {
            if matches!(result, Value::Promise(_)) {
                let _ = take_pending_throw();
                let _ = tishlang_runtime::await_promise(result);
                if has_pending_throw() {
                    let err = take_pending_throw().unwrap_or(Value::Null);
                    return Err(err.to_display_string());
                }
            }
        }
        #[cfg(not(feature = "promise"))]
        {
            let _ = result;
        }
    }
    Ok(())
}

/// Run one test body under `timeout_ms`.
///
/// The backends poll [`tishlang_core::execution_deadline_exceeded`] on loop back-edges, so a
/// runaway loop is aborted rather than hanging the run. A body that blocks inside a single
/// native call (a synchronous socket read, say) still cannot be preempted; the elapsed-time
/// check after it returns catches that case.
fn run_test_body(body: &Value, timeout_ms: u64) -> Result<(), String> {
    let _ = take_pending_throw();
    let start = Instant::now();
    let deadline = Duration::from_millis(timeout_ms);
    let timed_out = |extra: &str| -> String {
        if extra.is_empty() {
            format!("Test timed out after {timeout_ms}ms")
        } else {
            format!("Test timed out after {timeout_ms}ms ({extra})")
        }
    };

    tishlang_core::set_execution_deadline(Some(timeout_ms));
    let result = value_call(body, &[]);
    let tripped = tishlang_core::execution_deadline_tripped();
    tishlang_core::set_execution_deadline(None);

    if tripped {
        let _ = take_pending_throw();
        return Err(timed_out(""));
    }
    if has_pending_throw() {
        let err = take_pending_throw().unwrap_or(Value::Null);
        return Err(err.to_display_string());
    }
    if start.elapsed() > deadline {
        return Err(timed_out("body returned late"));
    }
    // Await promise-returning tests when the promise feature is on.
    #[cfg(feature = "promise")]
    {
        if matches!(result, Value::Promise(_)) {
            let _ = take_pending_throw();
            let remaining = deadline.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(timed_out("before await"));
            }
            tishlang_core::set_execution_deadline(Some(remaining.as_millis() as u64));
            let _ = tishlang_runtime::await_promise(result);
            let tripped = tishlang_core::execution_deadline_tripped();
            tishlang_core::set_execution_deadline(None);
            if tripped {
                let _ = take_pending_throw();
                return Err(timed_out("awaiting"));
            }
            if has_pending_throw() {
                let err = take_pending_throw().unwrap_or(Value::Null);
                return Err(err.to_display_string());
            }
            if start.elapsed() > deadline {
                return Err(timed_out("awaiting"));
            }
        }
    }
    #[cfg(not(feature = "promise"))]
    {
        let _ = result;
    }
    Ok(())
}

/// High-level entry: discover files, load each (when `runner` feature), execute, report.
pub fn run_tests(opts: TestOptions) -> TestRunResult {
    let mut opts = opts;
    apply_package_config(&mut opts);

    let times = opts.rerun_each.max(1);
    let mut last = TestRunResult {
        summary: RunSummary::default(),
        exit_code: 0,
    };
    // `--rerun-each` exists to surface flakes: ANY failing pass fails the command. Reporting only
    // the final pass would make a run that failed once and passed twice exit 0.
    let mut worst_exit = 0;
    let mut failed_passes = Vec::new();
    for i in 0..times {
        if times > 1 {
            eprintln!("── rerun-each {}/{} ──", i + 1, times);
        }
        last = run_tests_once(&opts);
        if last.exit_code != 0 {
            worst_exit = last.exit_code;
            failed_passes.push(i + 1);
            if opts.bail {
                break;
            }
        }
    }
    if times > 1 && !failed_passes.is_empty() {
        eprintln!(
            "\n{} of {} passes failed (pass {}) — flaky or consistently failing.",
            failed_passes.len(),
            times,
            failed_passes
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    last.exit_code = worst_exit;
    last
}

fn apply_package_config(opts: &mut TestOptions) {
    let cfg =
        crate::config::load_tish_test_config(opts.roots.first().unwrap_or(&PathBuf::from(".")));
    if opts.roots == vec![PathBuf::from(".")] && !cfg.root.is_empty() {
        opts.roots = cfg.root;
    }
    if opts.timeout_ms == 5_000 {
        if let Some(t) = cfg.timeout_ms {
            opts.timeout_ms = t;
        }
    }
    for p in cfg.preload {
        if !opts.preload.contains(&p) {
            opts.preload.push(p);
        }
    }
}

fn run_tests_once(opts: &TestOptions) -> TestRunResult {
    set_update_snapshots(opts.update_snapshots);
    set_ci_mode(opts.ci);
    set_ci_mode(opts.ci);
    let coverage_on = opts.coverage || opts.coverage_dir.is_some();
    if coverage_on {
        crate::coverage::begin();
    }
    let mut summary = RunSummary::default();
    let mut reporter = ConsoleReporter::new(opts.reporter);

    let mut files = crate::discovery::discover_tests(&opts.roots, &opts.filters);
    if opts.randomize || opts.seed.is_some() {
        let seed = opts.seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
        });
        eprintln!("--seed={seed}");
        shuffle_with_seed(&mut files, seed);
    }
    if let Some((index, total)) = opts.shard {
        if total > 0 && index >= 1 && index <= total {
            files = files
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % total == index - 1)
                .map(|(_, f)| f)
                .collect();
        }
    }
    if files.is_empty() {
        eprintln!("No test files found.");
    }

    // GitHub Actions annotations on failure
    let gh = std::env::var_os("GITHUB_ACTIONS").is_some();

    for file in &files {
        reset_between_files();
        set_current_file(&file.to_string_lossy());

        #[cfg(feature = "runner")]
        {
            match crate::load::load_and_collect_tests(
                file,
                &opts.preload,
                &opts.backend,
                opts.no_optimize,
                &opts.features,
            ) {
                Ok(loaded) => {
                    // A throw inside a `describe` body truncates registration silently. Report
                    // each one as a failure so a file that lost tests can never exit 0.
                    let had_collect_errors = !loaded.collect_errors.is_empty();
                    for err in &loaded.collect_errors {
                        if gh {
                            eprintln!("::error file={}::{}", file.display(), gh_annotation(err));
                        }
                        let rec = TestResultRecord {
                            full_name: format!("{} (collection error)", file.display()),
                            file: file.display().to_string(),
                            status: TestStatus::Fail,
                            duration: Duration::ZERO,
                            message: Some(err.clone()),
                        };
                        reporter.on_result(&rec);
                        summary.record(rec);
                    }
                    let suite_ok = run_suite(&loaded.suite, opts, &mut summary, &mut reporter);
                    if (!suite_ok || had_collect_errors) && opts.bail {
                        break;
                    }
                }
                Err(e) => {
                    if gh {
                        eprintln!("::error file={}::{}", file.display(), gh_annotation(&e));
                    }
                    let rec = TestResultRecord {
                        full_name: file.display().to_string(),
                        file: file.display().to_string(),
                        status: TestStatus::Fail,
                        duration: Duration::ZERO,
                        message: Some(e),
                    };
                    reporter.on_result(&rec);
                    summary.record(rec);
                    if opts.bail {
                        break;
                    }
                }
            }
        }

        #[cfg(not(feature = "runner"))]
        {
            let _ = file;
            let rec = TestResultRecord {
                full_name: "runner feature disabled".into(),
                file: String::new(),
                status: TestStatus::Fail,
                duration: Duration::ZERO,
                message: Some(
                    "tishlang_test built without `runner` feature; cannot load test files".into(),
                ),
            };
            reporter.on_result(&rec);
            summary.record(rec);
            break;
        }
    }

    reporter.finish(&summary);
    if let Some(path) = &opts.junit_path {
        if let Err(e) = write_junit_xml(path, &summary) {
            eprintln!("Failed to write junit report: {e}");
        }
    }

    if coverage_on {
        crate::coverage::retain_non_test_files();
        let dir = opts
            .coverage_dir
            .clone()
            .unwrap_or_else(crate::coverage::default_dir);
        let lcov = dir.join("lcov.info");
        if let Err(e) = crate::coverage::write_lcov(&lcov) {
            eprintln!("Failed to write coverage report: {e}");
        } else {
            eprintln!("Wrote {}", lcov.display());
        }
        let _ = crate::coverage::print_summary(std::io::stderr());
    }

    TestRunResult {
        exit_code: if summary.success() { 0 } else { 1 },
        summary,
    }
}

fn shuffle_with_seed(files: &mut [PathBuf], seed: u64) {
    // Simple LCG shuffle (Fisher–Yates).
    let mut state = seed;
    for i in (1..files.len()).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (state as usize) % (i + 1);
        files.swap(i, j);
    }
}

/// Paths a run writes into itself. Watching these would make `--watch --coverage` (or `-u`,
/// or `--reporter-outfile` inside the tree) re-trigger on its own output forever.
#[cfg(feature = "watch")]
fn is_self_written(path: &std::path::Path, opts: &TestOptions) -> bool {
    if let Some(junit) = &opts.junit_path {
        if path == junit.as_path() {
            return true;
        }
    }
    let coverage_dir = opts
        .coverage_dir
        .clone()
        .unwrap_or_else(crate::coverage::default_dir);
    path.components().any(|c| {
        let s = c.as_os_str();
        s == "__snapshots__" || s == coverage_dir.as_os_str() || s == ".git"
    })
}

/// Run tests once, then re-run when files under `opts.roots` change (`--watch`).
///
/// With the `watch` feature (notify), blocks forever on filesystem events. Without it,
/// runs a single pass and returns (same as [`run_tests`]).
pub fn run_tests_watch(opts: TestOptions) -> i32 {
    #[cfg(feature = "watch")]
    {
        use notify::{RecursiveMode, Watcher};
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let (tx, rx) = channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("tish test --watch: failed to start watcher ({e}); running once");
                return run_tests(opts).exit_code;
            }
        };
        for root in &opts.roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                eprintln!("tish test --watch: cannot watch {}: {e}", root.display());
            }
        }
        eprintln!("tish test --watch: watching for changes (Ctrl-C to stop)");
        loop {
            let result = run_tests(opts.clone());
            // Block until a change we did not cause ourselves, then debounce briefly.
            let relevant = loop {
                match rx.recv() {
                    Ok(Ok(event)) if event.paths.iter().any(|p| !is_self_written(p, &opts)) => {
                        break true
                    }
                    // Our own snapshot / coverage / junit writes: keep waiting.
                    Ok(_) => continue,
                    Err(_) => break false,
                }
            };
            if !relevant {
                return result.exit_code;
            }
            while rx.recv_timeout(Duration::from_millis(150)).is_ok() {}
            eprintln!("\n── file change; re-running ──");
        }
    }
    #[cfg(not(feature = "watch"))]
    {
        eprintln!("tish test --watch: notify feature not enabled in this build; running once");
        run_tests(opts).exit_code
    }
}
