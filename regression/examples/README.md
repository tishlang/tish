# Example-app regression suite

**RUNS** (not just builds) every example app we ship, against the tish checkout you're on (HEAD),
and asserts observable behavior. Companion to [`regression/downstream`](../downstream/README.md):
that suite guards external *consumers*; this one guards the examples in this repo.

A transpile smoke only proves "it parses". This suite proves "it **works**":

- **`run`** examples are executed and their **stdout** is asserted.
- **`serve`** examples (HTTP servers) are started, **probed over HTTP**, asserted, and shut down.
- **`lattish`** examples are built to JS, **mounted in jsdom**, and their **rendered DOM** is asserted.

## Why this exists

The downstream suite caught a real **tish JSX-lexer regression** — filed as
[tishlang/tish#108](https://github.com/tishlang/tish/issues/108). After a nested child element, a
text run is lexed as code, so a reserved keyword sitting in ordinary markup prose fails to parse:

```jsx
<div><span>x</span> as JSON</div>   // ✗ Parse error: Unexpected token in JSX children: As
<div>download as JSON</div>          // ✓ builds fine (no preceding element)
```

Affected after a child element: `as` `in` `if` `return` `let` (… any reserved word); fine for
non-keywords and in text-only children. It bit `tish-audio` (real prose: *"…download as JSON."*
after inline `<span>`s) and would have shipped silently. **Running the examples is the guard** that
turns a class of frontend regressions into a red build.

## Findings from the first sweep (tish HEAD around `a3d8747`)

Every example was exercised the right way for its kind. Result: **no frontend regressions** — all
50 example `.tish` files parse + transpile clean. Concretely:

- **All `run` / `serve` / `lattish` examples behave correctly** when their port is free / deps present.
- The 9 **lattish** examples render correctly against the **workspace lattish** (wired in like
  downstream does — we test the lattish being *developed* beside this checkout, not the published
  package). This is also the end-to-end proof that the `createRoot(container, host)` pluggable-host
  change (lattish #4) didn't regress the default DOM path.
- **Native-only** examples (`tish:http/fs/mlx/metal`, `ffi:`) parse clean; `async-await`,
  `json-file-edit`, `mdx-docs` build to a native binary. `matmul`'s GPU variants fail only at the
  **backend** (`mlx`/`metal` toolchain), not the frontend — tracked as `xfail`/env-gated, not a regression.

### Calibration gotchas baked into the manifest (so they don't read as failures)

- `new-expression` prints `sampleRate = 48000` on this machine (audio-device dependent) — assert the
  stable `sampleRate =`, not a specific rate.
- The `http-*` servers honor `$PORT`; `json-api`/`echo-server`/`counter-api` **hardcode 3000**. The
  runner relocates the former to a free port (`auto`) and **skips** the latter if 3000 is held by a
  foreign process (e.g. a local Next.js dev server) — a port conflict is a SKIP, never a regression.
- `lat-app` renders a "Sign up" form; `lat-06-effects`' `useEffect` does async work that rejects in
  jsdom — the render harness asserts the **synchronous** initial render and swallows async rejections.

## Run it

```bash
regression/examples/run.sh                 # deterministic core (run/serve/lattish); env-gated rows SKIP
regression/examples/run.sh --full          # also run net/db/gpu/wasm/npm/prebuild/slow rows
regression/examples/run.sh hello-world json-api lat-01-counter   # a subset by name
regression/examples/run.sh --list          # print the manifest
regression/examples/run.sh --tish /path/to/other/tish            # test a different checkout
```

The suite **fails (exit 1)** only on a real regression — a row marked `pass` that misbehaves. An
`xfail` row that unexpectedly passes is a warning (time to flip it to `pass`). Env/port conflicts
are reported as **SKIP**, not failures.

## How it wires a consumer to tish HEAD

1. Builds the HEAD `tish` binary (`cargo build --release -p tishlang`) and puts it on `PATH`.
2. Copies each example into a scratch dir (so a mutating example like `json-file-edit` never touches
   the repo).
3. For `lattish` examples, rsyncs the **workspace lattish** (`$TISH/../lattish`) into
   `node_modules/lattish` — the npm analog of downstream's Cargo path-rewrite — then builds + mounts
   it in jsdom (jsdom is resolved from the workspace lattish's `node_modules`).
4. Runs the example and asserts per its kind.

## Manifest — `examples.tsv`

Tab-separated: `name  dir  entry  kind  features  check  expect  tags  expected`

| field | values |
|---|---|
| `dir` | `examples/foo` (this repo) · `@lattish:PATH` (a path under `../lattish`) |
| `entry` | the `.tish` to run/build, relative to `dir` |
| `kind` | `run` (executes + exits) · `serve` (HTTP server) · `lattish` (build→jsdom render) |
| `features` | comma list for `--feature` (or `-`) |
| `check` | for `serve`: `METHOD:PORT:PATH` (PORT may be `auto` if the example honors `$PORT`). `-` otherwise |
| `expect` | substring that must appear (stdout · HTTP body · rendered DOM text) |
| `tags` | env needs; any of `net db gpu wasm npm prebuild native slow vite` makes the row SKIP unless `--full` (`mutates` is informational) |
| `expected` | `pass` (must stay green — a failure is a regression) · `xfail` (known-broken pending env/migration) |

## Adding an example

Append a line to `examples.tsv`. Pick the `kind`, give a concrete `expect` (quote a literal the app
prints / serves / renders), and tag any environment it needs so CI skips it cleanly. If it needs
network, a database, a GPU, a prebuild step, or an `npm install`, tag it so the default run stays
deterministic; document anything subtle here (see the calibration gotchas above).
