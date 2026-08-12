//! Regression suite for `tish test` itself.
//!
//! Every case here pins a behavior where the runner previously reported success while tests
//! were broken, missing, or never run. A test framework that can exit 0 with lost tests is
//! worse than no framework, so these assert on the **exit code** as much as on the output.
//!
//! Each case writes its fixtures to a fresh temp dir under `target/` (never `/tmp`) and runs
//! the real binary, so the CLI wiring is covered too.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A throwaway directory under `target/`, removed on drop.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-runner-fixtures")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create sandbox");
        Self { dir }
    }

    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(PathBuf::from(env!("CARGO_BIN_EXE_tish")))
            .arg("test")
            .args(args)
            .current_dir(&self.dir)
            .env_remove("CI")
            .output()
            .expect("spawn tish test")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_failed(out: &Output, why: &str) {
    assert!(
        !out.status.success(),
        "expected a non-zero exit ({why}), got success. Output:\n{}",
        combined(out)
    );
}

fn assert_passed(out: &Output, why: &str) {
    assert!(
        out.status.success(),
        "expected exit 0 ({why}). Output:\n{}",
        combined(out)
    );
}

/// A throw inside a `describe` body silently dropped every later registration and exited 0.
#[test]
fn collection_error_fails_the_run() {
    let sb = Sandbox::new("collect_error");
    sb.write(
        "a.test.tish",
        r#"import { describe, test, expect } from "tish:test"
describe("outer", () => {
  test("first", () => { expect(1).toBe(1) })
  throw "collection blew up"
  test("never registered", () => { expect(1).toBe(2) })
})
"#,
    );
    let out = sb.run(&[]);
    assert_failed(&out, "a describe body threw during collection");
    let text = combined(&out);
    assert!(
        text.contains("collection blew up") && text.contains("collection error"),
        "collection error not reported:\n{text}"
    );
}

/// `--rerun-each` reported only the final pass, so a run that failed once and passed twice
/// exited 0 — defeating the flake detection the flag exists for.
#[test]
fn rerun_each_fails_if_any_pass_failed() {
    let sb = Sandbox::new("rerun_each");
    sb.write(
        "flake.test.tish",
        r#"import { test, expect } from "tish:test"
import { readFileSync, writeFileSync, existsSync } from "tish:fs"
test("fails on the first pass only", () => {
  let n = existsSync("./counter.txt") ? Number(readFileSync("./counter.txt", "utf8")) : 0
  writeFileSync("./counter.txt", String(n + 1))
  expect(n).toBeGreaterThan(0)
})
"#,
    );
    let out = sb.run(&["--rerun-each", "3"]);
    assert_failed(&out, "pass 1 of 3 failed");
    assert!(
        combined(&out).contains("passes failed"),
        "no flake summary:\n{}",
        combined(&out)
    );
}

/// `--feature` was accepted and ignored, so a restricted run still had every capability.
#[test]
fn feature_flag_restricts_capabilities() {
    let sb = Sandbox::new("features");
    sb.write(
        "fs.test.tish",
        r#"import { test, expect } from "tish:test"
import { writeFileSync } from "tish:fs"
test("writes", () => {
  writeFileSync("./written.txt", "x")
  expect(1).toBe(1)
})
"#,
    );
    assert_passed(&sb.run(&["--feature", "fs"]), "fs granted");
    assert_failed(
        &sb.run(&["--feature", "timers"]),
        "fs must be denied when only timers is granted",
    );
}

