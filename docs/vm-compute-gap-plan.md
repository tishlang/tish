# VM compute gap — diagnosis + plan (2026-06-05)

Default `tish run` (the bytecode VM) is **4.5× Node** on sustained compute (bundle: vm 300ms vs
Node 67ms; cranelift/llvm mirror it at ~327ms since they embed the VM; wasi 19×). This is the
headline perf gap. Below is an **evidence-grounded** root-cause ranking (measured this run, not
assumed) and a sequenced, de-risked plan. It supersedes the original plan's RC1-centric view.

## Measured root causes (ranked by leverage)

### 1. Name-based variable resolution — THE dominant, broadest cost (the real RC2, mostly undone)
Every variable access in **all non-trivial code** runs `Opcode::LoadVar`/`StoreVar`
([vm.rs:1106](crates/tish_vm/src/vm.rs)):
```
local_scope.borrow()            // a Mutex LOCK under send-values (default build)
  .get(name.as_ref()).cloned()  // string-keyed hashmap lookup + Value clone
  .or_else(|| walk self.enclosing …)   // on miss: walk the whole captured scope chain
  .or_else(|| self.scope …).or_else(|| self.globals.borrow() …)  // …then globals
```
Slot infra EXISTS and is fast (`slot_locals: Vec<Value>`, `LoadLocal`/`StoreLocal` =
[vm.rs:988](crates/tish_vm/src/vm.rs) — a direct `Vec` index, no hashmap, no borrow, no walk).
But the compiler only uses it for `simple_fn_slots`
([compiler.rs:188](crates/tish_bytecode/src/compiler.rs)), which **bails to name-based the moment a
function "captures outer scope, declares locals, mutates, or defines nested functions."** That is
essentially every real function — including the entire `main.tish` bundle (each `__perf_run_modules_*`
declares `let`s and uses `.map`/`.filter` callbacks). **Top-level code is always name-based too.**
So a hot loop `for (let i…) { s = s + i*2 }` pays ~4–5 string-hashmap ops + borrows + clones PER
iteration where a real bytecode VM pays a few `Vec` indexes. This is the biggest lever and it lifts
the whole VM family (vm/cranelift/llvm/wasi).

### 2. `send-values` `Arc<Mutex>` tax — ~15% (smaller than the original plan assumed)
Measured: object_stress lean(no-send-values) ~84ms vs full ~100ms (~16%); array_stress ~44 vs ~48
(~9%). `ScopeMap = VmRef<ObjectMap>` ([vm.rs:798](crates/tish_vm/src/vm.rs)) → under the shipped
`full`→`http` build, every scope borrow is a mutex lock; container Values likewise. **Load-bearing:**
a `Value::Function` closure captures `enclosing: Vec<ScopeMap>`, and the HTTP/WS server dispatches
handler closures across worker threads, so the captured scopes must be `Send` → `Arc<Mutex>`.
Removing it requires the Phase-1 HTTP per-worker-VM isolation (so closures never cross threads). Real
but modest, and risky — do it AFTER slots.

### 3. Object representation — the object_stress lever (RC3/#13, still pending)
Even lean, object_stress is ~2.5× Node. Objects are `ObjectMap` (hashmap) keyed by `Arc<str>`; numeric
keys stringify per access. Node uses hidden classes (shape + slot). #13 (runtime hidden classes) is the
fix for object-heavy code. Independent of slots; do after slots.

### 4. Hot-loop / Math JIT — #14 (additive, the only path past the interpreter floor on pure loops)
`tish_vm/src/jit.rs` JITs numeric leaf functions (bitwise/ternary/arith landed). Bails on loops
(`JumpBack`) and Math calls. §06 hot-loop JIT (752ms in jit_probe) is the biggest single synthetic sink
but "the hardest" (top-level name-based loop → needs slots first, ironically). §05 Math is bounded
(cranelift libcalls + a soundness gate for reassignable `Math`).

## Sequenced plan (each step measurable + parity-gated)

**Step A — general slot-based locals (the lever). Biggest win, do first.**
Port the rust backend's capture analysis (`collect_vars_needing_capture_cell`,
[codegen.rs](crates/tish_compile/src/codegen.rs)) to the bytecode compiler. For each function:
1. Scope-aware slot allocation: params + every block-scoped `let`/`const` (incl. `for` headers) →
   a unique `u16` slot, respecting shadowing (same name in sibling blocks = distinct slots).
2. A local that is **captured by a nested closure** stays name-based (lives in the shared scope map);
   all **uncaptured** locals become slots (`LoadLocal`/`StoreLocal`). This sidesteps the full upvalue
   model and is exactly the rust backend's hybrid — proven correct there.
3. VM: size `slot_locals` to the function's slot count; bind params into slots 0..n (the call path
   already supports slot frames for `simple_fn_slots`).
