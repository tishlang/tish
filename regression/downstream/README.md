# Downstream regression suite

Builds and tests **tish consumers** against the tish checkout you're on (HEAD), so an API or
semantic change to tish is caught against the projects that depend on it — instead of discovering
weeks later that `tish-pg` / `tish-callbacks` no longer compile.

This exists because the `feature/perf` branch changed the embedding API (`NativeFn` →
`Arc<dyn Callable>`, `Value::String(Arc<str>)` → `ArcStr`, new `Value::NumberArray`, `PropMap` is now
an `IndexMap` struct) and that silently broke every `cargo:` extension. See
`docs/perf-branch-breaking-changes.md`.

## Run it

```bash
# everything in the manifest (uses your local ~/Projects checkouts for local: repos)
regression/downstream/run.sh all

# one or a few
regression/downstream/run.sh tish-pg ffi-mathext

# CI mode: only git:/self: sources (local-only repos are skipped on a runner)
regression/downstream/run.sh --git-only

# test a consumer against a different tish checkout
regression/downstream/run.sh tish-callbacks --tish /path/to/other/tish
```

The suite **fails (exit 1)** only on a real regression — a repo marked `pass` that fails to
build/test. A repo marked `xfail` that *unexpectedly passes* is reported as a warning (time to flip
it to `pass`).

## How it wires a consumer to tish HEAD

For each repo the runner (1) sources it — `git clone` for `git:`, an rsync copy for `local:`/`self:`
(never mutates your checkout) — then (2) rewrites every `path = ".../crates/tish_*"` dependency in
its `Cargo.toml`s to point at the tish-HEAD crates under test (robust for path-dep consumers, no
version-match issues), and (3) runs the manifest's build/test command. `tish`-kind repos get the
HEAD `tish` binary on `PATH` and run their `.tish` programs.

## Manifest — `repos.tsv`

Tab-separated: `name  source  subdir  kind  cmd  expected`

| field | values |
|---|---|
| `source` | `git:URL@REF` · `local:PATH` (dev machine) · `self:SUBDIR` (a subdir of the tish checkout) |
| `kind` | `rust` (cargo: extension/embedder) · `ffi` (extern-C extension) · `tish` (runs `.tish` programs) |
| `expected` | `pass` (must stay green — a failure is a regression) · `xfail` (known-broken pending migration) |

## Current state (on `feature/perf`)

- **`ffi:` extensions → `pass`.** The extern-C ABI (`TishValueRef` opaque handles) is unchanged, so
  these survive the breaking API change. The in-repo `ffi-mathext` / `ffi-statext` (always present)
  are the guarantee that the C-ABI stays stable.
- **`cargo:` extensions → `xfail`.** Broken by the `NativeFn`/`Value::String`/`PropMap` changes
  (confirmed by compiling `tish-pg` and `tish-callbacks`). They need a mechanical migration
  (`Arc::from(s)`→`s.into()` in `Value::String`, `ObjectMap`→`Value::object`, `.as_ref()`→`.as_str()`,
  `f()`→`f.call()`, add a `NumberArray` match arm). Once a repo is migrated, flip it to `pass` — the
  suite will then guard it against future breakage.
- **`tish`-program consumers → `pass`.** No compile link; affected only by semantic changes
  (div-by-zero→Infinity, insertion-order object keys, …), which are conformance fixes. The current
  check is a smoke run; add a per-repo output-diff for stronger coverage.

## Adding a repo

Append a line to `repos.tsv`. Most tish-ecosystem repos in `~/Projects` are local-only working dirs
(no git remote) → use `local:`; they run locally but are skipped in CI. Give a repo a public git
remote and switch it to `git:` to get CI coverage.

## First-run calibration (DONE — `--git-only` baseline, tish HEAD `ad61d002`)

The `tish`-program entries were seeded `expected=pass` (the language surface only *grew* on this branch).
The first `--git-only` baseline calibrated them against reality:

**PASS (11):** `ffi-mathext` `ffi-statext` (in-repo C-ABI guarantee) · `tish-apple` (`tish-apple-common`) ·
`lattish` · `tish-ide-panels` · `tish-learn` · `tish-playground` · `spider3-tish` · `spacekinematics` ·
**`tish-audio`** · **`tish-midi`**. The private `tishlang/*` repos (`tish-apple`, `tish-ide-panels`,
`learn`) **do** clone in CI when a `gh` credential helper is configured — they SKIP only if auth is absent.
`lattish` itself builds clean (the feared `RBrace` indent-parse issue did **not** materialize via `npm test`).

**`tish-audio` + `tish-midi` — lattish apps, made to PASS via dep provisioning.** Both `import {createRoot}
from "lattish"`. Their real lattish dependency is a **sibling `file:` link** (tish-audio:
`"lattish":"file:../tish/lattish"`) that doesn't exist in an isolated CI clone — and lattish *publishes* as
`@tishlang/lattish` while the apps import bare `lattish`. The suite bridges this with the **`provision`
column** (see manifest header): it clones lattish into the consumer's `node_modules/lattish` and normalizes
that copy's `package.json` `name` to `lattish` so the bare import resolves. HEAD tish then does **real
work** — `tish-audio` produces a 1.3 MB `dist/main.js` with lattish's `createRoot` inlined; `tish-midi`'s
56 `src/` files (no native `tish:` imports; `services/` daemons excluded) all transpile clean. This is the
mechanism for any lattish-consuming app — add a `provision` entry rather than marking it `xfail`.

**xfail — calibrated this run:**
- **`tish-polars`** — heavy polars `cargo check`, broken by the `Value`/`Callable` API change (cargo: ext).

**SKIP in CI (local-only):** the `cargo:` extensions (`tish-pg`, `tish-callbacks`, …), `tish-unity` (ffi),
`tish-tailwind` — run them with `regression/downstream/run.sh all` on a dev machine where `~/Projects` has
the working copies.

When refreshing this baseline, flip any repo that is **pre-existing-broken on HEAD** (broken for reasons
unrelated to this branch) to `xfail` so the suite tracks real regressions, not standing issues. Repos
whose build needs feature flags or heavy frontend toolchains may fail for non-tish reasons — refine the
`cmd` per repo before trusting the result.

`tish-apple`: only the cross-platform `tish-apple-common` crate is checked (verified clean on HEAD).
Its `tish-macos` / `tish-ios` crates are macOS-only and heavily use `Value` (likely hit the
`ArcStr`/`PropMap` breaks) — add a macOS-gated entry (`cargo check -p tish-macos`) to cover them.

### spider3-tish & spacekinematics (calibrated)

- **`spider3-tish`** (`schlopai/spider-gwen-webgpu`) — all 10 `.tish` transpile clean on HEAD → `pass`
  (transpile-all-`.tish` smoke, excluding `node_modules`/`vendor`/`dist`).
- **`spacekinematics`** (`spacedevin/solar-system-webgpu`) — 35/39 `.tish` transpile clean → `pass`. The
  smoke **excludes `packages/sgp4-wasm`**: those 4 files hit `Circular import detected` (satrec ↔ sgp4),
  which is a **PRE-EXISTING** module-resolver behavior (present on `main` at the merge-base, `tish_compile/
  src/resolve.rs`) — *not* a feature/perf regression. Excluding sgp4-wasm keeps the entry honest: it
  guards the 35 compilable files against feature/perf breaks. If circular imports become supported, drop
  the exclusion.