/// Two unnamed `toMatchSnapshot()` calls in sibling tests collided on one `.snap` file, so
/// each test was diffed against the other's value.
#[test]
fn unnamed_snapshots_do_not_collide_across_tests() {
    let sb = Sandbox::new("snapshots");
    sb.write(
        "snap.test.tish",
        r#"import { test, expect } from "tish:test"
test("alpha", () => { expect({ v: "alpha" }).toMatchSnapshot() })
test("beta", () => { expect({ v: "beta" }).toMatchSnapshot() })
"#,
    );
    assert_passed(&sb.run(&[]), "first run writes both snapshots");
    assert_passed(&sb.run(&[]), "second run compares against its own snapshot");

    let snaps: Vec<String> = std::fs::read_dir(sb.dir.join("__snapshots__"))
        .expect("snapshot dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        snaps.len(),
        2,
        "expected one snapshot per test, got {snaps:?}"
    );
}

/// In CI a *missing* snapshot was written and passed, so a deleted `.snap` went green on the
/// very run that should have caught it.
#[test]
fn ci_mode_fails_on_a_missing_snapshot() {
    let sb = Sandbox::new("snapshots_ci");
    sb.write(
        "snap.test.tish",
        r#"import { test, expect } from "tish:test"
test("alpha", () => { expect({ v: "alpha" }).toMatchSnapshot() })
"#,
    );
    assert_failed(&sb.run(&["--ci"]), "no snapshot exists yet and --ci is set");
    assert_passed(&sb.run(&[]), "without --ci the snapshot is written");
    assert_passed(&sb.run(&["--ci"]), "now that it exists --ci passes");
}

/// `describe.only` did not narrow the run at all, and once only-mode was active, tests nested
/// more than one level under it were silently filtered out.
#[test]
fn describe_only_selects_the_whole_subtree() {
    let sb = Sandbox::new("only");
    sb.write(
        "only.test.tish",
        r#"import { describe, test, expect } from "tish:test"
describe.only("kept", () => {
  test("direct", () => { expect(1).toBe(1) })
  describe("nested", () => {
    test("one level down", () => { expect(1).toBe(1) })
    describe("deeper", () => {
      test("deep", () => { expect(1).toBe(1) })
    })
  })
})
describe("dropped", () => {
  test("must not run", () => { expect(1).toBe(2) })
})
"#,
    );
    let out = sb.run(&[]);
    assert_passed(&out, "only the `.only` subtree runs, and it passes");
    let text = combined(&out);
    assert!(
        text.contains("deep"),
        "nested-under-only test was dropped:\n{text}"
    );
    assert!(
        text.contains("3 passed"),
        "expected all 3 subtree tests to run:\n{text}"
    );
}

/// A runaway loop could not be interrupted: the deadline was only checked after the body
/// returned, which never happened.
#[test]
fn timeout_interrupts_a_runaway_loop() {
    let sb = Sandbox::new("timeout");
    sb.write(
        "spin.test.tish",
        r#"import { test, expect } from "tish:test"
test("spins", () => { let i = 0; while (true) { i = i + 1 } })
test("still runs afterwards", () => { expect(1).toBe(1) })
"#,
    );
    let started = std::time::Instant::now();
    let out = sb.run(&["--timeout", "500"]);
    let elapsed = started.elapsed();
    assert_failed(&out, "the spinning test must time out");
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "runner did not abort the loop (took {elapsed:?})"
    );
    let text = combined(&out);
    assert!(text.contains("timed out"), "no timeout message:\n{text}");
    assert!(
        text.contains("still runs afterwards"),
        "the run did not continue past the timed-out test:\n{text}"
    );
}

