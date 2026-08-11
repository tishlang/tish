# Host wiring for `tish test` (audit)

This document lists **every host change outside [`crates/tish_test`](../crates/tish_test)** that lands with the native test runner, why it exists, and how dogfood packages split **language semantics** vs **compiled JS**.

## Critical: Tish is not JavaScript

**`tish test` runs Tish on the bytecode VM or interpreter (`--backend vm|interp`). It does not execute JavaScript.**

| Runner | What it proves |
|--------|----------------|
| `tish test` / `tish run` | Language + runtime semantics on the **Tish** VM/interp |
| `tish build --target js` + **node** (or bun) | Behavior of **compiled JS emit** under a JS engine |
| Vite / jsdom / browser | Host integration of that JS emit |

Implications:

1. Tish cannot run JS tests natively. JS-side suites need an **external** binary (`node`, etc.).
2. Moving a suite from `build --target js && node` → `tish test` **changes the subject under test**. Pass/fail may diverge.
3. Classify every suite before migrating:
   - **Language semantics** → prefer `tish test`
   - **JS-emit / browser / Node-host contract** → keep (or add) a Node dual-run; never drop the emit leg if that is what CI was guarding
4. Do **not** add `tish_compile_js` shims so that “tests compile to JS.” That muddies the model.

User-facing runner/API docs: [testing.md](./testing.md).

---

## Verdict legend

| Tier | Meaning |
|------|---------|
| **KEEP** | Required for `tish test` / `tish:test` / `tish:assert` on default `--backend vm` |
| **REVERT** | Not required for native testing (removed from this feature’s host surface) |

---

## KEEP — load-bearing host wiring

| File | Why |
|------|-----|
| [`Cargo.toml`](../Cargo.toml) | Workspace member for `crates/tish_test` |
| [`Cargo.lock`](../Cargo.lock) | Lockfile for `tishlang_test` + `notify` (`--watch`) |
| [`crates/tish/Cargo.toml`](../crates/tish/Cargo.toml) | Optional `tishlang_test` + `test-runner` feature; `full` enables it |
| [`crates/tish/src/cli_help.rs`](../crates/tish/src/cli_help.rs) | `Commands::Test` / `TestArgs` |
| [`crates/tish/src/main.rs`](../crates/tish/src/main.rs) | Dispatch to `run_tests` / `run_tests_watch`; `"test"` on implicit-run deny list |
| [`crates/tish_compile/src/resolve.rs`](../crates/tish_compile/src/resolve.rs) | Aliases `test` / `assert` / `assert/strict` and `node:` forms to `tish:test` / `tish:assert`; marks them builtin natives; allows **default** import only for `tish:assert` |
| [`crates/tish_vm/src/vm.rs`](../crates/tish_vm/src/vm.rs) | `LoadNativeExport` consults `register_native_module` for **all** specs (not only `cargo:` / `ffi:`). The test loader registers `tish:test` / `tish:assert` that way |
| [`crates/tish_test/**`](../crates/tish_test) | The feature |
| [`docs/LANGUAGE.md`](./LANGUAGE.md) | Short Testing section |
| [`docs/testing.md`](./testing.md) | Canonical runner / API / CLI guide |

---

## Docs / stdlib (ship with the feature)

| File | Note |
|------|------|
| [`stdlib/test.d.tish`](../stdlib/test.d.tish), [`stdlib/assert.d.tish`](../stdlib/assert.d.tish) | Documentation-shaped stubs. **Not wired into LSP/check today** (`declare module` is not parsed). |
| This file (`docs/test.md`) | Host-change audit + execution-model split |

---

## REVERT — intentionally not part of the host surface

| Change | Why reverted |
|--------|----------------|
| [`crates/tish_compile_js/src/codegen.rs`](../crates/tish_compile_js/src/codegen.rs) remaps for `tish:test` / assert | Reinforces a false “JS test runner” story. Native suites run on the VM; JS emit parity suites use Node’s own assert after compile |
| `justfile` `test-suite` recipe | Convenience only |
| create-tish-app http-hello `"test"` script + sample `main.test.tish` | Scaffold DX; real dogfood is scii / deck / deckard / lattish |

---

## Dogfood packages (semantics vs JS-emit)

| Package | Onto `tish test` | Must stay on external Node (or similar) |
|---------|------------------|----------------------------------------|
| **scii** | Migrated: all `*.test.tish` use `tish:test` + `node:assert/strict` (`npm test` → `tish test`) | None material |
| **deck** | Tish conformance + language smoke | `conformance.mjs`, `coverage.mjs`+c8, JS smoke, player `node:test`, rust |
| **deckard** (checkout `tish-midi`) | `test/*.test.tish` prepared (`tish:test` + assert) | **Primary CI:** JS-emit batch + ratchet. Native VM runs are **blocked** today because product sources reference JS `undefined` (works under emit+node; fails on the Tish VM). |
| **lattish** | `jsx-runtime.test.tish` prepared | **Primary CI:** DomHost (`run-tests.mjs` + jsdom + `jsx-runtime.jsemit.tish`) and Vite HMR. Native VM blocked by JS `undefined` throughout `Lattish.tish` |

Never delete a JS-emit CI leg solely because native tests were added.

### Known gaps

- Mocks are not cleared between files in `reset_between_files`
- No process-per-file isolation in the runner; no timer/HTTP route reset between files
- Stdlib ambient stubs are not loadable by the typechecker yet
- Product sources that use JS `undefined` (deckard, lattish) cannot run on the Tish VM until cleaned — that is a **language purity** issue, not a missing Node runner inside `tish test`
