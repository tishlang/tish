# Downstream regression suite

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

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

## Acceptance gates (validated each run — not a recorded state)

The PASS/xfail expectations below are **gates**, not frozen verdicts: `regression/downstream/run.sh`
re-checks them on every run (and in CI via `--git-only`), failing (exit 1) the moment a `pass` repo
regresses. The notes are *why* a repo currently sits where it does; the *expected* column in
`repos.tsv` is the contract, and `run.sh` is what proves it — re-run it rather than trusting the
prose. Treat any "→ pass"/"→ xfail" below as the last-observed reason, not a current guarantee.

- **`ffi:` extensions — gate: `run.sh <ffi-repos>` builds clean (expected `pass`).** Rationale: the
  extern-C ABI (`TishValueRef` opaque handles) is unchanged, so these should survive breaking
  embedding-API changes. The in-repo `ffi-mathext` / `ffi-statext` (always present) are the
  always-runnable guard that the C-ABI stays stable — validated on every run, not asserted once.
- **`cargo:` extensions — gate: `run.sh <cargo-repos>` (per-repo `cargo test`/`cargo check` from
  `repos.tsv`).** A repo's `expected` is `pass` once migrated and `xfail` while it's known-broken by
  the `NativeFn`/`Value::String`/`PropMap` changes; the mechanical migration is
  `Arc::from(s)`→`s.into()` in `Value::String`, `ObjectMap`→`Value::object`, `.as_ref()`→`.as_str()`,
  `f()`→`f.call()`, add a `NumberArray` match arm. `run.sh` flags both directions — a `pass` repo that
  breaks (regression, exit 1) and an `xfail` repo that starts passing (warning, time to flip it). Don't
  read "xfail" here as "still broken today"; re-run to confirm.
- **`tish`-program consumers — gate: `run.sh <tish-repos>` runs the repo's own harness against the
  HEAD binary (expected `pass`).** No compile link; affected only by semantic changes
  (div-by-zero→Infinity, insertion-order object keys, …), which are conformance fixes. The current
  check is a smoke run; add a per-repo output-diff for stronger coverage. For language-wide
  perf/conformance posture (not consumer builds), the canonical gates are
  `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL) and `scripts/run_parity_compare.sh`
  (cross-backend) — run those, don't infer status from this list.

## Adding a repo

Append a line to `repos.tsv`. Most tish-ecosystem repos in `~/Projects` are local-only working dirs
(no git remote) → use `local:`; they run locally but are skipped in CI. Give a repo a public git
remote and switch it to `git:` to get CI coverage.

## Calibration snapshot — STALE by default, regenerate with `regression/downstream/run.sh --git-only`

> Snapshot of a past `--git-only` baseline (taken at tish HEAD `ad61d002`); the standings below
> **drift the moment the code or consumers change** and are not a current verdict. Regenerate with
> `regression/downstream/run.sh --git-only` and reconcile against `repos.tsv` before relying on any
> count or per-repo result here. The numbers are illustration of what calibration produced once, not
> ground truth now.

The `tish`-program entries were seeded `expected=pass` (the language surface only *grew* on this branch).
This baseline calibrated them against reality at the time of the snapshot:

**PASS (11) — snapshot, re-derive with `run.sh --git-only`:** `ffi-mathext` `ffi-statext` (in-repo C-ABI guarantee) · `tish-apple` (`tish-apple-common`) ·
`lattish` · `tish-ide-panels` · `tish-learn` · `tish-playground` · `spider3-tish` · `spacekinematics` ·
**`tish-audio`** · **`tish-midi`**. The private `tishlang/*` repos (`tish-apple`, `tish-ide-panels`,
`learn`) **do** clone in CI when a `gh` credential helper is configured — they SKIP only if auth is absent.
`lattish` itself builds clean (the feared `RBrace` indent-parse issue did **not** materialize via `npm test`).

**`tish-audio` + `tish-midi` — lattish apps, tested against the LOCAL WORKSPACE lattish.** Both
`import "lattish"`. The runner wires the **local workspace** lattish (`$TISH/../lattish`) into
`node_modules/lattish` — the npm analog of `rewrite_tish_paths` for the Rust crates — so we test against
the lattish being *developed* next to this tish checkout, **not** the published package (which would only
re-test already-released code). Two enablers made this work:
- **Resolver fix** (`tish_compile/src/resolve.rs`): a bare `import "x"` now resolves `node_modules/x` by
  **directory** (Node semantics), not by requiring the package's `package.json` `name` to equal `x`. npm
  installs a dep under its *key*/directory (here `lattish`), even though the package's own name is
  `@tishlang/lattish`. (CI falls back to the cloned `lattish` entry when no local workspace is present.)
- With a valid workspace lattish, HEAD tish does real work: `tish-audio` → a 1.3 MB `dist/main.js` with
  `createRoot` inlined; `tish-midi`'s 56 `src/` files transpile clean.

Because it tests the *working tree*, this immediately catches WIP breakage in lattish: a brace-unbalanced
`Lattish.tish` (uncommitted stray `}`) makes both apps fail — which is the suite doing its job, not a
false alarm. Fix the workspace lattish and they go green.

**xfail — snapshot, re-confirm with `run.sh --git-only` (an xfail that now passes is a warning to flip):**
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

### spider3-tish & spacekinematics (snapshot — re-derive counts with `run.sh --git-only`)

The transpile counts below are a point-in-time snapshot and go stale as the repos or tish change;
they are illustrative of one calibration run, not a current pass count. Re-run `run.sh --git-only`
to get today's numbers.

- **`spider3-tish`** (`schlopai/spider-gwen-webgpu`) — all 10 `.tish` transpile clean on HEAD → `pass`
  (transpile-all-`.tish` smoke, excluding `node_modules`/`vendor`/`dist`).
- **`spacekinematics`** (`spacedevin/solar-system-webgpu`) — 35/39 `.tish` transpile clean → `pass`. The
  smoke **excludes `packages/sgp4-wasm`**: those 4 files hit `Circular import detected` (satrec ↔ sgp4),
  which is a **PRE-EXISTING** module-resolver behavior (present on `main` at the merge-base, `tish_compile/
  src/resolve.rs`) — *not* a feature/perf regression. Excluding sgp4-wasm keeps the entry honest: it
  guards the 35 compilable files against feature/perf breaks. If circular imports become supported, drop
  the exclusion.
