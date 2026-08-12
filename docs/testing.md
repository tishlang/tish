# Testing in Tish

Native test runner: **`tish test`**. The API is Bun/Jest-shaped via `tish:test`, plus a
Node-mirrored `assert` via `tish:assert` / `node:assert` / `node:assert/strict`.

## Tish is not JavaScript

**`tish test` runs on the Tish bytecode VM. It does not execute JavaScript.**

| Runner | What it proves |
|--------|----------------|
| `tish test` / `tish run` | Language + runtime semantics on the **Tish** VM |
| `tish build --target js` + **node** (or bun) | Behavior of the **compiled JS emit** under a JS engine |
| Vite / jsdom / browser | Host integration of that JS emit |

Moving a suite from `build --target js && node` → `tish test` **changes the subject under
test**; pass/fail may diverge. Keep Node dual-runs for JS-emit, browser, and DomHost contracts —
never drop an emit leg just because a native suite was added.

## Quick start

```bash
tish test                              # discover under .
tish test ./path/to/pkg                # discovery root
tish test ./foo.test.tish              # exact file
tish test -t "adds" --bail
```

```tish
import { describe, it, expect } from "tish:test"

describe("math", () => {
  it("adds", () => {
    expect(1 + 1).toBe(2)
  })
})
```

```tish
import { test } from "tish:test"
import assert from "node:assert/strict"

test("strict", () => {
  assert.strictEqual(1, 1)
})
```

Discovery globs: `*.test.tish`, `*_test.tish`, `*.spec.tish`, `*_spec.tish`.
Aliases: `node:test` and bare `test` normalize to `tish:test`.

The runner's own suites double as worked examples:
[`tests/runner/api.test.tish`](../tests/runner/api.test.tish) and
[`tests/runner/assert.test.tish`](../tests/runner/assert.test.tish).

## `tish:test`

| Export | Role |
|--------|------|
| `describe` / `suite` (+ `.skip` / `.only` / `.each`) | Suite grouping (collect-then-run) |
| `test` / `it` (+ `.skip` / `.only` / `.todo` / `.failing` / `.skipIf` / `.each`) | Cases |
| `expect(x)` | Matchers (see below) |
| `beforeAll` / `afterAll` / `beforeEach` / `afterEach` | Hooks (`before` / `after` aliases) |
| `mock` / `spyOn` / `clearAllMocks` / `restoreAllMocks` | Doubles |

Matchers: `toBe`, `toEqual`, `toStrictEqual`, `toMatchObject`, `toMatch`, `toContain`,
`toContainEqual`, `toHaveLength`, `toHaveProperty`, `toBeNull`, `toBeDefined`, `toBeUndefined`,
`toBeTruthy`, `toBeFalsy`, `toBeNaN`, `toBeTypeOf`, `toBeCloseTo`, `toBeGreaterThan`,
`toBeGreaterThanOrEqual`, `toBeLessThan`, `toBeLessThanOrEqual`, `toThrow` / `toThrowError`,
`toMatchSnapshot`, and the mock matchers `toHaveBeenCalled`, `toHaveBeenCalledTimes`,
`toHaveBeenCalledWith`, `toHaveBeenLastCalledWith`. Every matcher except `toMatchSnapshot` is
also available under `.not`.

Asymmetric matchers: `expect.any(type)`, `expect.anything()`, `expect.objectContaining(o)`,
`expect.arrayContaining(a)`, `expect.stringMatching(s)`.

Assertion contracts: `expect.assertions(n)` and `expect.hasAssertions()` fail the test if the
body made the wrong number of `expect(...)` calls — useful when an early return would otherwise
look like a pass.

Mocks expose both spellings: `m.mockClear()` / `m.mockImplementation(fn)` /
`m.mockReturnValue(v)` / `m.mockResolvedValue(v)` on the mock itself, and `m.mock.calls` /
`m.mock.callCount` for inspection.

## `tish:assert`

Callable `assert(value)` plus `ok`, `strictEqual`, `deepStrictEqual`, `partialDeepStrictEqual`,
`match`, `doesNotMatch`, `throws`, `doesNotThrow`, `rejects`, `doesNotReject`, `fail`,
`ifError`. Aliases: `node:assert`, `node:assert/strict`, bare `assert` / `assert/strict`.

`throws(fn, error?, message?)` validates the thrown value against `error` — a string substring,
a regex, or an `Error`-shaped object whose listed fields must all match. `rejects` /
`doesNotReject` accept a promise **or** a function returning one, as in Node.

## Divergences from Jest / Bun / Node

These are deliberate and permanent unless noted:

| Behavior | Tish | Why |
|----------|------|-----|
| `assert.equal` / `deepEqual` | Strict (aliases of `strictEqual` / `deepStrictEqual`) | Tish has no loose `==` |
| `toBeUndefined()` / `toBeNull()` | Both test for `null`; `toBeDefined()` fails on `null` | Tish has no `undefined`; `null` is the absence value |
| `toEqual` on functions | Identity comparison | Same as Jest |
| Parallelism | None — files and cases run sequentially | Single-threaded VM; `{ concurrent: true }` is accepted and ignored |
| Snapshots | One `.snap` file per snapshot, under `__snapshots__/` | Simpler diffs than a single combined file |
| Backends | `--backend vm` only | Interpreter closures are bound to the `Evaluator`, which is dropped before the collected suite runs; `--backend native` is not implemented |

