================================================================================
  OPTIMIZATION PASS (2026-06) — slots + numeric JIT + object layout
================================================================================
Three shared changes to the VM / compiler / core (no per-test hacks; every backend
that embeds the VM benefits). darwin-arm64, release, `tish run --backend vm`,
5-run avg via `./scripts/run_performance_suite.sh --release`.

  test                   before  after   Node   change
  core/array_stress        227     58     41    463% -> 141%  (numeric JIT: find/some/every
                                                              ran 96ms of callbacks, now 7ms
                                                              of native f64)
  core/benchmark_granular  113     97     36    slot-based call frames
  core/object_stress       111    105     36    insertion-order PropMap (small objs inline)
  core/new_features_perf    76     73     36
  bundle (whole program)   471    313     75    vm; cranelift 511->333, llvm 502->337
  46 startup-bound tests   ~unchanged — all beat Node (tish ~12ms cold start vs Node ~35ms)

Changes:
  RC2  tish_bytecode/compiler.rs + tish_vm/vm.rs : self-contained leaf functions compile to
       slot-indexed locals (LoadLocal/StoreLocal) on a bare Vec<Value> call frame — no per-call
       scope hashmap, no name lookups.
  JIT  tish_vm/jit.rs (cranelift, non-wasm) : straight-line numeric slot fns compile to native
       f64 code, called from `Call` when args are numbers (VM fallback otherwise). Decisive for
       array_stress's find/some/every. Carries to the cranelift/llvm native backends.
  RC3  tish_core/value.rs : object props use PropMap (inline <=8 keys, no separate hashmap alloc;
       IndexMap for large so JSON.parse stays O(1)); insertion order now matches JS/Node.

Ceiling: object/dynamic-heavy tests (object_stress, benchmark_granular, new_features_perf) stay
~2x Node — V8's JIT compiles property access to ~free. Beating Node there needs object/dynamic
native codegen (hidden classes); the numeric JIT is slice 1 of that. Dropping `send-values`
(Arc<Mutex>) would add ~15-19% on those but is RULED OUT: the tish-techempower TFB benchmark
(`tish build --native-backend rust` + tiny_http + Postgres) depends on it for multi-threaded
handler dispatch + tish-pg pipelining, so removing it would regress HTTP/DB throughput — and it
would not cross the ceiling anyway.

Note: object props now use `PropMap` (tish_core/value.rs) with ZERO-ALLOC concrete iterators
(`PropMapIter`/`Keys`/`Values`) — `json.rs` iterates `strings.keys()` per response object, so a
`Box<dyn Iterator>` there would heap-allocate per request on the TFB JSON/db hot path. Keep them
concrete.
================================================================================


================================================================================
  FOLLOW-UP (2026-06) — JIT correctness, interp parity, run-vs-build, regression test
================================================================================
Diagnostic: tests/core/jit_probe.{tish,js} — 10 isolated op categories (reduce, map ternary,
filter mod, bitwise, Math, inline loop, fib(30), string concat, array index, find) with
per-section timings. On-demand ONLY — excluded from the perf suite/bundle (its 4M-iteration loop
would dominate). Run: `target/release/tish run --backend vm tests/core/jit_probe.tish`.

`tish run` (vm) vs `tish build` (vm-embed) are CONSISTENT — both run the VM, so the numeric JIT
(incl. the new `mod`) fires identically. `tish build --native-backend cranelift` of jit_probe
matches `tish run --backend vm` section-for-section (reduce 25 vs 32, filter-mod 23 vs 20, find
11 vs 11; non-JIT loop/recursion/array-index ~750-900 on both), the cranelift build only slightly
slower on bytecode-deserialize startup. So the JIT roadmap lifts run AND the cranelift/llvm/wasi
builds. (Remaining gaps vs Node: non-JIT inline loops/recursion/array-index ~120x — these point at
a baseline/loop JIT, the next roadmap slice.)

Fixes (correctness; regression test tests/core/jit_regression.tish asserted on ALL 6 backends):
  - JIT bool-boxing: `map(x => x === c)` was returning Number [1,0,..] not Bool [true,false,..].
    jit.rs now tracks result kind (NumericFn.result_bool); vm.rs boxes Bool vs Number accordingly.
    Silent miscompilation — caught by jit_probe, not the prior corpus.
  - JIT `mod`: added `%` to the numeric JIT (`a - trunc(a/b)*b`, matches Rust f64 % and Node).
  - Interp object order: tish_eval used AHashMap (hash-order Object.keys) + alphabetically-sorted
    JSON, diverging from Node and the VM. EvalObjectData.strings is now an insertion-ordered
    IndexMap and the JSON key-sort is removed — interp == vm == node. (RC3 fixed tish_core only;
    the interpreter lagged, and the old corpus never exercised multi-key object order.)
================================================================================


================================================================================
  HTTP THROUGHPUT (2026-06) — tish vs Node, single- vs multi-worker
