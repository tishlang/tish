################################################################################
##  ★ FLAGSHIP WIN — array-element JIT (`arr[i]` in JIT'd loops), 38× (2026-06-06). DEFAULT ON.
################################################################################
The numeric JIT can now read array elements inside compiled loops, so numeric-array-REDUCTION
functions (`sum`, `dot`, `max`, scale-sum, …) run at native speed instead of per-element interpreter
dispatch. MEASURED: `sumArr(arr, n)` over a 100k array × 1000 calls = **7782ms → 205ms (38×)**, result
identical. This is the lever the rigorous Value-shrink null-result pointed to: tish is dispatch-bound,
so the win is ELIMINATING dispatch (native loop), not shrinking data.

HOW (crates/tish_vm/src/jit.rs + vm.rs): the f64-only JIT ABI can't pass arrays, so array-mode uses
ONE uniform signature for every such fn — `extern "C" fn(numeric: *const f64, handles: *const
ArrayHandle, deopt: *mut u8) -> f64` (a single transmute, no per-arity explosion). `classify_params`
marks a param as ARRAY iff it's used only as `arr[i]`/`arr[const]` (peephole `LoadLocal(arr);
LoadLocal|LoadConst; GetIndex`); ambiguous/complex-index/written params → bail. `arr[i]` lowers to a
bounds-checked native f64 load. The VM wrapper (`try_call_array_jit`) splits args, extracts each
all-numeric `Value::Array` into a scratch `Vec<f64>`, and calls.

CORRECTNESS (a JIT miscompile is SILENT — validated hard): purely ADDITIVE + bail-safe, three guards
keep it bit-exact with the interpreter:
  • non-numeric element → wrapper returns None → interpreter (e.g. `[1,2,"x",4]` → `"3x4"`).
  • out-of-bounds read → JIT sets the deopt byte & returns → wrapper re-runs the interpreter (OOB →
    `Value::Null`, whose per-operator coercion the JIT can't replicate).
  • `NumberArray` (packed, NaN=hole) → wrapper bails (hole semantics differ).
Validated: full cross-backend suite **17/0 both default-on AND `TISH_JIT_ARRAYS=0`**; new fixture
`tests/core/jit_arrays.tish` pins vm(JIT) ≡ interp ≡ node ≡ all 6 backends (sum/dot/max/const-index/
scale/float/empty); micros unchanged off-vs-on (it only fires on the target pattern, bails otherwise).
`TISH_JIT_ARRAYS=0` is the escape hatch. Carries to cranelift/wasi/llvm (they embed the VM).

TWO PRE-EXISTING divergences surfaced (orthogonal, spun off as tasks, NOT array-JIT bugs): the
eval-interpreter ERRORS on `number + null` where VM/node give NaN; the rust-AOT backend PANICS on a
string element in a numeric-typed reduction (its inference assumes f64). Both excluded from the parity
fixture (a cross-backend fixture can only hold cases all backends agree on).

################################################################################
##  ★ FLAGSHIP WIN — recursion-JIT via SelfCall (2026-06-06). fib(35) BEATS Node.
################################################################################
The numeric JIT now compiles SELF-RECURSIVE calls to native cranelift recursion. A new `SelfCall`
opcode is emitted by the compiler for `fn NAME(...)` bodies that call `NAME(args)` directly, ONLY when
NAME's binding is provably stable (not shadowed by a param, not reassigned/redeclared anywhere in the
body — conservative `stmt_rebinds` scan; a wrong SelfCall would be a miscompile). The VM runs SelfCall
as a direct recursive `run_chunk`; the JIT lowers it to `bcx.ins().call(self_ref, …)` (the function's
own FuncId, forward-referenced, resolved at finalize). `build_body_cfg`'s `!has_loop` bail was relaxed
to also accept `has_self_call`, so branch+recursion functions like `fib` (no loop) now JIT.

  RESULT:
    fib(30):  329ms → 5ms        (66×)
    fib(35):  tish 43ms  vs  Node 54ms   ← tish BEATS V8 on recursive fib
    recursion_stress depth-500 tail recursion: CRASHED (stack overflow) → 0ms (native frames are
      tiny, so the JIT also FIXES the overflow for JIT-eligible numeric recursion)
  CORRECTNESS (the JIT-recursion path is critical — a miscompile is silent): vm ≡ interp on fib, fact,
  pow2, collatz, gcd, ackermann, sumDigits, sumTo(1000); the reassignment case `tricky` (name rebound
  mid-body) correctly does NOT self-call; mutual recursion (isEven/isOdd) correctly stays normal calls;
  full suite 17/0; cranelift inherits it (embeds the VM). Additive: a non-eligible SelfCall bails to the
  VM. Carries to cranelift/llvm; wasi runs the (correct, non-JIT) VM SelfCall.

================================================================================
  WIN — JIT arity ceiling 3 → 8 (multi-arg numeric kernels) (2026-06-06)
================================================================================
`try_compile_numeric` bailed on `param_count > 3`; `NumericFn::call` only dispatched arities 1–3.
Both x86-64 SysV and AArch64 pass up to 8 `f64` in registers (XMM0-7 / V0-7), so extending to 8 is
ZERO correctness tax — the `extern "C"` transmute stays sound (all args register-passed). Now
multi-arg numeric functions (geometry/physics/matmul kernels: `dist3(x1,y1,z1,x2,y2,z2)`, `clamp5`,
`lerp4`, `poly8`) are JIT-eligible.
  - Latent panic fixed: the caller-side arg buffer was `[0f64; 4]`; the `for i in 0..arity` write would
    index out of bounds for arity 5–8. Now `[0f64; 8]` (sized to the ceiling).
  RESULT: multi-arg RECURSION shines (SelfCall → native): `sum6(20,…)` (6 args) interp 2055ms → vm
    **34ms (60×)**; values match interp exactly. Suite 17/0.

================================================================================
  PASS FINDING — the call-overhead wall (~275ns/call) is the frame-VM's justification (2026-06-06)
================================================================================
A high-level "what are we missing" pass, grounded in two fresh profiles:
  1. MIXED real-world code (objects + strings + calls): the leaf profile is FLAT — ~97% inside
     `run_chunk` with tiny spread leaves (malloc 3, hashmap.get 2, drop<Value> 2, memmove). Real
     object/string code is INTERPRETER-DISPATCH-bound (death by a thousand opcodes, no hot spot).
     → broad lever = SMALLER Value (NaN-box / thin variants 24→16→8): every clone/push/pop/dispatch
       moves less memory. All-or-nothing ~600-site change.
  2. COMPUTE with function calls: `add2(a,b)` (trivially JIT-eligible) loop-called 20M times = 5501ms
     (~275ns/call) — IDENTICAL wall whether the body is arity-2 or arity-8. The JIT'd body is fast; the
     CALL path (LoadVar lookup + `Arc<dyn Fn>` closure-wrapper dispatch + per-call arg marshalling +
     result wrap) dominates. Only RECURSIVE (SelfCall→native) or INLINED (no-call JIT loop) calls escape
     it. → broad lever = FRAME-VM (one VM + a CallFrame stack; kill per-call Vm + closure-wrapper
     dispatch; also fixes non-numeric recursion overflow). ~900-line core-loop rewrite.
  Achievable-now JIT extensions are NARROW + carry disproportionate correctness tax: Math.* intrinsics
  need a precise program-wide "is `Math` reassigned" AST walker (~80-100 lines) to avoid a silent
  miscompile, for code the benchmarks don't even exercise; known/mutual-fn calls need cross-fn FuncId
  ordering (batch declaration). Verdict: the two BIG broad wins (NaN-box Value, frame-VM) are the real
  levers — both multi-session reps. The 275ns/call number is the concrete frame-VM justification.

################################################################################
##  VALUE-SHRINK rearchitecture, step 1: String → thin `ArcStr` (2026-06-06)
################################################################################
The profile is unambiguous: even object_stress is INTERPRETER-DISPATCH-bound (`run_chunk` ~76% flat,
alloc/PropMap/Mutex all ≤3 samples). The one broad lever is a SMALLER `Value` (every opcode clones /
pushes / pops a 24-byte `Value`; a `Number` does a 24-byte memcpy to move an 8-byte float).

STEP 1 DONE + VALIDATED (17/0 all backends incl rust-AOT): `Value::String(Arc<str>)` → `String(ArcStr)`
(the `arcstr` crate — a thin 8-byte `Arc<str>`, transparent `Deref<str>`). ~60 sites across 6 crates,
compile-guided; `ArcStr` re-exported from tish_core + tish_runtime; rust-AOT codegen char-template fixed
(`std::sync::Arc::from` → `ArcStr::from`); object KEYs stay `Arc<str>` (converted only at the rare
dynamic-key boundary). String payloads now clone as 8 bytes + one atomic vs `Arc<str>`'s 16-byte fat
pointer.
  MEASURED: object_stress 51ms (≤ baseline 60, NO regression); bundle 160-170ms vs 180 baseline. HONEST
  CAVEAT: object_stress also improved without an object change → some of the bundle delta is machine
  state; the ArcStr clone win is mechanistically real but its isolated magnitude isn't cleanly proven
  (no apples-to-apples revert-compare on this tangled tree). Net: validated, correct, no regression.
  STILL 24 BYTES: `Value` size is unchanged because the `Arc<dyn>` variants (`Function`/`Promise`/
  `Opaque`, 16B) still cap it. So the BROAD dispatch win is NOT banked yet.
STEP 2 (24→16) — ATTEMPTED, MEASURED AS A REGRESSION, REVERTED (2026-06-06). Thinned all 3 `Arc<dyn>`
variants (`Function`/`Promise`/`Opaque`) to `Arc<Box<dyn>>` (thin 8B handle: the box makes the Arc thin;
clone stays a refcount bump, the box is never copied). Implemented cleanly via a `boxed_promise`/
`boxed_opaque` helper + a `NativeFn = Arc<Box<dyn Callable>>` alias; ~30 sites; the eval↔core boundary
kept consistent (eval's own Value variants thinned too so the conversion is a plain `Arc::clone`).
  RESULT: `Value` DID drop to **16 bytes** and the full suite stayed **17/0 green on all 6 backends**.
  BUT it REGRESSED numeric dispatch ~8–10%, proven by a rigorous A/B (two binaries, INTERLEAVED min-of-N
  so machine drift hits both equally):
    • large WS (3M-elem `Vec<Value>`, 72MB→48MB, memory-heavy):  24B = 1783ms,  16B = 1957ms  (+9.8%)
    • small WS (1000 elems, fits L1, ZERO memory pressure):       24B = 4435ms,  16B = 4812ms  (+8.5%)
  The small-WS case is the smoking gun: with the working set in L1 there is NO memory-bandwidth effect,
  yet 16B is STILL ~8.5% slower → the cost is in DISPATCH, not memory traffic. Shrinking `Value` does not
  help because tish is dispatch-bound on per-opcode WORK, not bandwidth-bound on `Value` size; the
  boxing/enum-layout change pessimized the hot `Number` extraction path (likely a niche/discriminant-offset
  effect once every non-`Number` variant became a thin pointer). On the micros the change was perf-NEUTRAL
  (object_stress/array_stress/etc. all within ±1ms interleaved) — they fit in cache, so neither the size
  win nor the dispatch loss showed; only the bigger numeric loops exposed the loss.
  IMPLICATION (reshapes the NaN-box plan): a smaller `Value` is NOT a win on its own for this VM. NaN-box
  (16→8) would only pay off via its OTHER property — a branch-free `is_number` TAG TEST replacing the enum
  match — i.e. a DISPATCH change, not a size change. Pursue NaN-box only if that tag-test path is the goal;
  do NOT chase `Value` size for its own sake. Reverted to the fast 24B state (parity re-verified 17/0).
  The real broad lever is reducing dispatch WORK: wider JIT coverage (array-element access in JIT'd loops)
  or superinstruction fusion — see the JIT sections above.

################################################################################
##  FRAME-VM (task #39) — BUILT + VALIDATED, flag-gated `TISH_FRAME_VM` (2026-06-06)
################################################################################
The iterative frame-stack execution path is built and validated. Calls + recursion run on a heap
`CallFrame` stack instead of recursively re-entering `run_chunk` (no per-call `Vm`, no native-stack
growth). Three pieces, all landed + green:
  1. `Callable` trait (tish_core): `NativeFn` = `Arc<dyn Callable>` (was opaque `Arc<dyn Fn>`), so a
     tish-closure exposes its chunk via `as_any` downcast. + `FnCallable` adapter + `native_fn()`.
     Fixed ~25 sites across 5 crates + the rust-AOT codegen (`f(req_args)`→`f.call`).
  2. `VmClosure` (tish_vm): every tish fn is now this — chunk (`Arc<Chunk>`) + captured chain + jit_fn
     + a precomputed `frameable` flag. `call()` is the fallback (= old behaviour, byte-identical).
  3. `run_framed`: the Call opcode (flag-on) downcasts a frameable callee and runs it on the frame
     stack — push on Call/SelfCall, pop on Return. `code` is bound once per frame (laundered borrow,
     re-derived on frame switch) to keep the hot opcode path Arc-deref-free.
VALIDATION: flag-off 17/0 (byte-identical) AND flag-on 17/0 (full corpus, incl. the unsafe binding);
flag-on ≡ interp on mutual recursion / helpers / fib; `ev(100000)` deep mutual recursion runs flag-on
with ZERO overflow (the wasi-trap + overflow fix made real, since the frame loop needs no JIT/stacker).

HONEST PERF VERDICT (data-backed) — a ROBUSTNESS mechanism, NOT a speed win. Keep flag-gated OFF.
  shallow mutual recursion (ev(20)×200k = 4.2M calls): flag-off ~660ms, flag-ON ~720ms (~9% SLOWER).
  deep mutual recursion (ev(20000)×2k = 80M calls):     flag-off 25.6s, flag-ON 24.5s (~5% faster).
  THREE optimizations tried — none closed the shallow gap: (1) bind `code` once per frame (laundered
  ptr, kills per-opcode Arc-deref); (2) slot-region pooling (shared `slots` Vec + per-frame base, no
  per-call slot alloc); (3) move-not-clone the caller frame on push (halve Arc traffic). flag-ON stayed
  ~9% slower. CONCLUSION: a heap CallFrame stack costs ≈ the recursive path's native stack + `Vm` for
  SHALLOW calls — it is NOT cheaper; the premise "killing the per-call Vm makes calls faster" is FALSE
  for this VM. The frame loop only wins where the native stack genuinely can't go: wasi deep recursion
  (stacker is a no-op on wasm) + unbounded depth without `maybe_grow`. That's a narrow robustness niche.
  ALSO: the ~275ns/call wall (jit'd `add2` loop-called) is JIT-DISPATCH overhead (LoadVar global lookup +
  `Arc<dyn Callable>` vtable + arg-number-check + result-wrap) — jit'd fns never create a per-call Vm, so
  the frame-VM was aimed at the wrong target for it. The real call-overhead levers are the JIT-dispatch
  path (for jit'd fns) and the closure-invocation model (shared by both exec paths), NOT per-call Vm.
  STATUS: built, validated (17/0 flag-off + flag-on), flag-gated OFF (zero shipped impact). The Callable
  trait + VmClosure are kept (cleaner than `Arc<dyn Fn>`, useful foundation); run_framed stays as the
  wasi/deep-recursion option. Not promoted to default — it would regress the common path.

================================================================================
  CORRECTNESS FIX — interp deep-recursion stack-overflow guard (2026-06-06)
================================================================================
The tree-walk interpreter had NO native-stack guard, so deeply recursive code (`fn deep(n){…deep(n-1)}`)
aborted the process with SIGABRT ("stack overflow") — debug at <100 levels, release at higher depths —
where the VM completes (its JIT lowers recursion to tiny native frames). Surfaced by the cross-backend
parity run on `recursion_stress`. FIX: `stacker::maybe_grow` around the body eval in `call_func` (the
recursion ACCUMULATOR; deliberately NOT in `call_with_scope`, the per-element HOF callback path, to keep
map/filter hot loops check-free). TUNING: the VM's 128 KiB red-zone is too small for the interp — one
tree-walker level spans a long eval chain (eval_statement→eval_expr×N→call_func), so the stack overflows
BETWEEN checks; **1 MiB red-zone + 16 MiB segments** (verified to depth 20000, debug + release). Suite
17/0, interp call throughput unchanged (~345ns/call; the per-call stack check is negligible).

KNOWN LIMITATION (defer to frame-VM #39): the **wasi** backend still traps ("call stack exhausted") on
deep recursion — its `run_chunk` re-entry maps each tish level to a wasm CALL frame (wasmtime exhausts
~313), stacker is a no-op on wasm32, and there is no JIT. A depth-limit band-aid would wrongly throw on
depth-500 recursion that node/vm/interp complete, so the proper fix is the frame-VM (iterative heap
frame-stack, no wasm call frames) — a SECOND concrete justification for #39 alongside the 275ns/call wall.

================================================================================
  WIN (small) — per-call allocation cuts: shared `enclosing` + lazy `local_scope` (2026-06-06)
================================================================================
Two safe, compiler-/suite-verified cuts to the per-call cost in `run_chunk`:
  1. `Vm.enclosing`: `Vec<ScopeMap>` → `SharedChain` (Rc/Arc<Vec>). It is never mutated after a
     closure is built, so the per-call propagation is now a refcount bump, not a Vec deep-clone.
  2. `local_scope`: eager `VmRef::new(Arc<Mutex<HashMap>>)` → lazy `Option`, created on first
     write/capture. Slot-based chunks (the hot path + all JIT-eligible fns) never write it, so they
     skip the box entirely. Safe because slot-based ⟹ no captured locals + StoreVar falls through.
  RESULT: 0-arg call 5M: 764→708ms; 1-arg: 826→776ms (~7%). Closure-capture correctness verified
  (vm≡interp on closure_capture_cell/loop_let_capture/arrow_functions/mutation); suite 17/0.
  The tracked object/array micros do NOT move (they aren't call-bound). `fib(30)` stays ~329ms vs
  Node ~5ms — confirming the per-call micro-opts only nibble; the closure-invocation MODEL is the wall.

================================================================================
  FINDING — function-call overhead is architectural (call-frame model), 2026-06-06
================================================================================
Investigated the call boundary (benchmark_granular's nested-fn lines, ~6ms; the broader concern that
"every function call is slow"). Measured per-call cost ≈ 150-165 ns (5M calls): a 0-ARG call is 764ms,
a 1-arg call 826ms — so the cost is NOT the args `Vec` allocation (only ~60ms of the 800ms). It is the
closure-invocation model itself:
  1. The function NAME is a global → resolved via a hashmap lookup on every call (LoadVar).
  2. Every tish function is a `Value::native(Arc<dyn Fn>)` → dynamic dispatch per call.
  3. NON-JIT functions create a fresh `Vm` struct per call, with `enclosing_chain.clone()` (a Vec
     clone), `globals.clone()`, capabilities/native_modules clones (vm.rs ~1091).
None of these is individually dominant; together they are the call tax.

Attempted a JIT call-inlining (JIT'd fn → direct cranelift `call` to a JIT'd callee). REVERTED: it is
correct but NARROW (both fns must be numeric + JIT-eligible; does not cover self-recursion, since a
function isn't cached while being compiled) and needs a `Chunk` name→nested-closure map to fire
reliably (bytecode can't be walked without a full instruction-size table). Not worth the structural
cost for the rare pattern.

The real lever for call overhead is a CALL-FRAME REDESIGN: an explicit heap call stack instead of a
per-call `Vm` struct + Arc-dyn-dispatch. This is the SAME work the recursion-overflow bug needs (the VM
overflows the native stack at depth ~500 because each tish call = a Rust recursion). Do both together:
a real frame stack fixes the crash AND removes the per-call clone/dispatch tax. Multi-session.

================================================================================
  PHASE 2 — packed f64 arrays LANDED (TISH_PACKED_ARRAYS=1, default=off) (2026-06-06)
================================================================================
`Value::NumberArray(VmRef<Vec<f64>>)` added. Deopt wrapper in all array builtins (as_boxed_array).
VM fast paths: GetIndex, SetIndex, In, push/pop/shift/unshift/reverse/splice/sort, fused opcodes
(ArrayMapBinOp/ArrayFilterBinOp/ArrayMapIdentity), ConcatArray deopt, Array.isArray. Sparse-assign
handled via NaN-as-hole marker. Suite 17/0 at flag=0 (byte-identical to interp). Correctness:
  - flag-off (default) identical to .expected for all core fixtures ✅
  - flag-on: all core fixtures output-identical to flag-off EXCEPT 2 edge cases:
    `splice(1,3,"a","b")` on a NumberArray (non-numeric values into packed Vec = stored as NaN holes;
    semantic loss is expected and documented — mixed-type splice requires scope-binding update to
    fully deopt, which `&Value` can't do without architectural changes).

FINDING — does NOT move array_stress (32ms → 32ms with flag=1). Root cause:
  - array_stress builds arrays via `push()` into empty `[]` literals
  - Empty `[]` stays regular Array (can't infer numeric from zero elements)
  - `sort_numeric_impl` on all-numeric regular Arrays already does unbox→sort→rebox, near-equivalent
  - NumberArray only created for NON-EMPTY ALL-NUMERIC array literals `[1,2,3]`

The infrastructure is CORRECT and benefits code with explicit numeric literals (e.g. benchmark suites
that use `[1,2,...,N]` syntax) and sort/HOF chains starting from those. For the `array_stress`
benchmarks to benefit: need to upgrade push-accumulated arrays to NumberArray (not done — requires
a different mechanism than &Value mutation, e.g. a "tagging" approach on the VmRef).

--------------------------------------------------------------------------------
  PHASE 2b — packed-native `Float64Array` (rust-AOT codegen only) (2026-06-10)
--------------------------------------------------------------------------------
Extends packed arrays to the typed-array constructor on the NATIVE rust-AOT path: `new Float64Array(...)`
lowers in codegen.rs (the real `Expr::New` emit site) to `tishlang_runtime::float64_array_packed(&[...])`
instead of the generic `tish_construct` → boxed `Value::Array`. The helper builds a `Value::NumberArray`
(`Vec<f64>`) directly when `TISH_PACKED_ARRAYS=1`, and returns the BYTE-IDENTICAL boxed value when the
flag is off (default), so stock builds are unchanged. `Float64Array` is the only view whose element type
IS `f64` — no coercion — so it maps onto the existing `NumberArray` with zero element work. The integer /
`Float32Array` views have no packed `Value` variant (would need `Vec<i32>`/`Vec<f32>`/… + the 24-byte
size assertion + every exhaustive match — a separate, larger effort) and keep the boxed path.

  WHY NATIVE-ONLY: the interp (`tishlang_eval::Value`) has no `NumberArray` variant and the core↔eval
  value bridge can't carry one, so the runtime constructor still returns boxed `Value::Array` for all
  backends; only codegen special-cases it. Consequence: on the native path a `NumberArray` is ALWAYS a
  `Float64Array` (codegen never packs array literals here), so the native runtime additions are sound.

  NATIVE RUNTIME ADDITIONS (tish_runtime/src/lib.rs — the VM had these, the rust runtime did NOT):
    get_prop (`.length` + numeric key), get_index, set_index, in_operator → NumberArray arms; plus the
    emitted for-of `match &_fof` grew a `Value::NumberArray` arm (codegen.rs). set_index stores the f64
    (`val.as_number().unwrap_or(NaN)`) — because NumberArray≡Float64Array here, this is the CORRECT view
    semantics and incidentally closes the v1 "no write-coercion" gap for this one view.

  PACKED HOF FAST PATH (follow-up, same session): the array HOFs in tish_builtins/src/array.rs no longer
  `as_boxed_array`-materialise a NumberArray before iterating. `packed_snapshot(arr, cb)` takes a cheap
  `Vec<f64>` snapshot (8 B/elem memcpy, no per-element Value construction) and the method folds/scans it,
  boxing one `Value::Number` at a time for the callback. Covers reduce/map/filter/for_each/find/find_index/
  some/every. Snapshot (not a held borrow) matches the boxed copy semantics and can't deadlock on a
  re-entrant callback. Identical results to the boxed path (unit tests: array::packed_hof_tests).

  PACKED RESULTS — map/filter STAY packed (2nd follow-up): `filter` keeps a subset of the input f64s, so
  it builds the result `Vec<f64>` directly → NumberArray. `map` speculatively builds a `Vec<f64>` and
  deopts to a boxed `Vec<Value>` on the FIRST non-numeric callback result (each element's callback still
  runs once, in order). So a numeric `map`/`filter` returns a NumberArray and chains stay packed end-to-end
  (empty results stay boxed, per convention). A NumberArray is observably identical to a boxed array of the
  same numbers (verified: display / JSON.stringify / index / for-of byte-identical across interp/vm-on/vm-off).

  BENCHMARK (/tmp/f64_bench.tish: N=1,000,000, 30 rounds; one binary, flag toggles at runtime; M-series):
    op                       boxed (flag=0)   packed (flag=1)   ratio
    construct(N) zero-fill        ~90 ms           ~1.5 ms       ~60×    ★ headline: memset vs N boxed Values
    construct(from 1M src)        ~7 ms            ~5 ms         ~1.3×
    index-sum  (big[i])           ~540 ms          ~499 ms       ~1.08×  denser scan; per-elem rebox dominates
    forof-sum  (for x of big)     ~188 ms          ~173 ms       ~1.09×
    reduce     (a,b)=>a+b         ~378 ms          ~327 ms       ~1.16×  WAS ~0.86× regression; HOF fast path flips it
    filter     (x>500)            ~323 ms          ~300 ms       ~1.08×  no input materialisation
    map(x*2).reduce               ~593 ms          ~541 ms       ~1.10×  map builds Vec<f64> direct, result stays packed
    filter(>500).map(+1).reduce   ~638 ms          ~515 ms       ~1.24×  whole chain stays packed end-to-end
  Output (sums/lengths) byte-identical between modes. Memory: 8 B/elem vs 24 B/elem = 3× denser.

  TAKEAWAY: the win is CONSTRUCTION (no per-element boxing, ~60×) + 3× footprint; scans gain cache density
  ~8-9% (the loop body re-boxes each elem to `Value::Number`). With the HOF fast paths, reduce/filter/map
  on a packed array BEAT boxed (no materialisation deopt), and map/filter now RETURN packed so chains stay
  packed end-to-end (1.10–1.24× on chains). Remaining ceiling: the per-element rebox in scans — a fully-
  unboxed numeric loop needs typed codegen (the M-series/typed-native path). Enable for Float64-heavy
  compute/memory-bound code.

################################################################################
##  WIN — parking_lot::Mutex on the send-values path (2026-06-06). 2nd profile lever.
################################################################################
The post-mimalloc profile (same heavy object/array workload) exposed `pthread_mutex_lock`/`unlock`
(~47 samples incl. stubs) as the #2 cost: under `send-values` (the shipped `full`/http build), every
object/array access goes through `Arc<Mutex<T>>`, and `std::sync::Mutex` on macOS makes a pthread
syscall even uncontended. Swapped to `parking_lot::Mutex` (crates/tish_core/src/vmref.rs, gated by the
existing `send-values` feature; parking_lot was already in the tree): uncontended lock = one atomic,
no syscall, no poisoning. Drop-in — still `Send + Sync`, behaviour identical (the std path already
swallowed poison). Full suite 14/0, vm≡interp clean.

  RESULT vs the post-mimalloc numbers (same harness):
    micro                mimalloc   +parking_lot
    object_stress          59          57
    array_stress           31          30
    new_features_perf      33          28     (−15%; access/mutex-heavy)
    benchmark_granular     51          48
    main.tish bundle      180         170
  CUMULATIVE this session vs the FROZEN baseline (two transparent, zero-risk runtime wins):
    object_stress 69→57 (−17%), array_stress 40→30 (−25%), new_features 47→28 (−40%),
    benchmark_granular 67→48 (−28%), main.tish BUNDLE 230→170 (−26%).
  The two transparent allocator/lock wins (mimalloc + parking_lot) are now harvested — the profile's
  top-2 costs. What remains (allocation COUNT, refcount) needs the bigger representation/arena work.

################################################################################
##  WIN — mimalloc global allocator (2026-06-06). The profile-driven lever.
################################################################################
A sampling profile (macOS `sample`, symboled release) of an object/array-heavy workload showed it is
**allocation-bound**: ~220 of the hot samples are in the system allocator (`_xzm_xzone_malloc_tiny`
51, `_xzm_free` 45, `_xzm_xzone_malloc` 20, `_malloc_zone_malloc` 20, `__bzero` 29, `_free` 19, …)
plus refcount drops (`drop_in_place<Value>` 32, `Arc::drop_slow` 11) — and ~0 in opcode dispatch.
Pure arithmetic is dispatch-bound (→ the JIT, already landed); object/array/string churn is
**malloc-bound**. tish was using the system allocator; **JSC ships its own (bmalloc)** for exactly
this reason.

Fix: `mimalloc` as the process `#[global_allocator]` — on BOTH paths:
  • `tish run` (the VM): crates/tish, `fast-alloc` feature in `default`; `--no-default-features` → system.
  • `tish build --native-backend rust` (rust-AOT output): crates/tish_native/src/build.rs injects the
    dep + a `#[global_allocator]` into the generated crate root (binary output only, native only;
    `TISH_NATIVE_FAST_ALLOC=0` opts out). Even bigger there — the AOT path has no interpreter overhead,
    so allocation is a larger fraction: an object/Object.entries workload went 944→564ms (**−40%**),
    result byte-identical to interp.
