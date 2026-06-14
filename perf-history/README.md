# perf-history — performance over time

Structured records of the bundled perf suite (`scripts/run_performance_suite.sh`), one TSV per
commit, so tish performance is tracked **across history** instead of measured once and discarded.
This is the durable companion to the typed-vs-untyped gauntlet (`docs/perf-typed-vs-untyped-baseline.md`),
which only validates the typing A/B at a single commit.

## What's recorded

Each `<commit-date>-<shortsha>.tsv` holds, per runtime:

- **`bundle`** — the whole-program `tests/main.tish` (5-run avg) on every backend: `vm`, `interp`,
  `rust` (native AOT), `cranelift`, `llvm`, `wasi`, and the JS engines `node`, `bun`, `deno`, `qjs`.
  This is the reliable signal.
- **`test`** — per-file micro-benchmarks (vm / interp / node columns the suite prints per file).
  These are ~13 ms of fixed process-startup overhead each, so only the compute-heavy ones carry
  signal; `perf_compare` floors them out by default (`--min-ms`).

Format (tab-separated): a `# meta` header (commit, date, tag, os, runtimes, runs) then
`scope <TAB> name <TAB> runtime <TAB> ms <TAB> status` rows.

## Tools

```bash
# Record the current checkout (runs the suite, writes perf-history/<date>-<sha>.tsv):
scripts/perf_record.sh                       # all runtimes, 5 runs
scripts/perf_record.sh --runtimes vm,interp,rust,node --runs 5

# Seed/backfill a record from a run you already captured, stamped with a given ref:
scripts/perf_record.sh --from-log run.log --ref v2.2.0

# Diff two records — reports regressions/improvements over time:
scripts/perf_compare.sh perf-history/<old>.tsv perf-history/<new>.tsv
```

`perf_compare` derives a **noise floor** from the JS engines' drift (they run identical `.js` at both
commits, so any move in *their* numbers is pure machine variance) and flags a tish backend only when
it moves more than that floor. Its exit status is `1` if any tish backend regressed — usable as a gate.

## How it accrues

`.github/workflows/perf-history.yml` records a point on every push to `main` (core runtimes, for
speed/stability on shared runners) and commits it here with `[skip ci]`. Run `perf_record.sh` locally
with the full runtime set on a quiet machine for the highest-fidelity points (e.g. at each release).

> Compare like with like: records carry their `os`/runner in the meta header. CI-runner timings are
> noisier than a quiet local machine — trends within one source are meaningful; absolute ms across
> different machines are not.