/// `assert.throws(fn, matcher)` treated the matcher as a message, so any throw satisfied it;
/// `assert.rejects` / `doesNotReject` never invoked a function argument.
#[test]
fn node_assert_validates_errors_and_rejections() {
    let sb = Sandbox::new("assert");
    sb.write(
        "ok.test.tish",
        r#"import { test } from "tish:test"
import assert from "node:assert/strict"
test("matching pattern", () => { assert.throws(() => { throw "Boom" }, /Boom/) })
test("rejects on an async fn", () => { assert.rejects(async () => { throw "nope" }) })
test("doesNotReject on a clean fn", () => { assert.doesNotReject(async () => { return 1 }) })
"#,
    );
    assert_passed(&sb.run(&[]), "the well-behaved assertions pass");

    let bad = Sandbox::new("assert_bad");
    bad.write(
        "bad.test.tish",
        r#"import { test } from "tish:test"
import assert from "node:assert/strict"
test("non-matching pattern must fail", () => {
  assert.throws(() => { throw "Boom" }, /TotallyDifferent/)
})
"#,
    );
    assert_failed(
        &bad.run(&[]),
        "the thrown value does not match the expected pattern",
    );

    let bad2 = Sandbox::new("assert_bad2");
    bad2.write(
        "bad2.test.tish",
        r#"import { test } from "tish:test"
import assert from "node:assert/strict"
test("doesNotReject on a rejecting fn must fail", () => {
  assert.doesNotReject(async () => { throw "rejects" })
})
"#,
    );
    assert_failed(&bad2.run(&[]), "the function rejects");
}

/// `expect(nonFunction).toThrow()` passed because the failed call itself set a pending throw.
#[test]
fn to_throw_rejects_non_functions() {
    let sb = Sandbox::new("to_throw");
    sb.write(
        "t.test.tish",
        r#"import { test, expect } from "tish:test"
test("not a function", () => { expect("a string").toThrow() })
"#,
    );
    assert_failed(&sb.run(&[]), "a string cannot throw");
}

/// Matcher semantics that previously diverged from Jest/Bun or were unusable outright.
#[test]
fn matcher_semantics_match_jest() {
    let sb = Sandbox::new("matchers");
    sb.write(
        "m.test.tish",
        r#"import { test, expect } from "tish:test"
test("identical functions are equal", () => {
  let f = () => 1
  expect({ f: f }).toEqual({ f: f })
})
test("null is the absence value", () => {
  expect(null).toBeUndefined()
  expect(1).toBeDefined()
})
test("added matchers", () => {
  expect({ a: { b: [1, 2] } }).toHaveProperty("a.b.1", 2)
  expect(0 / 0).toBeNaN()
  expect("x").toBeTypeOf("string")
  expect([{ id: 1 }]).toContainEqual({ id: 1 })
})
test("asymmetric matchers", () => {
  expect([1, 2, 3]).toEqual(expect.arrayContaining([1, 2]))
  expect({ a: 1, b: 2 }).toEqual(expect.objectContaining({ a: 1 }))
})
"#,
    );
    assert_passed(&sb.run(&[]), "all matcher semantics hold");

    let bad = Sandbox::new("matchers_bad");
    bad.write(
        "m.test.tish",
        r#"import { test, expect } from "tish:test"
test("jest fails this: |0.005 - 0| is not < 10^-2 / 2", () => {
  expect(0.005).toBeCloseTo(0, 2)
})
"#,
    );
    assert_failed(
        &bad.run(&[]),
        "toBeCloseTo must use Jest's half-precision rule",
    );
}

/// `expect.assertions(n)` turns a body that returned early into a failure rather than a pass.
#[test]
fn assertion_count_contract_is_enforced() {
    let sb = Sandbox::new("assertion_count");
    sb.write(
        "a.test.tish",
        r#"import { test, expect } from "tish:test"
test("promises two assertions but makes one", () => {
  expect.assertions(2)
  expect(1).toBe(1)
})
"#,
    );
    assert_failed(&sb.run(&[]), "only one assertion was made");

    let ok = Sandbox::new("assertion_count_ok");
    ok.write(
        "a.test.tish",
        r#"import { test, expect } from "tish:test"
test("keeps its promise", () => {
  expect.assertions(2)
  expect(1).toBe(1)
  expect(2).toBe(2)
})
"#,
    );
    assert_passed(&ok.run(&[]), "the contract is satisfied");
}