## CLI flags

| Flag | Meaning |
|------|---------|
| `--backend vm` | Execution backend. Only `vm` runs tests; other values are rejected up front. |
| `--feature NAME` | Restrict platform capabilities, exactly as `tish run --feature` does |
| `-t` / `--test-name-pattern` | Regex filter on the test's full name |
| `--timeout MS` | Per-test timeout (default `5000`) |
| `--bail` | Stop after the first failure |
| `--retry N` | Retry failed tests |
| `--only` | Only run `.only` cases |
| `-u` / `--update-snapshots` | Refresh snapshots |
| `--ci` | Fail on a missing snapshot instead of writing it (also on when `CI` is set) |
| `--watch` | Re-run on file changes |
| `--randomize` / `--seed N` | Shuffle file order |
| `--rerun-each N` | Run the suite N times; **any** failing pass fails the command |
| `--shard i/n` | Run file shard `i` of `n` (1-based) |
| `--tag TAG` | Filter by test `{ tags: [...] }` (repeatable) |
| `--preload FILE` | Evaluate a setup module before each test file, on the same VM instance |
| `--reporter console\|dots` | Console style; both replay full failure diagnostics at the end |
| `--reporter-outfile PATH` | Write JUnit XML |
| `--coverage` | Line coverage via AST instrumentation |
| `--coverage-dir DIR` | Write `lcov.info` (implies `--coverage`; default `coverage/`) |

### Timeouts

`--timeout` is enforced cooperatively: the VM polls the deadline on loop back-edges, so a
runaway loop is aborted and the run continues with the next test. A body blocked inside a
single native call (a synchronous socket read, say) cannot be preempted — that case is caught
by the elapsed-time check once the call returns.

## Config (`package.json`)

```json
{
  "tish": {
    "test": {
      "root": ["test", "src"],
      "timeout": 10000,
      "preload": ["./test/setup.tish"]
    }
  }
}
```

`root`, `timeout`, and `preload` apply when the CLI roots are still the default `.`.

## Isolation

Each test file gets a fresh VM. Between files the runner clears pending throws and the mock
registry. It does **not** reset timers, HTTP routes, or sockets in-process — suites that leak
host state should run in separate processes (a separate CI job or worker).

## Coverage

`tish test --coverage` instruments `.tish` ASTs inside `crates/tish_test` (not the VM), then
runs on the VM. It writes `coverage/lcov.info` plus a per-file summary. Only files actually
loaded by a test are counted, and test/spec files are excluded from the denominator; a run that
executed no non-test source says so rather than reporting 100%. This is **not** a substitute
for c8 (or similar) on compiled JS.

## Ecosystem CI layers

| Layer | Command |
|-------|---------|
| Language goldens | `just test` / cargo-nextest |
| Runner regressions | `cargo test -p tishlang --test test_runner` (+ `.github/workflows/test-runner.yml`) |
| App language suites | `tish test` in the package |
| JS-emit / browser parity | `tish build --target js` + `node` (package-specific) |
| Parity / test262 / perf | existing scripts/workflows |

## Host wiring

Everything this feature adds outside [`crates/tish_test`](../crates/tish_test):

| File | Why |
|------|-----|
| [`Cargo.toml`](../Cargo.toml) | Workspace member for `crates/tish_test` |
| [`crates/tish/Cargo.toml`](../crates/tish/Cargo.toml) | Optional `tishlang_test` + `test-runner` feature; `full` enables it |
| [`crates/tish/src/cli_help.rs`](../crates/tish/src/cli_help.rs), [`main.rs`](../crates/tish/src/main.rs) | `Commands::Test` / `TestArgs` and dispatch |
| [`crates/tish_compile/src/resolve.rs`](../crates/tish_compile/src/resolve.rs) | Aliases `test` / `assert` / `assert/strict` and their `node:` forms; allows a **default** import only for `tish:assert` |
| [`crates/tish_vm/src/vm.rs`](../crates/tish_vm/src/vm.rs) | `LoadNativeExport` consults `register_native_module` for all specs; `JumpBack` polls the execution deadline |
| [`crates/tish_core/src/lib.rs`](../crates/tish_core/src/lib.rs) | The cooperative deadline itself (disarmed on every path but `tish test`) |
| [`crates/tish_eval/src/eval.rs`](../crates/tish_eval/src/eval.rs) | Same deadline poll on interpreter loop back-edges |
| [`stdlib/test.d.tish`](../stdlib/test.d.tish), [`stdlib/assert.d.tish`](../stdlib/assert.d.tish) | Ambient declarations (flat `declare let` form, like `builtins.d.tish`) |

## Deferred

- `--backend interp` and `--backend native`
- True parallel `test.concurrent` and per-file process isolation
- Full Node `TestContext` / TAP / programmatic `run()`
- DOM / HappyDOM / RTL inside the Tish VM

See also the short **Testing** section in [LANGUAGE.md](./LANGUAGE.md).