================================================================================
The single-shot core/* tests never exercise the multi-worker HTTP server — the whole reason
`send-values` / prefork exist. `scripts/run_http_perf.sh` (`just perf-http`) fills that gap: it
builds `tests/http/server.tish` (rust backend), drives `oha` load at tish AND an equivalent
`node:http` server, across {1 worker, N workers} x {/plaintext, /json}, DB-free (isolates the
HTTP server + per-request VM handler dispatch). Server and load run as SEPARATE PROCESSES (never
self-fetch — you can't measure a port-holder from inside its own event loop): `--serve tish|node`
is the server process (blocks), `--url URL` is the external load process; the no-arg form
orchestrates both for a quick local comparison. HTTP/WS tests are EXCLUDED from the single-shot
suite (run_performance_manual.sh + the bundle) — opening a port / doing an outbound fetch can't be
timed in a one-process harness.

darwin-arm64, `oha -c128`, 14 cores, req/s (higher better) / p50 ms:
  engine       /plaintext       /json
  tish  w=1    125k / 1.02ms    121k / 1.05ms
  tish  w=14   124k / 1.02ms    119k / 1.07ms    <- no scaling on macOS (see below)
  node  w=1     95k / 1.35ms     93k / 1.38ms
  node  w=14   154k / 0.82ms    153k / 0.83ms

- SINGLE worker: tish beats Node by ~33% (faster per request, 1.02 vs 1.35 ms p50). This is the
  apples-to-apples local number — native rust server + cached Date header + Arc<str> bodies pay off.
- macOS MULTI-worker is a NO-OP: BSD SO_REUSEPORT does not kernel-load-balance, so all connections
  funnel to ONE worker (measured: 1 process at 252% CPU, the other 13 at 0%). So tish w=14 ~ w=1.
  The prefork scaling is real on LINUX (the TFB deployment target — the kernel distributes accept()
  across the per-worker sockets); verify multi-worker there, not on macOS. Node's `cluster`
  distributes FDs from the master, which works on macOS, so its multi-worker row scales locally and
  overtakes tish (154k vs 124k) — a platform artifact, not a runtime loss. Rationale:
  `crates/tish_runtime/src/http.rs` concurrency-model doc comment.
================================================================================


================================================================================
  AOT DE-BOXING (2026-06) — rust backend now BEATS V8 on numeric kernels
================================================================================
The rust backend (`tish build --native-backend rust`) already emits native f64 for typed/inferred
numeric locals — but every assignment / `i++` STATEMENT also emitted the expression's *value*
(`Value::Number(s)`), because JS "assignment yields its value". That boxed value is dead in
statement position, but `Value` has a non-trivial `Drop` (other variants hold `Rc`/`Arc`), so LLVM
could not prove it dead and therefore could NOT vectorize/fold the loop. The native f64 math was
free; the per-iteration construct+drop of a dead `Value` was the entire tax.

Fix: `emit_expr_discard` (`tish_compile/src/codegen.rs`) emits only the native side-effect for
assignments, inc/dec, AND compound-assign (`s += x`) in statement position (routed from `ExprStmt`
and the for-loop update). Each covered form was independently ~2.2x node before / now beats it:
40M-iter `s = ...` 48ms, `s += i` 23ms (node ~50ms); 2M-element `a[i] = i*2` 6ms (node ~8ms).

darwin-arm64, `--native-backend rust` vs node (V8), lower = better:
  workload                        before    after    node    result
  40M-iter numeric loop           111 ms    48 ms    52 ms   BEATS V8 (was 2.2x slower)
  matmul 256x256 (typed-local N)  boxed     13 ms    45 ms   3.5x FASTER than V8

Whole corpus byte-identical (full integration suite green) — pure de-boxing, zero semantic change.

KEYSTONE LANDED (M1, dark-shipped behind `TISH_PARAM_NATIVE`): a typed scalar param used to arrive
boxed (`let N = args.get(0).cloned()`; `types.rs:388` `push_fun_param_scope` -> `RustType::Value`),
and ONE boxed param poisoned every index in the hot loop. Now codegen binds a native SHADOW at the
closure top — coerce `args.get(i)` once to f64/bool/String (`from_value_expr`), then register the
native type so the body lowers the param exactly like a native local. Real param-based matmul
(`fn bench(N: number)`), 256x256:
  boxed param (default):   301 ms
  native param (flag on):   15 ms     <- 20x faster, and 3x FASTER than node (45ms)
Identical check value (correct). Flag OFF: whole corpus byte-identical (zero risk). Flag ON: the
entire native corpus still passes (correct output, no panics). Conservative gate: simple params,
native-scalar annotation, no default value. Next: M4 (infer param types from use, so unannotated
`fn bench(N)` benefits too) + harden capture/escape cases, then default-on.
================================================================================


================================================================================
  M5 LANDED (2026-06): native monomorphic calls — recursion now BEATS V8
================================================================================
The call ABI was recursion's whole tax: every `fib(n-1)` went through `value_call(&fib,
&[Value::Number(n-1)])` — clone the closure Value, box the arg, dynamic-dispatch, unbox in the
native shadow — ~30M times for fib(35) (512ms vs node 52ms; the arithmetic was already native).

M5 (dark-shipped behind `TISH_NATIVE_FN`, in `codegen.rs`): for a native-eligible top-level fn (all
params `: number`, `: number` return, native-safe body — a conservative fixpoint `collect_native_fns`
over block/if/return/expr-stmt with native exprs + calls to other eligible fns or 1-arg Math) emit a
parallel free `fn f_native(f64,..)->f64` at top level, and route DIRECT calls to it in
`emit_typed_expr` (`fib(x)` -> `fib_native(x)`), bypassing `value_call` + boxing. The boxed closure
wrapper stays for dynamic use. Result: fib(35) 512ms -> 31ms — BEATS node (48ms), identical result;
flag OFF the corpus is byte-identical, flag ON the whole native corpus still passes. Remaining call
work: closures passed to builtins (array_hof's reduce callback) still box — extend to native closures
/ fused reduce.

M4 param inference + M5 return inference LANDED (dark-shipped behind `TISH_PARAM_INFER`):
`infer.rs::param_infer_program` gives a top-level fn param used PURELY numerically a synthetic
`: number`, and M5's `collect_native_fns` now INFERS a numeric return (`returns_numeric`/
`numeric_shaped`, folded into its existing fixpoint). Together they make IDIOMATIC UNANNOTATED
`function fib(n) {...}` compile to a native `fib_native` — gauntlet `recursion_untyped` 31ms, BEATS
node (51ms). Corpus sound (flag-on passes), flag-off byte-identical.

SOUNDNESS LESSON (a real bug caught + fixed): the numeric-use checker must NOT treat `+` and
comparisons as numeric — `+` is also string concat, `<`/`===` also compare strings — so `first + ":"`
wrongly typed `first` as a number (`rest_params` miscompiled, "expected number" panic). Fix: only
`-`/`*`/`/`/`%`/`**` imply numeric directly; `+`/comparisons require the OTHER operand to be PROVABLY
numeric (`numeric_provable`). Also fixed a stale-cache bug: the native batch cache hashed codegen.rs
+ value.rs but NOT infer.rs, so inference changes served stale binaries.

STILL boxed (the remaining inference pieces): unannotated matmul (483ms) — `let a = []` is a boxed
array, needs array-ELEMENT inference; array_hof (native closures); object_sum (hidden classes, #13).
"Native unannotated code" is a compounding inference effort — params + numeric-returns now done.
================================================================================


================================================================================
  PERF GAUNTLET (2026-06) — tracked targets we currently LOSE, to evolve past
================================================================================
`scripts/run_perf_gauntlet.sh` (`just perf-gauntlet`): compute-only benchmarks (self-timed, process
startup excluded) for the rust backend vs node V8, with per-benchmark correctness checks.
DELIBERATELY includes tests we lose, so each backend change is measured and red turns green over
time. Fixtures: `tests/perf/<name>.tish` (+ `.js` for the native-param ones; the rest are valid in
both tish and node). Baseline (darwin-arm64, rust backend + `TISH_PARAM_NATIVE=1`, min of 2):

  benchmark      tish    node   ratio   verdict / lever to flip it green
  matmul          14ms   16ms   0.87x   PASS  (M1 native params)
  numeric_loop    44ms   47ms   0.94x   PASS  (statement-position de-boxing)
  math_trig       12ms   82ms   0.15x   PASS  native Math intrinsics LANDED (sqrt/sin/...->f64 method)
  string_concat    3ms    3ms   1.00x   PASS  self-append `s=s+x` -> push_str (O(1)); was O(n^2)
  recursion_fib     31ms   48ms   0.65x  PASS  M5 native monomorphic calls (TISH_NATIVE_FN)
  recursion_untyped 31ms   51ms   0.61x  PASS  M4 param + M5 return inference -> native, NO annotations
  array_hof        257ms   29ms   8.9x   FAIL  native closure calls / fused reduce over native arrays
  object_sum     145ms    3ms   48x     FAIL  hidden classes / unboxed objects (task #13)

6/8 beating V8. Four flipped FAIL->PASS across recent passes: math_trig (native Math intrinsics ->
direct `f64` methods), string_concat (String self-append -> `push_str`), recursion_fib (M5 native
calls), and recursion_untyped (M4 param + M5 return inference — IDIOMATIC untyped recursion compiles
native, no annotations). The 2 remaining FAILs are the representation rearchitectures: `array_hof` -> native
CLOSURE calls (the reduce callback still boxes; M5 only covers named top-level fns) or a fused
native reduce; `object_sum` -> hidden classes / unboxed objects (task #13).
================================================================================


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Column meanings (see `LANGUAGE.md` → *Native compile (implementation status)*):
**rust** = transpiled Rust + `tishlang_runtime` (`Value`). **cranelift** / **wasi** = embedded bytecode + **`tishlang_vm`** (not CLIF lowering of opcodes).

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress            10012       26    10011    10010       41       23       36       67     24419%
object_stress             989       40      103      355       34       20       29       66      2908%
benchmark_granular        881       30       98      327       35       21       30       69      2517%
new_features_perf         482       28       63      182       35       19       30       64      1377%
string_methods_perf        30        9       10       22       28       13       23        8       107%
objects_perf               22        9       10       20       28       13       24        7        78%
array_methods_perf         18        8        9       20       28       12       23        9        64%
nested_complex             11        8        8       16       28       13       24        8        39%
template_literals          10        9        9       15       28       12       24        7        35%
arrays                     10        9        9       15       28       13       25        7        35%
objects                    10        8        9       15       28       12       24        7        35%
math                       10        8        9       16       28       12       23        7        35%
higher_order_methods       10        8        8       15       28       13       24        7        35%
const                      10        9        8       15       28       13       24        7        35%
array_methods              10        9        8       15       28       12       23        7        35%
nested_loops               10        9        9       16       29       13       23        7        34%
mutation                   10        9        8       16       29       12       24        7        34%
rest_params                 9        8        8       15       27       13       23        7        33%
compound_assign             9        8        8       15       28       12       23        7        32%
builtins                    9        9        9       16       28       13       24        7        32%
break_continue              9        9        8       15       28       12       24        7        32%
void                        9        8        9       15       28       12       24        7        32%
uri                         9        9        8       15       28       12       23        7        32%
types                       9        8        8       15       28       13       24        7        32%
typeof                      9        9        9       15       28       12       23        7        32%
try_catch                   9        8        8       15       28       13       23        7        32%
switch                      9        8        8       15       28       13       24        7        32%
string_methods              9        8        9       15       28       13       24        7        32%
strict_equality             9        9        8       15       28       12       24        7        32%
space_indent                9        8        8       15       28       12       23        7        32%
scopes                      9        8        8       15       28       13       24        7        32%
optional_chaining           9        8        8       15       28       12       23        7        32%
optional_braces_braced        9        8        9       15       28       12       23        7        32%
optional_braces             9        8        8       15       28       12       23        7        32%
length                      9        8        9       15       28       12       24        7        32%
json                        9        8        8       15       28       14       23        7        32%
inc_dec                     9        8        8       15       28       12       24        7        32%
in_op                       9        8        8       15       28       12       23        7        32%
for_of                      9        8        8       15       28       12       23        7        32%
fn_any                      9        9        8       15       28       12       23        7        32%
exponentiation              9        8        8       15       28       13       23        7        32%
do_while                    9        9        9       15       28       13       23        7        32%
conditional                 9        9        9       16       28       14       25        8        32%
arrow_functions             9        9        9       16       29       14       26        7        31%
bitwise                     9        8        8       15       31       13       23        7        29%
tab_indent                  8        8        8       15       28       13       24        7        28%
object_methods              9        9        8       15       33       13       29        7        27%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                   12804      486    10638    11543     1359      624     1145      572       942%


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       26      180      559       41       23       36       66      3446%
object_stress            1000       40      105      364       35       23       32       67      2857%
benchmark_granular        853       30       96      315       36       22       31       71      2369%
new_features_perf         491       28       62      184       36       20       31       64      1363%
objects_perf               23        9       10       20       29       14       26        8        79%
array_methods_perf         18        9       10       20       29       15       26       10        62%
length                     12        9        8       15       28       13       24        7        42%
strict_equality            11        9        9       15       28       13       24        7        39%
space_indent               11       10       10       16       28       14       25        8        39%
scopes                     11        9        9       16       28       14       26        8        39%
void                       11        9        9       16       29       14       25        7        37%
optional_braces_braced       11        9        9       16       29       13       25        8        37%
nested_complex             11        9        9       15       29       13       24        7        37%
optional_chaining          11       12       11       19       30       15       26        8        36%
compound_assign            10        9        8       15       28       13       24        7        35%
uri                        10        8        9       15       28       13       24        7        35%
types                      10        8        9       15       28       13       24        7        35%
try_catch                  10        9        9       16       28       13       25        7        35%
template_literals          10        9        9       16       28       13       25        8        35%
tab_indent                 10        9        9       16       28       14       25        7        35%
switch                     10        9        9       16       28       13       24        7        35%
string_methods_perf        10       10       10       16       28       15       26        9        35%
string_methods             10        9        9       15       28       13       25        8        35%
rest_params                10       10        9       16       28       13       24       10        35%
arrays                     10        9        8       15       28       13       26        7        35%
math                       10        8        9       15       28       13       24        7        35%
in_op                      10        8        9       16       28       13       24        7        35%
higher_order_methods       10        9        9       15       28       13       24        7        35%
for_of                     10        9        8       15       28       13       24        7        35%
fn_any                     10        9        9       15       28       13       24        7        35%
const                      10        9        9       15       28       13       24        7        35%
builtins                   10        9        9       15       29       13       24        7        34%
typeof                     10        9        9       15       29       14       25        7        34%
arrow_functions            10        9        9       15       29       13       24        7        34%
objects                    10        9       10       16       29       13       26        7        34%
json                       10        9        8       15       29       13       25        8        34%
exponentiation             10        9        9       15       29       13       24        8        34%
do_while                   10        9        9       15       29       14       25        7        34%
array_methods              10       11        9       15       29       13       24        7        34%
bitwise                     9        8        9       15       28       13       24        7        32%
mutation                    9        9        9       15       28       13       24        7        32%
inc_dec                     9        8        9       15       28       13       24        7        32%
conditional                 9        8        8       15       28       13       24        8        32%
break_continue              9        9        9       15       29       13       24        7        31%
optional_braces             9        9        9       16       29       14       30        9        31%
nested_loops                9        8        8       15       29       13       25        7        31%
object_methods             10        9        9       15       34       13       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4210      511      830     2094     1376      661     1199      589       305%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       28      179      563       41       24       38       67      3446%
object_stress             998       40      106      357       35       21       30       67      2851%
benchmark_granular        851       30       96      316       35       22       30       70      2431%
new_features_perf         487       28       62      184       35       20       31       64      1391%
objects_perf               23        9        9       19       29       14       24        7        79%
array_methods_perf         20        9       10       20       29       13       25        9        68%
arrow_functions            11        9        9       16       28       14       25        8        39%
nested_complex             11        9        9       15       28       13       24        8        39%
compound_assign            10        9        8       15       28       13       24        7        35%
builtins                   10        9        9       15       28       13       24        7        35%
break_continue             10        9        9       15       28       13       24        7        35%
types                      10        9        9       15       28       13       24        7        35%
strict_equality            10        8        9       15       28       13       25        7        35%
space_indent               10        9        8       15       28       13       24        7        35%
optional_chaining          10        9        9       15       28       13       24        7        35%
length                     10        8        9       15       28       13       24        7        35%
template_literals          10        9        9       15       29       13       24        8        34%
string_methods_perf        10        9        8       14       29       14       25        8        34%
string_methods             10        9        8       15       29       14       24        7        34%
rest_params                10        9        9       15       29       13       24        7        34%
optional_braces            10        9        9       15       29       14       24        7        34%
arrays                     10        9        9       16       29       13       24        8        34%
objects                    10        9        9       15       29       13       31        7        34%
nested_loops               10        8        8       15       29       13       24        7        34%
mutation                   10        9        9       15       29       13       24        7        34%
math                       10        9        8       15       29       13       24        7        34%
json                       10        8        9       15       29       13       25        7        34%
higher_order_methods       10        9        9       15       29       13       24        7        34%
for_of                     10        9        8       15       29       13       24        7        34%
fn_any                     10        9        8       15       29       13       24        7        34%
const                      10        9        9       15       29       13       24        7        34%
array_methods              10        9        9       15       29       13       24        7        34%
tab_indent                 10        9        8       17       30       13       24        7        33%
optional_braces_braced        9        9        9       15       27       13       24        7        33%
bitwise                     9        9        9       15       28       13       25        7        32%
void                        9        9        8       15       28       13       24        7        32%
switch                      9        9        9       15       28       13       24        7        32%
scopes                      9        9        9       15       28       13       24        7        32%
in_op                       9        9        9       15       28       13       24        7        32%
exponentiation              9        8        8       15       28       14       24        7        32%
do_while                    9        9        9       15       28       13       23        7        32%
conditional                 9        9        8       15       28       13       24        7        32%
uri                         9        9        8       15       29       13       24        7        31%
typeof                      9        9        9       15       29       13       24        7        31%
try_catch                   9        9        8       15       29       13       24        7        31%
inc_dec                     9        9        8       16       29       13       24        7        31%
object_methods             10        9        9       15       34       13       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4191      508      816     2078     1379      652     1179      576       303%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────
Done.



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1488       31      170      591       42       24       36       66      3542%
object_stress            1043       41      108      357       35       21       30       67      2980%
benchmark_granular        907       32      100      319       36       22       30       70      2519%
new_features_perf         514       29       63      183       36       19       31       64      1427%
objects_perf               23        9       10       20       28       14       25        8        82%
array_methods_perf         19        9       10       20       29       13       25       12        65%
nested_complex             11        9        9       16       28       13       24        7        39%
in_op                      10        9        8       15       27       13       24        7        37%
for_of                     10        8        8       15       27       13       24        7        37%
builtins                   10        9        8       15       28       13       24        7        35%
break_continue             10        8        8       15       28       13       24        7        35%
uri                        10        9        9       15       28       13       24        7        35%
optional_braces_braced       10        9        9       15       28       12       24        7        35%
objects                    10        9        9       15       28       13       24        7        35%
math                       10        9        8       15       28       13       24        7        35%
json                       10        8        9       15       28       13       24        7        35%
higher_order_methods       10        9        9       16       28       13       24        7        35%
conditional                10        9        9       15       28       12       24        7        35%
compound_assign            10        9        9       15       29       13       24        7        34%
void                       10        9        9       16       29       14       25        8        34%
types                      10        8        9       17       29       14       23        7        34%
template_literals          10        9        9       15       29       13       24        7        34%
tab_indent                 10        9        9       15       29       13       24        7        34%
arrow_functions            10        9        8       15       29       13       24        7        34%
switch                     10        9        8       15       29       12       24        7        34%
string_methods_perf        10       10        9       15       29       14       24        8        34%
string_methods             10        9        9       14       29       13       25        7        34%
scopes                     10        9        9       15       29       13       24        7        34%
rest_params                10        9        9       15       29       13       24        7        34%
length                     10        9        9       15       29       13       24        7        34%
do_while                   10        9        9       15       29       13       24        7        34%
array_methods              10        9        9       15       29       14       25        7        34%
typeof                      9        8        9       15       28       12       24        7        32%
try_catch                   9        8        8       15       28       13       24        7        32%
strict_equality             9        9        9       15       28       13       24        7        32%
optional_chaining           9        9        9       15       28       13       24        7        32%
inc_dec                     9        9        8       15       28       13       24        7        32%
exponentiation              9        8        9       15       28       13       24        7        32%
const                       9        9        9       15       28       13       24        7        32%
bitwise                     9        9        9       15       29       13       24        7        31%
space_indent                9        9        8       15       29       13       25        7        31%
optional_braces             9        9        8       15       29       13       24        7        31%
arrays                      9        9        9       15       29       13       24        7        31%
nested_loops                9        9        8       15       29       13       24       10        31%
mutation                    9        9        8       15       29       13       24        7        31%
fn_any                      9        9        9       15       29       13       24        7        31%
object_methods              9        9        9       15       34       13       30        7        26%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4390      514      817     2109     1379      646     1170      579       318%



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1405       27      186      563       42       24       36       67      3345%
array_methods_perf         19        9       10       20       29       14       25        9        65%
arrays                     10        9        9       15       29       13       25        7        34%
array_methods              10        9        9       15       29       13       24        7        34%



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       26      175      562       42       24       36       66      3364%
object_stress            1000       41      104      360       35       21       30       67      2857%
benchmark_granular        851       30       97      317       35       23       31       71      2431%
new_features_perf         487       28       62      185       36       19       31       64      1352%
objects_perf               23        9       10       20       29       14       24        8        79%
array_methods_perf         20       10       11       21       30       15       26       10        66%
nested_complex             11        8        9       15       29       13       25        8        37%
higher_order_methods       11        9        8       16       29       14       24        7        37%
array_methods              11       10       10       16       29       14       26        8        37%
builtins                   10        9        9       16       28       13       25        8        35%
types                      10        9        9       15       28       13       24        7        35%
template_literals          10        8        9       15       28       13       24        7        35%
switch                     10        9        9       15       28       13       24        7        35%
string_methods             10        9        9       15       28       13       24        7        35%
strict_equality            10        8        9       15       28       13       24        7        35%
scopes                     10        9        9       15       28       13       24        7        35%
inc_dec                    10        9        9       15       28       13       24        7        35%
for_of                     10        8        8       15       28       13       24        8        35%
do_while                   10        8        9       15       28       13       24        7        35%
compound_assign            10        9        9       15       29       13       25        7        34%
void                       10        9        9       15       29       13       24        7        34%
string_methods_perf        10       10        9       15       29       14       24        8        34%
rest_params                10        9        9       15       29       13       24        7        34%
optional_braces_braced       10        8        8       15       29       13       24        7        34%
objects                    10        9        9       15       29       13       24        7        34%
nested_loops               10        8        9       15       29       13       24        7        34%
mutation                   10        9        8       15       29       13       24        7        34%
length                     10        9        9       15       29       13       24        7        34%
json                       10        9        9       15       29       13       24        7        34%
fn_any                     10        9        9       17       29       13       24        7        34%
exponentiation             10        9        9       15       29       13       24        7        34%
const                      10        9        9       15       29       13       25        7        34%
uri                         9        9        8       15       27       13       24        7        33%
arrow_functions            10        9        9       16       30       14       25        7        33%
break_continue              9        9        9       16       28       14       25        7        32%
try_catch                   9        9        9       15       28       13       25        7        32%
space_indent                9        9        8       15       28       13       24        8        32%
optional_chaining           9        9        8       15       28       13       24        7        32%
optional_braces             9        9        9       15       28       13       24        7        32%
bitwise                     9        9        9       15       29       13       24        7        31%
typeof                      9        9        9       15       29       13       26        7        31%
tab_indent                  9        9        8       15       29       13       24        7        31%
arrays                      9        9        8       15       29       14       25        8        31%
math                        9        8        9       15       29       13       25        7        31%
in_op                       9        9       10       16       29       13       24        7        31%
conditional                 9        9        8       15       29       13       24        8        31%
object_methods             10        9        9       15       34       13       30        8        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4194      507      820     2088     1385      655     1181      582       302%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────






════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1412       26      177      178      560       42       24       36       66      3361%
object_stress            1003       39      104      104      360       35       21       31       69      2865%
benchmark_granular        847       29       95       96      312       35       22       32       70      2420%
new_features_perf         487       26       62       62      184       37       20       31       65      1316%
objects_perf               23        9       10       10       20       29       14       25        8        79%
array_methods_perf         19        9       10       10       20       29       14       25        9        65%
tab_indent                 12       14        9        8       15       28       14       24        8        42%
nested_complex             11        9        9        9       16       29       14       25        8        37%
fn_any                     11        9        9        9       15       29       13       25        7        37%
uri                        10        9        9        9       15       28       13       25        8        35%
typeof                     10        9        9        9       15       28       13       25        7        35%
template_literals          10        9        8        8       15       28       13       25        7        35%
arrow_functions            10        9        9        9       15       28       13       25        8        35%
strict_equality            10        9        9        9       15       28       13       25        7        35%
rest_params                10        9        9        9       15       28       13       25        7        35%
arrays                     10        9        9        9       15       28       13       24        8        35%
in_op                      10        9        9        9       16       28       14       25        7        35%
compound_assign            10        9        9        9       16       29       13       24        7        34%
builtins                   10        9        9        9       15       29       13       25        7        34%
break_continue             10        9        8        9       15       29       13       24        8        34%
bitwise                    10        9        9        8       16       29       13       24        7        34%
types                      10        9        9        9       16       29       14       31        9        34%
try_catch                  10        9        9        9       15       29       13       24        7        34%
switch                     10        8        9        9       16       29       13       24        7        34%
string_methods_perf        10        9        9        9       15       29       14       25        9        34%
string_methods             10        9        9        9       15       29       15       27        8        34%
space_indent               10        9        9        9       15       29       13       25        7        34%
scopes                     10        9        9        9       15       29       14       24        7        34%
optional_braces_braced       10        9        9        9       16       29       13       27        7        34%
objects                    10        9        9        9       15       29       13       25        7        34%
nested_loops               10        9        9        9       15       29       13       24        7        34%
mutation                   10        9        9        8       15       29       13       24        7        34%
math                       10        8        9        9       15       29       13       24        7        34%
length                     10        8        9        8       14       29       13       25        7        34%
json                       10        9        9        8       16       29       13       24        7        34%
inc_dec                    10        9        9        9       15       29       14       25        7        34%
higher_order_methods       10        9        9        9       16       29       14       24        8        34%
for_of                     10        9        9        9       15       29       13       24        8        34%
exponentiation             10        9        9        9       15       29       13       24        7        34%
do_while                   10        9        9        9       15       29       13       24        7        34%
const                      10        9        9        9       15       29       14       25        7        34%
array_methods              10        9        9        9       15       29       14       25        8        34%
optional_chaining           9        9        8        9       15       28       14       26        7        32%
optional_braces             9        9        9        9       15       28       13       24        7        32%
void                        9        8        9        9       15       29       13       24        7        31%
conditional                 9        9        9        9       16       29       13       25        8        31%
object_methods             10        9        9        9       15       34       14       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4201      508      824      823     2080     1390      662     1203      589       302%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime




Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1407      499       26      189      177      551       42       24       37       67      3350%
array_methods_perf         19       17        9       10        9       20       29       14       24        9        65%
array_methods              10       10        9        9        9       16       29       14       25        8        34%
arrays                      9        9        8        9        9       16       30       14       25        7        30%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    1445      535       52      217      204      603      130       66      111       91      1111%


--release
Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress              169       69       26      195      187      552       41       23       37       67       412%
array_methods_perf         10       11        9       10        9       20       29       13       24        9        34%
array_methods               9        9        8        9        9       16       29       13       24        7        31%
arrays                      9       10        9        9        9       15       30       13       24        7        30%


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         166       69       26      190      189      555       42       23       36       66       395%
core/object_stress        109       91       39      110      115      369       35       21       30       69       311%
core/new_features_perf       63       56       27       64       64      185       36       20       31       64       175%
core/benchmark_granular       39       91       28       39       39      121       36       23       32       73       108%
core/array_methods_perf       10       10        9        9        9       19       28       13       24        9        35%
core/break_continue        10       10       10        9        9       16       29       14       24        7        34%
core/template_literals       10        9        8        9        9       15       29       13       24        8        34%
core/arrow_functions       10        9        9        9        9       15       29       13       24        7        34%
core/rest_params           10        9        9        9        9       15       29       13       25        7        34%
core/nested_complex        10        9        9        9        9       16       29       14       25        8        34%
core/length                10        9        9        8        9       14       29       13       25        7        34%
core/array_methods         10        9        8        9        9       16       29       13       25        7        34%
core/bitwise               10        9        9        9        9       15       30       13       25        8        33%
modules/settimeout         10       10        -        9        9       15       30       14       26        8        33%
modules/file_io            10        9        -        -        -        -       30       16       24        7        33%
core/objects_perf          10       10        9       10       10       20       30       14       25        8        33%
core/compound_assign        9        9        9        9        9       15       28       13       24        7        32%
modules/promise             9        9        -        -        -        -       28       14       25        7        32%
core/typeof                 9        9        8        9        9       15       28       13       24        7        32%
core/try_catch              9        9        9        9        9       15       28       13       24        7        32%
core/string_methods         9        9        9        8        9       14       28       13       24        7        32%
core/optional_chaining        9        9        9        9        8       15       28       13       24        7        32%
core/optional_braces_braced        9        9        9        9        8       15       28       13       24        8        32%
core/optional_braces        9        9        8        9        9       15       28       13       24        7        32%
core/arrays                 9        9        9        9        9       15       28       13       24        7        32%
core/in_op                  9        9        9        9        9       15       28       13       25        7        32%
core/builtins               9       10        9        9        9       16       29       13       24        7        31%
core/void                   9        9        9        9        8       15       29       13       24        7        31%
core/uri                    9        9        9        9        9       15       29       13       24        7        31%
core/types                  9        9        9        9        8       15       29       13       24        7        31%
core/tab_indent             9        9        8        9        8       15       29       13       24        7        31%
core/switch                 9        9        9        9        9       15       29       13       24        7        31%
core/string_methods_perf        9       11        9        8        9       14       29       14       24        9        31%
core/strict_equality        9        9        9        9        8       15       29       13       25        7        31%
core/space_indent           9        9        9        9        8       15       29       13       24        7        31%
core/scopes                 9        9        8        9        9       15       29       13       24        8        31%
core/objects                9        9        9        9        9       16       29       13       24        7        31%
core/nested_loops           9        9        9        8        8       15       29       13       24        7        31%
core/mutation               9        9        9        9        8       16       29       13       25        8        31%
core/math                   9        9        9        9        9       15       29       13       24        7        31%
core/json                   9        9        9        9        9       15       29       13       24        7        31%
core/inc_dec                9        9        9        9        9       15       29       13       24        8        31%
core/higher_order_methods        9       10        9        9        9       16       29       13       24        7        31%
core/for_of                 9        9        9        8        9       15       29       13       24        7        31%
core/fn_any                 9        9        8        9        9       15       29       13       24        7        31%
core/exponentiation         9        9        9        9        9       15       29       13       24        8        31%
core/do_while               9        9        8        9        9       15       29       13       24        7        31%
core/const                  9        9        9        9        9       15       29       13       24        8        31%
core/conditional            9        9        9        9        8       15       29       13       24        7        31%
core/object_methods         9        9        8        8        9       15       34       14       30        7        26%
modules/process             9        9        -        -        -        -       39       13       24        8        23%
modules/http_server         9        9        -        -        -        -       51       30       24        8        17%
modules/file_io_perf        9        9        -        -        -        -      162      147       25        7         5%
modules/http_perf           9        9        -        -        -        -     1861     1515     1413        8         0%
modules/http_fetch          9        9        -        -        -        -     1034      871      844        7         0%
modules/async_promise_settimeout        9        9        -        -        -        -      931      992      964        7         0%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                     857      783      499      794      794     1903     5559     4263     4545      654        15%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         171       67       29      185      188   213721       38       21       32       59       450%
core/object_stress         99       82       36      100       98      317       31       18       26       60       319%
core/new_features_perf       57       51       24       58       57      162       32       17       27       59       178%
core/benchmark_granular       48      119       40       56       66      355       32       18       27       63       150%
core/string_methods_perf       11       11        9        9        9       19       26       12       21        7        42%
core/break_continue        10        9        9        7        7       14       25       11       22        6        40%
core/higher_order_methods       10       10        9        7        7       14       25       11       21        6        40%
core/objects_perf          10        9        9        8        8       17       26       11       20        6        38%
core/nested_complex        10        9        8        7        7       13       26       11       21        7        38%
core/in_op                  9        9        9        8        8       13       24       12       21        6        37%
core/array_methods_perf       11       11       11       10       11       21       29       13       24        9        37%
core/void                   9        8        8        7        7       13       25       11       21        6        36%
core/uri                    9        9        8        7        7       13       25       11       21        6        36%
core/types                  9        9        8        7        7       13       25       11       21        6        36%
core/typeof                 9        9        9        7        7       13       25       11       21        6        36%
core/try_catch              9        8        9        7        8       13       25       10       21        6        36%
core/tab_indent             9        8        8        7        7       14       25       11       21        6        36%
core/switch                 9        8        8        7        7       13       25       11       21        6        36%
core/space_indent           9        8        9        7        7       13       25       10       21        6        36%
core/rest_params            9        9        8        7        7       14       25       11       21        6        36%
core/optional_braces_braced        9        9        8        7        7       14       25       11       21        6        36%
core/optional_braces        9        8        8        7        7       13       25       11       21        6        36%
core/objects                9        9        8        7        7       13       25       11       21        6        36%
core/length                 9        9        9        7        7       13       25       11       21        6        36%
core/json                   9        8        9        7        7       14       25       11       21        6        36%
core/inc_dec                9        9        9        7        7       14       25       11       21        6        36%
core/for_of                 9        9        9        7        7       14       25       12       22        6        36%
core/do_while               9        9        9        7        7       14       25       11       21        6        36%
core/const                  9        9        8        7        7       14       25       11       22        6        36%
core/conditional            9        9        8        7        7       14       25       12       21        6        36%
core/builtins               9        9        9        7        8       14       26       11       21        6        34%
core/bitwise                9        9        9        8        7       14       26       11       22        6        34%
core/template_literals        9        9        8        7        7       13       26       11       21        7        34%
core/string_methods         9        8        8        7        7       14       26       11       21        6        34%
core/scopes                 9        8        8        7        7       14       26       11       21        6        34%
core/arrays                 9        9        9        7        7       14       26       11       22        6        34%
core/mutation               9        9        8        8        7       14       26       11       21        6        34%
core/math                   9        9        8        7        7       13       26       11       21        6        34%
core/fn_any                 9        9        9        7        7       14       26       11       22        6        34%
core/array_methods         10       10        9        8        8       15       29       13       24        7        34%
core/compound_assign        9        9        9        8        8       15       27       12       21        6        33%
core/arrow_functions       11       11       11        9        9       17       34       16       32       10        32%
core/optional_chaining        8        9        9        8        7       13       25       10       21        6        32%
core/exponentiation         8        9        9        7        8       13       25       11       21        6        32%
core/strict_equality        8        9        8        7        7       14       26       11       21        6        30%
core/object_methods         9        9        9        7        7       14       30       11       25        6        30%
core/nested_loops           8        8        9        7        7       13       26       11       21        6        30%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                     769      704      502      714      725   215160     1245      558     1041      510        61%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime




./scripts/run_object_stress_profile.sh; \
./scripts/run_benchmark_granular_profile.sh; \

./scripts/run_object_stress_profile.sh --instrument; \
TISH_PROFILE=1 cargo run -p tishlang--features "full,profile" -- run tests/core/benchmark_granular_04_nested_fn.tish



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         160       63       30      163      173      606       44       24       37       66       363%
core/object_stress         85       78       37       87       86      339       36       25       31       66       236%
core/new_features_perf       57       51       28       58       58      181       36       20       32       63       158%
core/benchmark_granular       35       80       29       33       33      121       37       22       32       69        94%
core/string_methods_perf       13       14       12       11       11       23       29       14       25        9        44%
core/json                  13       11       10        9        9       17       29       13       25        7        44%
core/objects_perf          12       11       11       10       10       21       29       14       25        7        41%
core/array_methods_perf       12       12       11       10       10       22       30       14       25        9        40%
core/void                  11       10       10        8        9       16       28       13       25        7        39%
core/uri                   11       11       10        9        9       16       28       13       25        8        39%
core/switch                11       11       11        9        9       16       28       13       25        8        39%
core/optional_braces       11       11       10        9        8       16       28       13       25        7        39%
core/math                  11       11       11        9        9       16       28       13       24        8        39%
core/compound_assign       11       11       11        9        9       17       29       14       26        7        37%
core/break_continue        11       11       10        9        9       17       29       13       25        7        37%
core/bitwise               11       11       11        9        9       16       29       14       25        7        37%
core/types                 11       10       10        9        8       16       29       14       26        7        37%
core/typeof                11       14       11        9        9       17       29       13       25        7        37%
core/try_catch             11       10       10        9        8       17       29       13       25        7        37%
core/template_literals       11       11       10        9        9       16       29       14       25        7        37%
core/tab_indent            11       11       11        9        9       16       29       14       25        7        37%
core/string_methods        11       11       11        9        9       17       29       13       25        7        37%
core/space_indent          11       11       10        9        9       17       29       13       25        7        37%
core/scopes                11       11       11        9        9       16       29       13       25        7        37%
core/rest_params           11       11       11        9        9       16       29       13       25        7        37%
core/optional_chaining       11       11       11        9        9       17       29       13       25        8        37%
core/optional_braces_braced       11       11       10        9        9       17       29       14       25        7        37%
core/arrays                11       11       11        9        9       17       29       13       25        7        37%
core/nested_loops          11       11       10        9        9       17       29       13       25        7        37%
core/nested_complex        11       11       11        9        8       17       29       14       25        8        37%
core/mutation              11       11       10        9        8       17       29       14       25        9        37%
core/length                11       11       11        9        8       16       29       13       25        7        37%
core/inc_dec               11       11       10        9        9       17       29       13       25        7        37%
core/in_op                 11       11       11        9        8       16       29       14       25        7        37%
core/higher_order_methods       11       10       10        9        9       16       29       14       25        7        37%
core/for_of                11       11       11        9        9       16       29       14       27        7        37%
core/fn_any                11       11       10        8        9       16       29       13       25        7        37%
core/do_while              11       11       10        9        9       16       29       13       25        8        37%
core/conditional           11       11       11        9        9       17       29       13       26        7        37%
core/array_methods         12       11       10        9        9       17       32       13       25        8        37%
core/arrow_functions       11       11       11        9        9       17       30       14       25        8        36%
core/objects               11       11       10        9        9       16       30       14       26        7        36%
core/builtins              11       11       10        9        9       17       31       14       25        7        35%
core/const                 11       11       10        9        9       17       31       14       25        7        35%
core/strict_equality       10       13       11        9        9       16       29       13       25        7        34%
core/exponentiation        10       11       10        9        9       16       29       13       25        7        34%
core/object_methods        11       11       10        9        9       17       36       14       31        7        30%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                     815      750      576      730      734     1973     1412      669     1218      579        57%


debug (not part of release metrics):
════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress        1368      452       30      165      167      592       42       24       36       65      3257%
core/object_stress        869      623       38       85       89      323       35       21       30       66      2482%
core/new_features_perf      455      348       26       58       56      173       35       19       31       63      1300%
core/benchmark_granular      297      611       28       32       33      115       36       21       31       70       825%
modules/http_server        69       68       67       66       68        -       51       27       27        7       135%
core/string_methods_perf       31       24       10       10       10       21       28       13       24        8       110%
modules/file_io_perf      142      153      107      116      107       20      157      133      181        7        90%
core/objects_perf          23       18       10        9        9       19       28       13       24        7        82%
core/array_methods_perf       20       19       10        9        9       21       30       14       25        9        66%
core/nested_complex        13       13       10        8        9       16       29       13       24        7        44%
core/arrays                12       11       11        8        9       16       28       13       24        7        42%
core/length                12       11       10        9        9       15       29       12       24        7        41%
modules/promise            11       10       70        9        8       14       27       12       23        6        40%
core/uri                   11       11       10        9        8       15       27       13       24        7        40%
core/array_methods         12       12       10        8        8       16       30       13       25        8        40%
core/void                  11       10        9        8        8       15       28       12       23        7        39%
core/types                 11       11       10        8        8       15       28       12       24        7        39%
core/typeof                11       11        9        8        8       15       28       12       24        7        39%
core/try_catch             11       11        9        8        8       15       28       13       24        7        39%
core/template_literals       11       11        9        8        8       16       28       13       24        7        39%
core/switch                11       10        9        8        8       15       28       12       23        6        39%
core/string_methods        11       11        9        8        8       15       28       13       24        7        39%
core/scopes                11       10       10        8        8       15       28       12       24        7        39%
core/rest_params           11       10        9        8        8       15       28       12       23        7        39%
core/optional_chaining       11       11       10        8        9       15       28       13       23        6        39%
core/optional_braces_braced       11       10        9        8        8       15       28       12       24        7        39%
core/optional_braces       11       10        9        8        8       15       28       12       24        7        39%
core/nested_loops          11       11       10        8        9       15       28       12       24        7        39%
core/math                  11       11       10        8        8       15       28       12       23        7        39%
core/inc_dec               11       11       11        8        8       16       28       14       24        7        39%
core/in_op                 11       11       10        8        8       15       28       12       24        7        39%
core/fn_any                11       10       10        8        8       15       28       12       24        7        39%
core/exponentiation        11       11        9        8        8       15       28       13       24        7        39%
core/conditional           11       11       10        8        8       15       28       12       24        7        39%
core/compound_assign       11       11       10        9        8       15       29       13       24        7        37%
core/builtins              11       11       11        8        8       15       29       12       24        6        37%
core/break_continue        11       10        9        8        8       15       29       13       23        7        37%
core/bitwise               11       10        9        8        8       16       29       12       23        7        37%
core/arrow_functions       11       11       10        9        9       16       29       13       24        7        37%
core/strict_equality       11       11        9        8        8       15       29       12       24        7        37%
core/space_indent          11       11        9        8        8       15       29       12       24        7        37%
core/objects               11       11        9        8        8       16       29       13       24        7        37%
core/mutation              11       11       10        8        8       15       29       12       24        7        37%
core/json                  11       11       11        8        9       16       29       13       24        7        37%
core/higher_order_methods       11       11       10        8        8       15       29       13       24        7        37%
core/for_of                11       10        9        8        8       15       29       13       24        7        37%
core/do_while              11       11       12       10        9       16       29       12       24        7        37%
modules/process            11       10        9        8        7       15       30       17       24        7        36%
modules/file_io            11       11        9        9        9       15       30       16       24        6        36%
modules/settimeout         10       10        -        7        7       14       28       13       24        6        35%
core/tab_indent            10       10        9        8        8       15       28       12       23        7        35%
core/const                 11       12       14       12       15       18       31       14       26        7        35%
core/object_methods        11       11        9        8        8       15       34       13       29        7        32%
modules/http_fetch         13     1501     7135       10       10        -     1287     1142     1047        8         1%
modules/async_promise_settimeout       13     1605     1529       11        9        -     1253     1484      942        8         1%
modules/http_perf          13     3634     3308       10        9        -     2047    17914     1677        7         0%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    3800     9530    12778      944      942     1955     6289    21384     5130      628        60%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime


release (core only)


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         160       63       30      163      173      606       44       24       37       66       363%
core/object_stress         85       78       37       87       86      339       36       25       31       66       236%
core/new_features_perf       57       51       28       58       58      181       36       20       32       63       158%
core/benchmark_granular       35       80       29       33       33      121       37       22       32       69        94%
core/string_methods_perf       13       14       12       11       11       23       29       14       25        9        44%
core/json                  13       11       10        9        9       17       29       13       25        7        44%
core/objects_perf          12       11       11       10       10       21       29       14       25        7        41%
core/array_methods_perf       12       12       11       10       10       22       30       14       25        9        40%
core/void                  11       10       10        8        9       16       28       13       25        7        39%
core/uri                   11       11       10        9        9       16       28       13       25        8        39%
core/switch                11       11       11        9        9       16       28       13       25        8        39%
core/optional_braces       11       11       10        9        8       16       28       13       25        7        39%
core/math                  11       11       11        9        9       16       28       13       24        8        39%
core/compound_assign       11       11       11        9        9       17       29       14       26        7        37%
core/break_continue        11       11       10        9        9       17       29       13       25        7        37%
core/bitwise               11       11       11        9        9       16       29       14       25        7        37%
core/types                 11       10       10        9        8       16       29       14       26        7        37%
core/typeof                11       14       11        9        9       17       29       13       25        7        37%
core/try_catch             11       10       10        9        8       17       29       13       25        7        37%
core/template_literals       11       11       10        9        9       16       29       14       25        7        37%
core/tab_indent            11       11       11        9        9       16       29       14       25        7        37%
core/string_methods        11       11       11        9        9       17       29       13       25        7        37%
core/space_indent          11       11       10        9        9       17       29       13       25        7        37%
core/scopes                11       11       11        9        9       16       29       13       25        7        37%
core/rest_params           11       11       11        9        9       16       29       13       25        7        37%
core/optional_chaining       11       11       11        9        9       17       29       13       25        8        37%
core/optional_braces_braced       11       11       10        9        9       17       29       14       25        7        37%
core/arrays                11       11       11        9        9       17       29       13       25        7        37%
core/nested_loops          11       11       10        9        9       17       29       13       25        7        37%
core/nested_complex        11       11       11        9        8       17       29       14       25        8        37%
core/mutation              11       11       10        9        8       17       29       14       25        9        37%
core/length                11       11       11        9        8       16       29       13       25        7        37%
core/inc_dec               11       11       10        9        9       17       29       13       25        7        37%
core/in_op                 11       11       11        9        8       16       29       14       25        7        37%
core/higher_order_methods       11       10       10        9        9       16       29       14       25        7        37%
core/for_of                11       11       11        9        9       16       29       14       27        7        37%
core/fn_any                11       11       10        8        9       16       29       13       25        7        37%
core/do_while              11       11       10        9        9       16       29       13       25        8        37%
core/conditional           11       11       11        9        9       17       29       13       26        7        37%
core/array_methods         12       11       10        9        9       17       32       13       25        8        37%
core/arrow_functions       11       11       11        9        9       17       30       14       25        8        36%
core/objects               11       11       10        9        9       16       30       14       26        7        36%
core/builtins              11       11       10        9        9       17       31       14       25        7        35%
core/const                 11       11       10        9        9       17       31       14       25        7        35%
core/strict_equality       10       13       11        9        9       16       29       13       25        7        34%
core/exponentiation        10       11       10        9        9       16       29       13       25        7        34%
core/object_methods        11       11       10        9        9       17       36       14       31        7        30%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                     815      750      576      730      734     1973     1412      669     1218      579        57%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime



        ════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
modules/http_server     30019    30024        -        -        -        -       49       54       58        8     61263%
modules/http_fetch       7245     1646        -        -        -        -     1324     1232     1747        7       547%
core/array_stress         160       56       29      162      162      548       40       23       37       65       400%
core/object_stress         86       73       38       86       85      317       35       20       30       69       245%
modules/http_perf        4449     3859        -        -        -        -     2459     2112     2782        8       180%
core/new_features_perf       56       47       27       57       57      170       35       20       31       66       160%
core/benchmark_granular       48       76       28       32       32      109       35       23       32       72       137%
modules/file_io_perf      111      124        -        -        -        -      165      130      170        7        67%
core/array_methods_perf       14       12       10        9        9       19       28       14       27       10        50%
core/space_indent          14       11       11       10        9       19       29       16       25        7        48%
core/types                 13       11       11        9       10       17       30       14       25        7        43%
core/higher_order_methods       12       11       10        8        8       17       28       13       24        8        42%
core/optional_braces       12       10       10        9        9       16       29       12       23        6        41%
core/fn_any                11       10       11        8        8       15       27       13       25        7        40%
core/array_methods         11       11        9        8        8       15       27       13       25        7        40%
core/break_continue        11       12       11        9        9       23       28       14       30        8        39%
core/bitwise               11       12       12        8        9       16       28       12       26        7        39%
modules/settimeout         11       10        -        -        -        -       28       13       23        6        39%
modules/promise            11       10        -        -        -        -       28       13       25        8        39%
core/uri                   11       11       11       10        9       16       28       14       25        7        39%
core/try_catch             11       10       11        9        9       16       28       12       24        8        39%
core/arrow_functions       11       11       10        8        8       15       28       12       25        7        39%
core/optional_braces_braced       11       11       11        9        9       16       28       14       25        8        39%
core/objects_perf          11       11       12       10        9       20       28       14       26        7        39%
core/in_op                 11       11       12       10        9       17       28       13       25        7        39%
core/void                  11       10       10       10        9       17       29       14       26        7        37%
core/nested_loops          11       11       10        8        9       15       29       12       24        7        37%
core/nested_complex        11       11       10        9        8       16       29       13       25        7        37%
core/inc_dec               11       11       10        9        8       15       29       13       24        7        37%
core/do_while              10       10        9        9        8       15       27       12       25        8        37%
core/compound_assign       10       10        9        8        8       15       28       13       24        7        35%
core/builtins              10       10       10        8        8       16       28       12       24        7        35%
modules/file_io            11       11        -        -        -        -       31       17       25        7        35%
core/tab_indent            10        9       10        8        9       16       28       12       27        8        35%
core/switch                11       10       10        9        9       16       31       13       26        8        35%
core/optional_chaining       11       12       10        9        8       17       31       13       24        7        35%
core/arrays                10       11        8        9        8       15       28       14       25        7        35%
core/mutation              10       10       10        8        8       16       28       12       26        8        35%
core/length                10       10        9        8        8       15       28       12       23        7        35%
core/json                  10       11       10        9        8       15       28       12       23        7        35%
core/exponentiation        10       10       11        9        9       15       28       13       25        7        35%
core/conditional           10       10       10        8        8       16       28       13       24        7        35%
core/typeof                10       11       10        9        9       16       29       14       25        8        34%
core/string_methods_perf       10       11       11       11       10       21       29       15       25        8        34%
core/strict_equality       10       11       10        9       10       16       29       12       25        7        34%
core/scopes                11       11       10       12       11       19       32       14       26        8        34%
core/rest_params           10        9       10        9        8       16       29       13       49       13        34%
core/objects               10       10       10       10       16       17       29       14       26        8        34%
core/math                  10       10       10        8        9       16       29       12       24        7        34%
core/for_of                10       11       10        9        9       16       29       13       23        7        34%
core/const                 10       10       10        9        8       16       29       15       25        8        34%
modules/process            10       11        -        -        -        -       30       18       24        7        33%
core/template_literals       10       10       10        8        9       16       30       14       24        7        33%
core/string_methods        11       17       10        8        8       15       34       14       24        8        32%
core/object_methods        10       11       10        9        9       17       35       16       31        7        28%
modules/async_promise_settimeout       12     1726        -        -        -        -     1473     1163     1289        9         0%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                   42692    38136      561      720      717     1852     6974     5407     7375      662       612%




════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         165       58        -      172      180      583       44       25       38       68       375%
core/object_stress         98       73       37      102      105      352       37       21       31       74       264%
core/new_features_perf       60       49       26       64       65      181       37       22       32       68       162%
core/benchmark_granular       56       78       27       56       56      185       37       23       32       74       151%
core/string_methods_perf       12       12       10       11       10       23       29       15       25        9        41%
core/void                  11       10        9        9        8       16       29       12       24        7        37%
core/template_literals       11       11       12       10        9       16       29       13       24        7        37%
core/higher_order_methods       11       10        -       12       13       18       29       13       25        8        37%
core/exponentiation        11       11       12        9       20       26       30       15       27        9        36%
core/arrow_functions       11        9        9        9        9       17       31       14       26        7        35%
core/rest_params           11       12       11       10       12       18       31       14       24        7        35%
core/array_methods_perf       12       12       11       10       11       22       34       15       30       10        35%
core/builtins              11       10        9       10        9       16       32       13       25        7        34%
core/bitwise               10       10        9        9        9       15       29       13       24        7        34%
core/typeof                10       10       10        9        9       17       29       12       25        7        34%
core/space_indent          10       10        8        8       10       17       29       13       27        8        34%
core/optional_braces       10        9       23       12       13       17       29       13       25        7        34%
core/for_of                10        9        9        9        9       17       29       14       26        8        34%
core/objects               10        9        9       10       13       17       30       13       26        7        33%
core/const                 10       10        9        8        9       17       30       14       26        8        33%
core/array_methods         10        9        9        9        8       16       30       14       25        7        33%
core/tab_indent             9        8        8        9        9       15       28       13       24        8        32%
core/switch                 9        9        8        8        8       16       28       13       24        7        32%
core/string_methods         9        9        9        8        8       16       28       15       26        7        32%
core/strict_equality        9        9        9        9        9       15       28       12       23        7        32%
core/scopes                 9        8        8        8        8       16       28       13       25        7        32%
core/length                 9        9        9        9        9       16       28       12       25        7        32%
core/inc_dec                9        9        8        8        9       16       28       12       24        8        32%
core/compound_assign        9        8        8        8        8       17       29       14       25        7        31%
core/break_continue         9        9        8        8        9       16       29       15       26        8        31%
modules/promise             9       10        -       11       10       18       29       14       25        8        31%
modules/file_io            10        9        -        9        9       16       32       18       27        7        31%
core/uri                    9        9        8        8        8       15       29       12       24        7        31%
core/types                  9        8        8        8        8       16       29       12       23        7        31%
core/try_catch              9        9        8        9        8       16       29       14       24        8        31%
core/optional_chaining        9        9        9        9       10       16       29       14       26        8        31%
core/mutation               9        9        9        8        8       15       29       13       25        7        31%
core/json                   9        9        9        8        8       16       29       12       26        7        31%
core/in_op                  9        9        8        9        9       16       29       12       24        7        31%
core/do_while               9        9        9       10        9       17       29       12       25        8        31%
modules/settimeout          9        9        -        9        9       17       30       13       26        7        30%
core/optional_braces_braced        9       10        9        9        8       16       30       13       29        8        30%
core/fn_any                 9        9       10       10       10       17       30       14       26        8        30%
core/objects_perf          10        9        9        9        9       21       34       15       27        7        29%
core/conditional            9        8        8        8        9       16       31       14       24        8        29%
core/nested_complex         9        9        -        8        9       16       32       14       25       16        28%
core/math                   9        9        8        8       11       17       32       14       24        7        28%
modules/process             9       10        -        9       10       16       33       19       26        8        27%
core/nested_loops           9        9        8        9        9       21       33       12       25        8        27%
core/object_methods         9        9        8       14       10       15       34       13       31        7        26%
core/arrays                 9        9       10       10       10       26       35       15       29        7        25%
modules/http_server        11       10        -    30016    30018        -       53       53       55        8        20%
modules/file_io_perf        9       10        -      111      118       23      160      146      179        8         5%
modules/http_perf          10        9        -     9409     4012        -     2582     1906     1910        8         0%
modules/http_fetch         12       10        -     1707     1698        -     1351     1265     1015        8         0%
modules/async_promise_settimeout        9        8        -        9        9        -     1320     1140     1073        9         0%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                     883      746      472    42077    36710     2131     7031     5239     5562      686        12%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime

─────────────────────────────────────────
Done.
a_@s-MacBook-Pro tish % 


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(vm)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                       vm   interp     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS   vm/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
core/array_stress         163       57       29      171      173      584       43       24       37       67       379%
core/object_stress        101       75       39      102      109      359       38       22       32       70       265%
modules/http_perf        3994     4370     3683     4345     3826        -     2265     2557     1833        8       176%
core/new_features_perf       63       59       28       64       66      182       36       21       34       71       175%
modules/http_fetch       2122     1717     1669     1615     1616        -     1223     1105     1222        9       173%
core/benchmark_granular       56       77       30       55       55      187       36       23       31       74       155%
modules/http_server        65       68       68       68        -        -       51       30       29        8       127%
modules/file_io_perf       37       16       16       47       15       28       30       17       88        8       123%
core/do_while              18       10       10       10       10       18       30       14       27        8        60%
core/const                 27       28       26       22       22       39       52       36       47       19        51%
core/conditional           26       24       26       23       23       35       52       30       45       17        50%
core/space_indent          14       12       11        9        9       17       30       15       25        8        46%
modules/promise            14       12       75       10        9       18       31       13       25        7        45%
core/compound_assign       22       23       25       26       26       34       49       35       53       17        44%
core/string_methods_perf       13       13       12       11       12       24       30       15       26        9        43%
core/objects_perf          13       13       12       11       11       21       30       15       26        9        43%
core/array_methods_perf       13       12       11       10       10       21       30       14       50        9        43%
core/uri                   12       12       12       10       10       17       28       13       25        8        42%
core/builtins              15       15       16       13       22       18       36       20       32        9        41%
core/optional_braces_braced       12       11       10        9        9       16       29       13       25        7        41%
core/optional_braces       12       13       11        9        9       17       29       14       26        8        41%
core/objects               12       12       11       10        9       16       29       13       24        7        41%
core/nested_loops          12       12       11        9        9       17       29       14       26        8        41%
core/scopes                12       13       11       10        9       17       30       14       26        8        40%
core/rest_params           12       12       11       10        9       18       30       15       26        8        40%
core/mutation              12       12       11        9        9       17       30       15       26        8        40%
core/inc_dec               12       17       12        9        9       17       30       15       26        8        40%
core/exponentiation        12       11       11        9       10       17       30       14       26        8        40%
core/switch                11       10       11        9        9       16       28       13       24        7        39%
core/break_continue        12       16       11       10       12       18       31       14       27        8        38%
core/try_catch             12       11       11        9       10       17       31       15       27        8        38%
core/strict_equality       12       11       12       10       10       17       31       15       26        8        38%
core/nested_complex        12       12        -       10       11       18       31       18       26        8        38%
modules/file_io            12       12       11        9       10       17       32       18       27        9        37%
core/void                  11       11       11        8        9       17       29       13       24        8        37%
core/types                 11       12       11       10       10       16       29       14       25        8        37%
core/typeof                11       11       11        9        9       16       29       14       25        8        37%
core/template_literals       11       11       10        9        9       19       29       14       26        8        37%
core/tab_indent            11       10       10        9        9       16       29       14       25        8        37%
core/arrow_functions       11       11       10       12        9       17       29       14       26        8        37%
core/string_methods        11       12       11        9        9       17       29       14       33        8        37%
core/arrays                11       11       10        8        9       17       29       13       25        7        37%
core/length                11       11       11        9        9       17       29       14       25        7        37%
core/for_of                11       11       12       10        9       17       29       14       27        8        37%
core/bitwise               11       11       11       10        9       17       30       14       26        8        36%
modules/settimeout         11       11       10        8        9       18       30       16       27        8        36%
modules/process            12       12       11       10        9       17       33       19       25        8        36%
core/json                  11       11       10        9        9       17       30       14       26        7        36%
core/in_op                 11       11       12       11        9       17       30       13       25        8        36%
core/array_methods         11       11       10        9        9       17       30       14       42        7        36%
core/math                  12       11       11       10        9       17       34       13       25        7        35%
core/higher_order_methods       11       10       11       10        9       18       32       15       28        8        34%
core/optional_chaining       10       10       10        8        9       16       31       14       25        8        32%
core/object_methods        11       11       11       10        9       17       35       14       32        8        31%
core/fn_any                11       11       11        9        9       17       37       14       26        7        29%
modules/async_promise_settimeout       12     1759     1478       12       13        -     1239     1422     1230        8         0%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    7211     8788     7656     6972     6371     2214     6451     5959     5873      723       111%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime
