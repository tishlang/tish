# Testing in Tish

Native test runner: **`tish test`**. API is Bun/Jest-shaped via `tish:test`, plus Node-compatible `assert` via `tish:assert` / `node:assert` / `node:assert/strict`.

## Tish is not JavaScript

**`tish test` runs on the Tish VM or interpreter (`--backend vm|interp`). It does not execute JavaScript.**

| Runner | What it proves |
|--------|----------------|
| `tish test` / `tish run` | Language + runtime semantics on the **Tish** VM/interp |
| `tish build --target js` + **node** (or bun) | Behavior of **compiled JS emit** under a JS engine |
| Vite / jsdom / browser | Host integration of that JS emit |

Moving a suite from `build --target js && node` → `tish test` changes the subject under test. Keep Node dual-runs for JS-emit / browser / DomHost contracts. Host-wiring audit: [test.md](./test.md).

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

## Bun / Jest surface (`tish:test`)

| Export | Role |
|--------|------|
| `describe` / `suite` (+ `.skip` / `.only` / `.each`) | Suite grouping (collect-then-run) |
| `test` / `it` (+ `.skip` / `.only` / `.todo` / `.failing` / `.skipIf` / `.each`) | Cases |
| `expect(x).toBe` / `toEqual` / … | Matchers (incl. mock + snapshot matchers) |
| `beforeAll` / `afterAll` / `beforeEach` / `afterEach` | Hooks (`before` / `after` aliases) |
| `mock` / `spyOn` / `clearAllMocks` / `restoreAllMocks` | Doubles |

Discovery globs: `*.test.tish`, `*_test.tish`, `*.spec.tish`, `*_spec.tish`.

Aliases: `node:test` and bare `test` normalize to `tish:test`.

## Node assert (`tish:assert`)

Callable `assert(value)` plus `ok`, `strictEqual`, `deepStrictEqual`, `throws`, `match`, and related Node methods. `equal` / `deepEqual` are **strict** aliases (Tish has no loose `==`).

Aliases: `node:assert`, `node:assert/strict`, bare `assert` / `assert/strict` → `tish:assert`.

## Suggested layout (optional)

Packages typically use `test/` or `src/**/*.test.tish`. Folders like `unit/` / `regression/` / `integration/` are conventions only — discovery does not require a special in-repo tree.

## CLI flags (common)

| Flag | Meaning |
|------|---------|
| `--backend vm\|interp` | Execution backend (default `vm`). Other values are rejected. |
| `-t` / `--test-name-pattern` | Regex filter on test full name |
| `--timeout` | Per-test timeout ms (default `5000`; enforced after the body returns, and while awaiting promises) |
| `--bail` | Stop after first failure |
| `--retry N` | Retry failed tests |
| `--only` | Only run `.only` cases |
| `-u` / `--update-snapshots` | Refresh snapshots |
| `--watch` | Re-run on file changes |
| `--randomize` / `--seed N` | Shuffle file order |
| `--rerun-each N` | Run the suite N times |
| `--shard i/n` | Run file shard `i` of `n` (1-based) |
| `--tag TAG` | Filter by test `{ tags: [...] }` (repeatable) |
| `--preload FILE` | Evaluate setup module before each test file |
| `--reporter console\|dots` | Console style |
| `--reporter-outfile PATH` | Write JUnit XML |
| `--coverage` | Line coverage via AST instrumentation (vm + interp) |
| `--coverage-dir DIR` | Write `lcov.info` (implies `--coverage`; default `coverage/`) |

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

`root`, `timeout`, and `preload` are applied when CLI roots are still the default `.`.

## Isolation

Between files the runner clears pending throws. It does **not** reset timers, HTTP routes, or sockets in-process. Suites that leak host state should run in separate processes (separate CI job / worker).

## Dogfood / migration

Replace local `assertEq` / `throw "FAIL"` / `check()` helpers with `import { expect } from "tish:test"` or `import assert from "node:assert/strict"`. Rename language-semantics suites to `*.test.tish` and run `tish test`.

| Package | Onto `tish test` | Keep on Node (or similar) |
|---------|------------------|---------------------------|
| **scii** | All suites (already `tish run`) — migrated to `tish:test` + `node:assert/strict` | — |
| **deck** | `smoke.test.tish` + `conformance.test.tish` (language) | `conformance.mjs`, c8 on `dist/deck.js`, `test:js-smoke`, player, rust |
| **deckard** (`tish-midi`) | `test/*.test.tish` prepared | **Primary CI:** JS-emit batch + ratchet. Native VM blocked by JS `undefined` in product sources |
| **lattish** | `jsx-runtime.test.tish` prepared | **Primary CI:** DomHost (`run-tests.mjs` + jsdom) + Vite HMR. Native VM blocked by JS `undefined` in `Lattish.tish` |

`stdlib/test.d.tish` and `stdlib/assert.d.tish` are documentation-shaped stubs and are **not** wired into LSP/check yet (`declare module` is unsupported).

## Ecosystem CI layers

| Layer | Command |
|-------|---------|
| Language goldens | `just test` / cargo-nextest |
| App language suites | `tish test` in the package |
| JS-emit / browser parity | `tish build --target js` + `node` (package-specific) |
| Parity / test262 / perf | existing scripts/workflows |

## Deferred

- Full Node `TestContext` / TAP / programmatic `run()`
- DOM / HappyDOM / RTL inside the Tish VM
- `--backend native` (build+exec) for `tish test`
- True parallel `test.concurrent` (single-threaded VM today)

## Coverage

`tish test --coverage` instruments `.tish` ASTs inside `crates/tish_test` (not the VM), then runs on **vm** or **interp**. Writes `coverage/lcov.info` and a per-file summary. Test/spec files are excluded from the report denominator. This is **not** a substitute for c8 (or similar) on compiled JS.

See also the short **Testing** section in [LANGUAGE.md](./LANGUAGE.md) and the host audit in [test.md](./test.md).