/// A failing `beforeAll` dropped its tests from the report entirely and skipped `afterAll`,
/// leaking whatever the hook had already acquired.
#[test]
fn before_all_failure_reports_tests_and_runs_after_all() {
    let sb = Sandbox::new("hooks");
    sb.write(
        "h.test.tish",
        r#"import { describe, test, expect, beforeAll, afterAll } from "tish:test"
describe("setup fails", () => {
  beforeAll(() => { throw "boom" })
  afterAll(() => { console.log("AFTER_ALL_RAN") })
  test("t1", () => { expect(1).toBe(1) })
  test("t2", () => { expect(1).toBe(1) })
})
"#,
    );
    let out = sb.run(&[]);
    assert_failed(&out, "beforeAll threw");
    let text = combined(&out);
    assert!(
        text.contains("AFTER_ALL_RAN"),
        "afterAll did not run:\n{text}"
    );
    assert!(
        text.contains("3 failed"),
        "expected the hook plus both tests to be reported:\n{text}"
    );
}

/// `--preload` ran in its own backend instance (so nothing it defined was visible) and
/// discarded load errors, making a typo'd path a silent no-op.
#[test]
fn preload_shares_the_backend_and_surfaces_errors() {
    let sb = Sandbox::new("preload");
    sb.write(
        "setup.tish",
        r#"import { beforeEach } from "tish:test"
beforeEach(() => { console.log("PRELOAD_HOOK") })
"#,
    );
    sb.write(
        "p.test.tish",
        r#"import { test, expect } from "tish:test"
test("t", () => { expect(1).toBe(1) })
"#,
    );
    let out = sb.run(&["--preload", "./setup.tish"]);
    assert_passed(&out, "preload + test both load");
    assert!(
        combined(&out).contains("PRELOAD_HOOK"),
        "the preload's beforeEach did not reach the test file:\n{}",
        combined(&out)
    );

    assert_failed(
        &sb.run(&["--preload", "./nope.tish"]),
        "a missing preload must not be silently ignored",
    );
}

/// `--reporter dots` printed a bare `F` and no diagnostics at all.
#[test]
fn dots_reporter_still_reports_failures() {
    let sb = Sandbox::new("dots");
    sb.write(
        "d.test.tish",
        r#"import { test, expect } from "tish:test"
test("passes", () => { expect(1).toBe(1) })
test("fails", () => { expect(1).toBe(2) })
"#,
    );
    let out = sb.run(&["--reporter", "dots"]);
    assert_failed(&out, "one test fails");
    let text = combined(&out);
    assert!(text.contains("Failures:"), "no failure section:\n{text}");
    assert!(
        text.contains("Expected: 2") && text.contains("Received: 1"),
        "failure diagnostics missing from dots output:\n{text}"
    );
}

/// A thrown `AssertionError` was printed as a whole object literal, burying the message.
#[test]
fn failures_are_printed_readably() {
    let sb = Sandbox::new("format");
    sb.write(
        "f.test.tish",
        r#"import { test, expect } from "tish:test"
test("fails", () => { expect(1).toBe(2) })
"#,
    );
    let out = sb.run(&[]);
    assert_failed(&out, "the test fails");
    let text = combined(&out);
    assert!(
        text.contains("AssertionError: expect(received).toBe(expected)"),
        "failure was not reformatted:\n{text}"
    );
    assert!(
        !text.contains("{generatedMessage:") && !text.contains("{code: ERR_ASSERTION"),
        "raw object dump leaked into the report:\n{text}"
    );
}