4. **De-risk:** land behind incremental gates — first "functions with NO nested closures → all locals
   slotted" (zero capture complexity), full-suite + bundle parity, measure; then add the
   captured/uncaptured split. Watch the scope-assignment divergence class (the memory's recurring
   interp↔vm hazard) — diff `tests/main.tish` across all backends after each increment.

**Step B — #14 §05 Math JIT** (bounded, additive) then **§06 hot-loop JIT** (now tractable once loop
counters are slots from Step A).

**Step C — #13 hidden-class objects** (object_stress) — independent; large.

**Step D — `send-values` removal** via HTTP per-worker-VM isolation (Phase 1) — ~15%, highest risk,
last.

## Honest scope
Fully closing 4.5× is a multi-step compiler project (Steps A–D compound; none alone wins). Step A is
the highest-leverage, most-broadly-beneficial start and unblocks §06. The micro "wins" elsewhere in
the suite are **startup-bound** (tish ~9ms vs Node ~30ms) and must not regress — keep cold start ≤ ~28ms.

## Step A — implementation map + the critical finding (recon done 2026-06-05)

**Touch points (all in `crates/tish_bytecode/src/compiler.rs` unless noted):**
- `slot_ctx: Option<HashMap<Arc<str>,u16>>` (field 90) — drives reference resolution at the ident-load
  site (1367). It is the SINGLE SOURCE OF TRUTH: a name in the map → `LoadLocal`/`StoreLocal`; absent →
  `LoadVar`/`StoreVar`. **Partial conversion is automatically consistent** (slot-based VM frames also
  carry a `local_scope` hashmap — [vm.rs:931](crates/tish_vm/src/vm.rs) — so slotted + name-based locals
  coexist), PROVIDED every write site agrees with the map.
- Write sites that must become slot-aware (route through one `emit_var_store(name)` helper): Assign
  (1655), CompoundAssign/LogicalAssign (1772+), Postfix/Prefix Inc/Dec (1732–1780), VarDecl init (733),
  for-init `let` (`compile_for_init_statement` 348), and skip `LoopVarsBegin/End` (770/798/822/861/901/
  942/1072/1100) when the loop var is slotted (no closures → no per-iteration capture needed).
- VM side needs NO change: `slot_locals = vec![Null; num_slots]` ([vm.rs:936](crates/tish_vm/src/vm.rs))
  already sizes + zero-inits the frame; set `inner.slot_based=true; inner.num_slots=<count>` at the two
  function-compile sites (1023, 1667).
- **Dark-ship behind `TISH_VM_SLOTS`** (the project's standard pattern): eligibility returns `None` when
  the flag is off ⇒ byte-identical corpus. Eligibility walker must **default-bail** on any unrecognized
  `Statement`/`Expr` variant (a missed variant could otherwise hide a closure ⇒ capture miscompile).
  Closures = `Expr::ArrowFunction`, `Statement::FunDecl`/`DeclareFun`.

**CRITICAL FINDING — the safe first increment doesn't pay off; go straight to capture-aware:**
- The standalone micros (object_stress, array_stress) are **top-level scripts**, not functions — so
  *function*-level slotting doesn't touch them (top-level is its own name-based sink; slotting it is
  capture-sensitive too because later `function`s capture top-level vars as globals).
- The bundle's hot functions (`__perf_run_modules_*`) **use `.map`/`.filter`/`.reduce` callbacks** =
  nested closures, so the conservative "no nested closure → slot everything" gate **excludes them →
  ~0 bundle benefit.**
- ∴ Real benefit needs the **capture-aware** split: a local is slotted iff it is NOT in the
  over-approximated captured set (= every identifier appearing inside any nested `ArrowFunction`/
  `FunDecl` in the body); captured locals stay name-based. **VM path VERIFIED OK (no VM change needed):** `Constant::Closure`
  ([vm.rs:1036](crates/tish_vm/src/vm.rs)) pushes `local_scope.clone()` into the new closure's
  `enclosing` chain **unconditionally — regardless of `slot_based`**. So captured locals (name-based,
  living in `local_scope`) are seen by closures; uncaptured locals (in `slot_locals`, NOT in
  `local_scope`) are invisible to closures — exactly correct, since by definition nothing captures
  them. The per-iteration-`let` overlay path (1041–1057) likewise reads `local_scope`, so a captured
  loop var (name-based) still works; an uncaptured one is slotted and unreferenced. No new VM code.

**Sequencing (fresh focused session — recon makes it mechanical):** (1) capture-aware slot map behind
`TISH_VM_SLOTS` (eligibility walker default-bails on unknown variants; captured-set = idents inside any
nested `ArrowFunction`/`FunDecl`; slot the uncaptured params+lets, route every write through one
`emit_var_store` helper, skip `LoopVarsBegin` for slotted loop vars); (2) flag-off byte-identical check;
(3) flag-on validation (full suite + `tests/main.tish` bundle diff across ALL backends + closure/
recursion fixtures); (4) measure bundle, flip default-on only if parity holds; (5) extend to top-level.