Semantically transparent — only changes which malloc backs every allocation. Differential vm≡interp
clean on object_stress/array_stress/new_features; rust-AOT acc ≡ interp.

  RESULT vs FROZEN baseline (target/release/tish run, best-of-3 Σms; bundle = best-of-5 wall):
    micro                BEFORE   AFTER   Δ
    object_stress          69      59    −14%
    array_stress           40      31    −22%
    new_features_perf      47      33    −30%
    benchmark_granular     67      51    −24%
    main.tish bundle      230     180    −22%   <- the real-world workload
  (All rows: shipping release config, system-malloc BEFORE vs mimalloc AFTER, no debug symbols on
  either — apples-to-apples.) Every tracked micro AND the bundle beat the frozen baseline. This is
  the single highest
  win/effort change of the JSC work: one transparent dependency, no representation change, no risk.

  SYNTHESIS (the corrected strategy, from 5 measurements): tish's perf gap is TWO distinct levers,
  by workload — (1) PURE COMPUTE is dispatch-bound → the numeric/control-flow JIT (landed, 89×→1× on
  numeric loops); (2) OBJECT/ARRAY/STRING churn is ALLOCATION-bound → a fast allocator (this) +
  representation changes that REDUCE allocation count (packed f64 arrays = one buffer not per-element
  boxing; shape objects = no per-key storage). Property-lookup ICs (Phase 1a) and NaN-boxing address
  neither dominant cost directly (lookups aren't hot; NaN-box shrinks Values but not alloc COUNT), so
  they rank below the allocator + packed arrays. The allocator is now on BOTH the VM and rust-AOT
  paths (above). NEXT highest-confidence: Phase 2 packed f64 arrays (now clearly motivated: cuts alloc
  count) and the rust-AOT struct/array inference that already exists behind flags.

================================================================================
  PHASE 1a — shapes + inline caches LANDED, but NOT the object_stress lever (2026-06-05)
================================================================================
Implemented JSC-style object shapes (`tish_core/src/shape.rs`: interned ShapeId + structure
transitions) + a per-name inline cache on `GetMember`/`SetMember` (`Chunk.inline_caches`, atomic,
serde-skip). CORRECT: full cross-backend suite 14/0; vm ≡ interp on an IC stress (mono/poly shapes,
update, delete→DICT, missing, nested, array-of-objects). Additive — `DICT_SHAPE`/non-object/miss fall
to the existing path.

  RESULT vs BEFORE baseline: object_stress 69→72ms (≈flat), bundle 230→240ms (≈flat). **The IC did
  NOT move the tracked numbers.** Measured why: object_stress's hot sections are `Object.entries`/
  `Object.keys` for-of (build arrays), spread+extend, `Object.assign`, destructure — BULK ops that
  allocate, NOT `o.x` access in a loop. And an access-heavy micro (3M × 4 reads) is 501ms vs Node 5 —
  the interpreter loop + boxing dominate, not the property lookup.

  STRATEGIC FINDING (reshapes the ENTIRE JSC plan — backed by 4 measurements): tish's micro gap to
  Node/Bun is dominated by **interpreter dispatch overhead** (per-opcode match + stack push/pop +
  ip-advance) and allocation — NOT value representation (boxing / shapes / NaN-box). Proof:
    1. The access IC (faster lookup) did not move object_stress (72 vs 69) or the bundle (240 vs 230).
    2. object_stress hot sections are Object.entries/keys (build arrays), spread, assign, destructure —
       allocation + iteration, not `o.x` lookups.
    3. THE CLINCHER — same arithmetic, two ways: `arr.map(x=>x*2)` ×5000 (the FUSED `ArrayMapBinOp`
       opcode, one dispatch per call) = 40ms ≈ 8ns/op, near-native; the IDENTICAL work as an inline
       interpreted index loop (one dispatch *per element op*) = 646ms ≈ 130ns/iter. **16× — and the
       only difference is dispatch count.** Element boxing is identical in both; representation is not
       the variable.
    4. This session's control-flow JIT crushed numeric loops 89×→1× by ELIMINATING dispatch — the same
       lever, proven.

  Therefore the high-leverage direction is **dispatch elimination**, not representation:
    • Extend the proven JIT beyond numeric-only function bodies → top-level hot loops, more callback
      shapes, object/array ops (with the IC from 1a as the in-JIT property fast path). The 89× lever.
    • More fused super-opcodes for common non-fused HOF shapes (map-returning-array for `.flat()`,
      map-to-property) — each amortizes dispatch over N elements like ArrayMapBinOp already does.
  The JSC representation plan addresses a smaller fraction: Phase 1b (butterfly) helps construction/
  spread ~30% but objects stay Arc<Mutex>; Phase 2 (packed arrays) removes element boxing — real but
  secondary to dispatch; Phase 3 (NaN-box) halves per-op memory traffic but not dispatch *count*.
  Phase 1a (the IC) is kept as correct infrastructure (it is the in-JIT property fast path the
  extended JIT will use, and it speeds property-access-heavy REAL code) — it is simply not, by itself,
  the lever these benchmarks needed.

################################################################################
##  FROZEN BASELINE (2026-06-05) — BEFORE shapes/inline-caches, packed arrays,
##  NaN-boxing. This is the line the JSC/Bun-guided work (docs/jsc-bun-perf-
##  guidance.md, plan: shapes+ICs → packed arrays → NaN-box) MUST beat. Captured
##  AFTER this session's slot-based locals + control-flow JIT already landed.
################################################################################

darwin-arm64, release. THIS IS "BEFORE". Each phase appends an "AFTER" block above
and must show its target micro faster than these numbers, suite still 14/0.

BUNDLED PERF SUITE (tests/main.tish, sustained, 5-run avg, ms) — run_performance_suite.sh --release:
  backend       ms     vs Node
  rust (AOT)   114     1.5x
  vm (default) 252     3.3x   <- the JSC work targets this via objects+arrays+Value
  interp       326     4.3x
  cranelift    263     3.5x   (embeds the VM → inherits the wins)
  llvm         260     3.4x   (embeds the VM)
  wasi        1308    17.2x
  Node          76     1.0
  Bun           58     0.76x  <- the real target
  Deno          69     0.91x
  QuickJS      263     3.5x

COMPUTE MICROS (vm, internal Date.now Σms, startup excluded, 3-run best):
  micro                vm     Node   Bun    vm/Node   vm/Bun
  object_stress        69ms   6      6      11.5x     11.5x   <- Phase 1 target (≤35ms)
  array_stress         40ms   12     10      3.3x      4.0x   <- Phase 2 target (≤22ms)
  new_features_perf    47ms   6      6       7.8x      7.8x
  benchmark_granular   67ms   6      8      11.2x      8.4x
  main.tish bundle (vm wall-clock, 3-run best): 230ms

Reproduce a micro: target/release/tish run tests/core/object_stress.tish (self-times
each section via Date.now); sum the `…ms` lines. Numeric loop fns already match Node
(sumTo 62ms) via the control-flow JIT below — the remaining gap is objects/arrays/Value.

================================================================================
  VM CONTROL-FLOW JIT — numeric LOOP functions now run at native speed (2026-06-05)
================================================================================
Extended the Cranelift numeric JIT (`tish_vm/src/jit.rs`) from straight-line/ternary leaf functions
to **functions with loops + branches** (`build_body_cfg`). A cranelift `Variable` per frame slot
(loop-carried locals; cranelift inserts the SSA phis at `seal_all_blocks`) + one block per bytecode
jump target. Handles for/while/do-while/nested loops, if/else, early-return, break, continue.
**Unblocked by slot-based locals** — those made such functions `slot_based` (JIT-eligible) and gave
the JIT slot indices to consume. Default-on, additive (a bail runs the VM; only a logic bug here is
wrong → differential-validated).

  THE WIN (default `tish run`; lifts cranelift/wasi — they embed the JIT):
    fn sumTo(n){let s=0;for(let i=0;i<n;i++){s=s+i*2-1}return s}, 500×100k iters:
        5530ms (interpreted, ~89× Node)  →  71ms (native, ≈1.1× Node 63ms)  =  78× FASTER.
    collatz kernel (longer per-call loop): tish 146ms ≈ Node 134ms — MATCHING V8.
  Numeric loop functions now run at ~native speed. (Tiny-loop-called-Nx — e.g. fib(20)×3M — is
  CALL-boundary-bound, not loop-bound: 532ms vs 18ms; the per-call box/dispatch dominates → next
  lever is the call boundary, a separate win.)

  CORRECTNESS (a JIT miscompile = silent wrong result, so validated hard): vm(JIT) ≡ interp(no JIT)
  ≡ node on a differential fuzz over arithmetic/mod/div(float)/negatives/if-else/nested-loops/while/
  early-return/break/continue/2-param/bitwise; full cross-backend suite 14/0; `jit_regression` +
  new `tests/core/jit_loops.tish`. Conservative: bails (→VM) on member/call/array/object, booleans
  in slots, a ternary-merge inside a loop (non-empty operand stack at a block boundary), unknown
  opcodes. KEY BUG fixed in bring-up: the compiler appends an unreachable trailing `LoadConst Null;
  Return` after an explicit `return` — must NOT be a block leader (only real jump targets are), else
  the JIT bails translating the unreachable Null. Skipped as dead tail instead.

================================================================================
  VM SLOT-BASED LOCALS — NOW DEFAULT-ON + SCOPE-AWARE + TOP-LEVEL (2026-06-05)
================================================================================
Extended the slot work to a proper SCOPE-AWARE allocator (block-scoped `slot_scopes` stack
→ correct shadowing/sibling-`let i`) + TOP-LEVEL slotting (non-REPL) + broadened eligibility
(SlotScan now recurses for-of/try/switch/throw/destructure/`||=` for a complete capture set,
bailing only on ambient/module constructs). Flipped **DEFAULT ON** (`TISH_VM_SLOTS=0` disables).

  MEASURED WINS (default `tish run`, vm; lifts cranelift/wasi too — they embed the VM):
    object_stress       95→71ms  (-25%)     new_features_perf   59→45ms  (-24%)
    array_stress        50→39ms  (-22%)     benchmark_granular  89→65ms  (-27%)
    main.tish BUNDLE    0.32→0.25s (-22%)   ← the headline sustained-compute number

  WHY IT NOW MOVES THE BUNDLE/MICROS (the earlier flag-gated version didn't): the micros are
  TOP-LEVEL scripts (now slotted) and the bundle's `__perf_run_modules_*` use for-of/try (now
  handled, not bailed). Pure numeric loops are still ~Node×N (boxed-Value arithmetic remains →
  that's #14 JIT; slots remove name-resolution overhead, the necessary first layer).

  CORRECTNESS: full cross-backend suite 14/0 BOTH default-on AND `TISH_VM_SLOTS=0`; `main.tish`
  bundle ≡ interp; `tests/core/slot_capture.tish` (closure/FunDecl param-default capture, forEach
  capture, shadow-across-closure). Two bugs caught+fixed in validation: (1) `Expr::Ident` read used
  the simple flat map not `resolve_slot` (general-slot reads → LoadVar → undefined); (2) `||=`/`&&=`/
  `??=` compile site wasn't slot-aware. The single source of truth is `resolve_slot` (checks the
  simple param map THEN the general scope stack); every var read/write routes through
  `emit_var_load`/`emit_var_store`. Captured locals (referenced by a nested closure's body or param
  defaults) stay name-based in `local_scope` (which closures capture) — VM needed ZERO changes.

================================================================================
  VM SLOT-BASED LOCALS LANDED (2026-06-05) — TISH_VM_SLOTS, the RC2 lever
================================================================================
Capture-aware general slot-based locals in the bytecode VM (`tish_bytecode/compiler.rs`),
the long-missing half of RC2. A function's params + **uncaptured** body `let`/`const` now
resolve to frame slots (`LoadLocal`/`StoreLocal` — a direct `Vec` index) instead of
name-keyed `LoadVar`/`StoreVar` (hashmap lookup + `borrow()` mutex-lock-under-send-values +
Value clone + scope-chain walk). Locals **captured** by a nested closure (via its body OR its
param defaults) stay name-based in `local_scope` (which closures capture) — so closures still
work. Lifts the whole VM family (vm/cranelift/wasi embed the VM); the rust-AOT backend has its
own compiler and is unaffected.

  RESULT: a function-local hot loop (`fn work(n){let s=0;for(let i=0;i<n;i++){…};return s}`,
  200×100k iters) went **~8000ms → ~2900ms (2.7×)** on `tish run`. Pure numeric loops are still
  ~76× Node (V8 JITs them; slots remove name-resolution overhead but the boxed-`Value` arithmetic
  + interpreter dispatch remain → that's #14, the JIT — slots are necessary-not-sufficient).

  ELIGIBILITY (conservative, default-bails): flag on; simple params, no rest; no shadowing; no
  captured param; body uses only analysable constructs (bails on for-of, try, switch, destructuring
  decls, `||=`/`&&=`/`??=`, unknown variants). So the `main.tish` BUNDLE does NOT yet benefit (its
  `__perf_run_modules_*` use for-of/try/etc. → bail; the standalone micros are top-level, not
  function-scoped). Broadening eligibility + top-level slotting = increment 2.

  VALIDATED: flag-OFF byte-identical (suite 14/0); flag-ON full cross-backend suite 14/0 + bundle
  ≡ interp + `tests/core/slot_capture.tish` (closure/FunDecl param-default capture, forEach capture,
  shadow-across-closure, uncaptured hot loop). Flag-gated (no runtime fallback, unlike the JIT) until
  broader soak. Bug caught in validation: nested-closure PARAM DEFAULTS capture (`(a=secret)=>a`).

================================================================================
  FULL SUITE RUN (2026-06-05) — all backends, after rust-AOT async parity fix
================================================================================
Fresh `scripts/run_performance_suite.sh --release`. The headline change: the rust
backend no longer CRASHES on the bundle (was exit 134 on `.then()` chains), so its
sustained-compute number is now real — and much better than the prior unreliable
175ms. Suite failures are down to 2/52, **both QuickJS** (a reference runtime), not
any tish backend; every tish backend passes.

BUNDLED PERF SUITE — tests/main.tish, sustained compute, 5-run avg (ms):
  backend       ms     vs Node   note
  rust (AOT)   106     1.6x      best tish; beats QuickJS (255)
  vm (default) 300     4.5x      <- the real gap (#13/#14 lever territory)
  interp       311     4.6x
  cranelift    327     4.9x      mirrors vm (embeds it)
  llvm         326     4.9x      mirrors vm
  wasi        1268    18.9x      still broken for compute; needs own triage
  Node          67     1.0
  Bun           49     0.7x
  Deno          61     0.9x
  QuickJS      255     3.8x

PER-TEST compute (micro, vm vs Node%): object_stress 321% · benchmark_granular 294%
  · new_features_perf 208% · array_stress 142%.  Startup-bound micros (~46 of them):
  ~30% (tish ~9ms vs Node ~30ms) — tish WINS ~3x on small scripts (durable startup edge).

Reconciliation vs the prior log: array_stress micro is startup-amortized at the file
level; the BUNDLE (all tests in one process) is the honest sustained-compute figure.
rust 1.6x (not the earlier 2.3x) — the old number was measured while the bundle was
aborting mid-run. Default `tish run` (vm) at 4.5x and cranelift/llvm (which embed the
VM) are the remaining compute reds; wasi at 19x is a separate problem.

================================================================================
  GAUNTLET PASS (2026-06-05) — native struct fields + native numeric reduce
================================================================================
Two shared rust-backend codegen changes (no per-test hacks). The compute gauntlet
(`scripts/run_perf_gauntlet.sh`, compute-only / startup excluded, rust-AOT vs node V8)
went from **6/8 → 8/8 beating V8**. Both closed reds:

  benchmark    before        after        change
  object_sum   11ms (3.67x)  1ms (0.33x)   native struct-field arithmetic
  array_hof    109ms (3.86x) 11ms (0.38x)  native numeric reduce fold

  object_sum: `let o = {x,y}` is already struct-inferred, but `sum + o.x + o.y` boxed every
    operand through `tishlang_runtime::ops::add(&Value::Number(..))`. Added an `Expr::Member`
    arm to `emit_typed_expr` (codegen.rs) so a native struct field reads as raw f64 with its
    field type — the binop then stays native `((sum + o.x) + o.y)`. Helps ALL struct-field
    arithmetic, not just this micro.
  array_hof: `a.reduce((acc,x)=>acc+x, 0)` fused-folds, but each step was a boxed `ops::add`.
    `try_fused_reduce` now emits a native-f64 fold guarded by a runtime all-numeric check
    (init + every element is `Value::Number` → fold in raw f64; else fall back to the boxed
    fold from the original init). Correct: `+`'s string-concat path is preserved — verified
    `["a","b"].reduce(+,"")`="ab", `[1,"x"].reduce(+,0)`="1x", mul/sub/empty all match interp.

Still the lever for the *full* array_stress/object_stress (multi-section) + the VM family:
unboxed packed `Vec<f64>` arrays (#13) and more VM JIT coverage (#14).

VM JIT — bitwise + ternary slices (same day): `tish_vm/src/jit.rs`.
  • **bitwise** `BitAnd/BitOr/BitXor` + `~BitNot` via `fcvt_to_sint_sat` (= VM's saturating `f64 as i32`,
    bit-for-bit; verified incl. `3e9 & 1 = 1`, negatives, `2.9|0=2`). jit_probe §04 → ~15 ms.
  • **ternary** `cond ? A : B` → a Cranelift `select` (build_body refactored: `emit_simple_op` helper +
    a peephole over forward `JumpIfFalse`/`Jump` with branch-free, net-+1, agreeing-is_bool arms;
    `falsy_flag` reproduces JS truthiness incl. NaN/-0). Bails (→VM) on loops, early-return-in-branch,
    nested/general if-else, mismatched is_bool. jit_probe §02 map-ternary **269 ms → 17 ms** (~16×).
  Verified bit-for-bit vs interp + zero interp-vs-vm divergences across the suite. Benefits the whole
  VM family (vm/cranelift/llvm/wasi embed the JIT). Next: §05 Math 703 ms (cranelift `sqrt` + libcalls),
  §06 inline hot loop 752 ms (whole-loop JIT, the hardest), general if/else (blocks/SSA rewrite).

Note on **packed `Vec<f64>` arrays (#13)** — de-risked but *lower value than expected*: the rust-backend
codegen is already complete (an explicit `let a: number[]` lowers push/index/reduce/map/filter/spread/…
natively or auto-converts at boundaries), so only the inference is missing. BUT the rust backend already
*beats* Node on array-heavy tests (array_stress 26 ms vs 42), and gauntlet array_hof is already green — so
packing would only widen an already-winning margin. It does NOT help the VM-family array_stress (166 ms),
which runs the bytecode VM (boxed `Value` arrays) and needs the JIT instead. So the JIT is the real lever
for the remaining compute reds; packed-array inference is a rust-backend polish for later.

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
  array_hof        108ms   29ms   3.7x   FAIL  fused reduce LANDED (8.9x->3.7x); rest = packed f64 arrays
  object_sum        11ms    3ms   3.7x   FAIL  struct inference LANDED (48x->3.7x); rest = native struct arithmetic

6/8 beating V8. TWO big representation wins this pass:

FUSED REDUCE (TISH_FUSED_HOF): `arr.reduce((acc,x)=>acc OP x, init)` with a plain binop of the two
params lowers to a native fold using the SAME runtime Value op the closure body would
(`try_fused_reduce` in codegen.rs) — eliminating the per-element `value_call`. SOUND: identical Value
semantics, so string `reduce((a,x)=>a+x,"")` still concatenates; no-init + any non-`param OP param`
body fall back. array_hof 264ms->108ms (8.9x->3.7x). Rest = **packed f64 arrays** (boxed `Vec<Value>`
still pays a clone + `ops::add` match per element).

STRUCT INFERENCE now FIRES for object_sum (TISH_STRUCT_INFER): #12's struct lowering existed but
`infer_object_shape` couldn't type `{ x: i }` because the struct-infer ctx didn't record local var
types. Two-line fix in `infer.rs::si_block`: record each plain `let`'s type (from its annotation —
the first inference pass adds `: number` — OR inferred from its init) so a later object literal can
type its fields. `{ x: i, y: i+1 }` now infers a `#[derive] struct { x: f64, y: f64 }` (stack, native
field access) instead of a boxed `Rc<RefCell<PropMap>>`. object_sum 145ms->11ms (48x->3.7x); result
identical, `objects`/`object_methods` byte-identical struct-infer vs not. Soundness gate
(`uses_are_struct_safe`) UNCHANGED — only eligibility widened. Rest = the field arithmetic
`sum + o.x + o.y` still routes through boxed `ops::add` even though `o.x`/`o.y` are native f64 struct
fields → needs typed-member arithmetic de-boxing in `emit_typed_expr`.

So BOTH gauntlet reds are now ~3.7x (was 48x / 8.9x) and BOTH are one codegen step from green: native
struct-field arithmetic (object_sum) and packed f64 arrays (array_hof). Earlier flips: math_trig
(Math intrinsics), string_concat (`push_str`), recursion_fib (M5), recursion_untyped (M4+M5).
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