/// Coverage skipped arrow bodies nested in most expression forms, so they never entered the
/// denominator and the reported percentage was inflated.
#[test]
fn coverage_counts_arrows_in_nested_expressions() {
    let sb = Sandbox::new("coverage");
    sb.write(
        "src/lib.tish",
        r#"export fn covered(x) {
  let a = x + 1
  return a
}
export let pick = true ? (v) => {
  let inCond = v * 2
  return inCond
} : (v) => v
export let holder = {}
holder.cb = (v) => {
  let inAssign = v + 5
  return inAssign
}
"#,
    );
    sb.write(
        "c.test.tish",
        r#"import { test, expect } from "tish:test"
import { covered, pick, holder } from "./src/lib.tish"
test("covers", () => {
  expect(covered(1)).toBe(2)
  expect(pick(2)).toBe(4)
  expect(holder.cb(1)).toBe(6)
})
"#,
    );
    let out = sb.run(&["--coverage"]);
    assert_passed(&out, "the suite passes under coverage");
    let lcov = std::fs::read_to_string(sb.dir.join("coverage/lcov.info")).expect("lcov written");
    // Lines 6-7 (conditional arrow) and 11-12 (member-assign arrow) must be present.
    for line in ["DA:6,", "DA:7,", "DA:11,", "DA:12,"] {
        assert!(
            lcov.contains(line),
            "coverage omitted a nested arrow body ({line}):\n{lcov}"
        );
    }
}

/// An all-test-file run reported "100%" coverage of nothing, which reads as a passing gate.
#[test]
fn empty_coverage_does_not_claim_full_coverage() {
    let sb = Sandbox::new("coverage_empty");
    sb.write(
        "e.test.tish",
        r#"import { test, expect } from "tish:test"
test("t", () => { expect(1).toBe(1) })
"#,
    );
    let out = sb.run(&["--coverage"]);
    assert_passed(&out, "the suite passes");
    let text = combined(&out);
    assert!(
        !text.contains("100.0%"),
        "reported full coverage with nothing instrumented:\n{text}"
    );
    assert!(
        text.contains("nothing to report"),
        "empty coverage was not called out:\n{text}"
    );
}

/// `--backend interp` was advertised but failed on the simplest possible test file, because
/// interpreter closures cannot be invoked after the evaluator is dropped.
#[test]
fn unsupported_backends_are_rejected_up_front() {
    let sb = Sandbox::new("backend");
    sb.write(
        "b.test.tish",
        r#"import { test, expect } from "tish:test"
test("t", () => { expect(1).toBe(1) })
"#,
    );
    for backend in ["interp", "native"] {
        let out = sb.run(&["--backend", backend]);
        assert_failed(&out, "unsupported backend");
        let text = combined(&out);
        assert!(
            text.contains("unsupported --backend"),
            "no clear rejection for --backend {backend}:\n{text}"
        );
    }
}

/// Sanity: the happy path still works end to end, including discovery and nesting.
#[test]
fn a_passing_suite_exits_zero() {
    let sb = Sandbox::new("happy");
    sb.write(
        "nested/ok.test.tish",
        r#"import { describe, test, expect, beforeEach } from "tish:test"
let seen = 0
describe("math", () => {
  beforeEach(() => { seen = seen + 1 })
  test("adds", () => { expect(1 + 1).toBe(2) })
  test.each([[1, 1], [2, 4]])("squares %i", (n, sq) => { expect(n * n).toBe(sq) })
  test.skip("skipped", () => { expect(1).toBe(2) })
  test.todo("todo")
})
"#,
    );
    let out = sb.run(&[]);
    assert_passed(&out, "the happy path");
    let text = combined(&out);
    assert!(
        text.contains("3 passed") && text.contains("1 skipped") && text.contains("1 todo"),
        "unexpected summary:\n{text}"
    );
}

/// Discovery only picks up recognized test-file suffixes.
#[test]
fn discovery_ignores_non_test_files() {
    let sb = Sandbox::new("discovery");
    sb.write(
        "helper.tish",
        "export fn helper() { return 1 }\nthrow \"helper must not run as a suite\"\n",
    );
    sb.write(
        "real.spec.tish",
        r#"import { test, expect } from "tish:test"
test("t", () => { expect(1).toBe(1) })
"#,
    );
    let out = sb.run(&[]);
    assert_passed(&out, "only real.spec.tish is a suite");
    assert!(
        combined(&out).contains("1 passed"),
        "unexpected discovery result:\n{}",
        combined(&out)
    );
}
