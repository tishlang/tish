# Type System: Status & Roadmap to Native, Dynamic-Free AOT

> **Status assessment + implementation roadmap** for tish's static type system.
> Last updated: 2026-06-04. Line numbers are snapshots from that date and may drift.
> Companion to the **"Roadmap: checked types (Phase 2)"** section of [`LANGUAGE.md`](LANGUAGE.md).

## Goal

tish source should use TypeScript-style type annotations that **lower to native Rust/machine
types**, so AOT compilation can do compile-time optimization and emit code with **no
dynamic/boxed values** — hardening compiled apps and improving performance. This doc records
how far that effort has come and the sequenced plan to finish it.

**Direction:** pursue **all three workstreams, sequenced** (coverage → soundness →
machine-AOT), targeting a **full TS-like** surface (generics, unions, interfaces, narrowing).

---

## Part 1 — Status assessment ("how far along")

### Scorecard

| Capability | State | Where |
|---|---|---|
| Type-annotation **syntax** (lex/AST/parse) | ✅ ~70% of a pragmatic TS subset | `tish_lexer`, `tish_ast/src/ast.rs:11`, `tish_parser/src/parser.rs:484` |
| Internal **type representation** | ✅ Solid for scalars/arrays/structs; ❌ no generics/real-unions | `tish_compile/src/types.rs:11` (`RustType`) |
| Type **inference** | ⚠️ ~15% — forward, literal + numeric-arithmetic only | `tish_compile/src/infer.rs` |
| Type **checking / soundness** | ❌ 0% — nothing validates annotations | (no checker crate exists) |
| **Type-driven native codegen** (Rust backend) | ✅ Real but trapped *inside* one function body | `tish_compile/src/codegen.rs:1721,5009,5127` |
| **Cross-function** native typing | ⚠️ params: native SCALAR params (`number`/`boolean`/`string`) now get a native shadow, dark-shipped behind `TISH_PARAM_NATIVE` (matmul `fn bench(N: number)` 301→15ms, 3x node, corpus correct); returns still boxed | `codegen.rs` param-bind + `push_fun_param_scope` |
| **Machine-code AOT** (Cranelift/LLVM) | ❌ ~5% — stubs that embed bytecode + run the VM | `tish_cranelift/src/lower.rs:3` |
| Optimizations exploiting types | ⚠️ const-fold/DCE/algebraic only (not type-driven) | `tish_opt/src/lib.rs` |

**Net: ~25–35% of the way** to "TS-like types that make most code fully-static native." The
foundation exists and demonstrably accelerates annotated numeric/struct code (see
`examples/matmul`), but it covers only a thin slice of real programs.

### What parses today (frontend)

Supported: primitives (`number/string/boolean/void/null/any`), `T[]`, `{k:T}`, `A | B` unions,
`(T) => R` function types, `type X = …`, `declare let/const/function`.

**Missing:** generics `<T>`, optional `T?`, intersection `A & B`, literal types, `as` casts,
`interface`, tuples. (`parser.rs:484` `parse_type_annotation`; AST enum `ast.rs:11`
`TypeAnnotation`.)

### How types lower today (`RustType`, `types.rs:11`)

`number→f64`, `string→String`, `boolean→bool`, `void/null→()`, `T[]→Vec<T>`,
`T|null→Option<T>`, `{..}`→emitted `TishStruct_*`, `(T)=>R→Rc<dyn Fn>`. **Everything else →
`RustType::Value`** (the dynamic tagged enum, `tish_core/src/value.rs:284`). Codegen gates on
`RustType::is_native()` to pick native vs boxed emission; `f64`/`bool` arithmetic, `Vec<f64>`,
and struct field access all emit natively. `result_type_of_binop` (`types.rs:142`) only handles
`f64×f64` and `bool×bool` — **not even string `+`**.

### Three architecture facts that dominate the roadmap

1. **The real "native" path is the Rust backend.** `tish build --native-backend rust` emits
   Rust source calling `tishlang_runtime`, then `cargo build` → rustc produces the machine code.
   The **`cranelift` and `llvm` backends do not consume type info at all** — they serialize
   bytecode into an object file and run `tishlang_vm` (VM-class throughput).
   `tish_cranelift/src/lower.rs:3`: *"This is **not** AOT compilation of Tish into Cranelift
   IR."* So typed→native already works via rustc; a *direct* typed-IR→machine-code path does not
   exist.

2. **User functions are boxed closures with a fixed ABI.** Every `fn f(...)` is emitted as a
   `Value::native(move |args: &[Value]| -> Value { … })` closure living inside one giant `run()`
   function; calls go through `tishlang_runtime::value_call(&callee, &[args])` with a fixed
   `(&Value, &[Value]) -> Value` signature. Params are read as `Value` from `args.get(i)`;
   `Return` always yields a `Value`. So parameters *genuinely arrive boxed* — typing them is an
   ABI change, not a one-line fix.

3. **No checker; mismatches become runtime panics.** Annotations are never validated. A `Value`
   reaching a typed slot is unwrapped by `from_value_expr` (`types.rs:227`) which emits
   `match … _ => panic!("expected number")`. That panic is the only "enforcement," and it fires
   at runtime, not compile time.

### The keystone blocker

Because of fact #2 and the deliberate `push_fun_param_scope` → `RustType::Value` (`types.rs:384`,
guarded by the `push_fun_param_scope_shadows_outer` test at `types.rs:481`), **static types
can't cross a function call, parameter, return, array element, or member access** — they revert
to `Value`. This is why only ~5–15% of typical code currently goes native. **Unlocking
cross-function typing is the single highest-leverage change** and is Phase 1's centerpiece.

---

## Part 2 — Roadmap (sequenced)

```
Phase 1  Coverage      maximize native lowering on the Rust backend (kill the boxing)
Phase 2  Soundness     a real type checker: Ty IR + unification + diagnostics (hardening)
Phase 3  Full TS + AOT generics/mono, union enums + narrowing, interfaces, true machine-AOT
```

Each phase is built so every milestone is **additive and independently verifiable**: with new
behavior gated/dark-shipped, the entire `tests/core/` + `examples/` corpus must stay
byte-identical until a milestone is deliberately switched on. Test harness:
`crates/tish/tests/integration_test.rs` (stdout vs `*.tish.expected`, `REGENERATE_EXPECTED=1`
to refresh); north-star fixtures `tests/core/types.tish` and `examples/matmul/src/main.tish`.
Cross-runtime parity via `just parity`.

### Phase 1 — Coverage: lower the bulk of code to native (Rust backend)

Strategy **A (boundary-coercion, low risk, ships first):** keep the `Value::native` closure ABI;
bind a *native shadow* for typed params at the top of the closure body, compute sub-expressions
natively, and pay one `from_value_expr`/`to_value_expr` coercion at each Value↔native boundary
(args in, result out). Strategy **B (native monomorphic `fn`, stretch):** for fully-typed
non-escaping functions, also emit a parallel free `fn f_native(a:f64,…)->R` and route direct
calls to it, bypassing `value_call` entirely.

| # | Milestone | Core change | Files |
|---|---|---|---|
| **M0** | Function **signature table** (no-op pre-pass) | `FnSig{params,rest,returns,native_safe}` + `FnSigTable`, built after `collect_type_aliases`; unused at first so it can't regress | `types.rs` (new), `codegen.rs:~1348` |
| **M2** | **String** concat + native string methods | add `String+String→String` (+ comparisons) to `result_type_of_binop` (`types.rs:142`) and mirror in `infer.rs`; whitelist UTF-safe `str` methods | `types.rs:142`, `infer.rs:80`, `codegen.rs:~3060/5170` |
| **M3** | **Collections**: arrays-of-structs + nested element typing | much may be free (`Vec<Named>` composes); generalize the typed Member fast path to any `object` whose type is `Named` (not just `Ident`) so `xs[i].field`/`a.b.c` lower native | `codegen.rs:3124`, `infer.rs` (Array arm) |
| **M1** | **Cross-function param + return typing** (keystone) | type annotated params via `from_value_expr` native shadow; thread return type; add a `Call` arm to `emit_typed_expr` that reports the signature's `returns` (wrapping `value_call` result with `from_value_expr`) | `types.rs:384`, `codegen.rs:2303,1906,5127,5258` |
| **M4** | **Inference upgrade** (bidirectional, dark-shipped) | extend `infer.rs` to propagate through call returns (via table), member access on structs, array elements, string concat, loop/closure vars; **param inference** from use-sites ∩ call-sites behind `TISH_PARAM_INFER` (mirrors `TISH_STRUCT_INFER`) | `infer.rs:71,123,656` |
| **M5** | **Native monomorphic `fn`** (stretch, Strategy B) | emit parallel `fn f_native` for eligible (`native_safe`) functions; route direct calls; keep closure wrapper as safety net | `codegen.rs:2069,5127` |

**Ship order:** **M0 → M2 → M3 → M1 → M4 → M5** (build confidence on low-risk surface before
touching the param ABI). Highest-risk audit point before M1: `emit_expr`'s `Expr::Ident` arm
(`codegen.rs:2593`) must auto-box a native-typed ident used in a `Value` position via
`to_value_expr`.

**Reuse, don't reinvent:** `from_value_expr`/`to_value_expr` (`types.rs:227/278`) for every
boundary; `is_native`/`result_type_of_binop`/`from_annotation_with_aliases`; the
`InferCtx`/`TypeContext` scope machinery; the escape-safety predicate pattern
`uses_are_struct_safe` (`infer.rs:389`) as the template for "bail to Value on any uncertainty";
the existing native-VarDecl capture path (`refcell_wrapped_vars`/`rc_cell_storage`,
`codegen.rs:1727`) for closures capturing typed vars.

**Phase 1 verify:** with flags off, `tests/core` byte-identical at every milestone; inspect
generated Rust for `examples/matmul` — `bench`'s `i < N` and `i*N+k` should be `f64` ops, not
`ops::lt`/`ops::mul`; benchmark the ms drop; `just parity` to catch semantic drift.

### Phase 2 — Soundness: a real type checker

**Architectural decision: introduce a dedicated `Ty` IR — do *not* overload `RustType`.** Keep
`TypeAnnotation` (syntax) and `RustType` (Rust emission) as the two ends; insert `Ty` as the
semantic middle (`crates/tish_compile/src/ty.rs`). Rationale: `RustType` is *lossy by design*
(`A|B`→`Value`, inline objects→`Value`) and that lossiness is load-bearing for codegen, so a
checker that reasons in `RustType` literally cannot tell `string|number` from `any` and can't
narrow. `Ty` adds inference variables (`Ty::Var`), `Unknown`/`Never`, literal types, tuples,
generic params, and distinct unions. `Ty::lower(env) -> RustType` is where Phase 3 later
enriches lowering (real union enums, monomorphized generics). Invariant kept green forever:
`lower(from_annotation(x)) == legacy RustType::from_annotation_with_aliases(x)`.

| # | Milestone | Core change |
|---|---|---|
| **M2.0** | `Ty` IR + lowering parity (dormant) | `ty.rs`: `Ty`, `from_annotation`, `Ty::lower`; property-test parity vs current `RustType` mapping |
| **M2.1** | Checker core, **warn-only** | `check.rs`: scoped `TyEnv`, bidirectional `synth`/`check`, `unify` + directional `is_assignable` (structural width-subtyping for objects), `check_program -> Vec<TypeDiagnostic>`; wire into codegen (warn) + LSP `publish_parse_and_lint` |
| **M2.2** | `--checked` gating + **panic-path hardening** | flag plumbed through `compile_*`; in checked mode, error-severity diags abort with `CompileError{span}`; the `panic!` coercions become statically unreachable for checked programs (kept only as dynamic-boundary backstop) |
| **M2.3** | Structural object/alias compatibility + `: Foo` navigation | full width-subtyping, alias-resolution fixpoint (reuse `collect_type_aliases`), LSP go-to-def on type refs (extend `tish_resolve::definition_span`) |

**Pipeline placement:** insert `check_program` between `infer::infer_program` and `emit_program`
(`codegen.rs:671`). One shared `check_source(text)` powers both `tish build` and `tish-lsp` so
CLI and editor never drift.

**Gradual typing:** `any ≡ Ty::Any` unifies with everything and suppresses errors (lowers to
`Value`); unannotated bindings/params stay permissive by default, with `--checked`/`--strict`
escalating implicit-any and assignability failures to hard errors. The `Value↔native` seam
(`JSON.parse`, FFI, untyped imports) is the first-class dynamic boundary: legal only behind
`any` or a narrowing guard.

**Error channel:** `CompileError{message, span}` (`codegen.rs:281`) already feeds the CLI and is
re-parsed by LSP via `parse_error_pos` (`tish_lsp/src/main.rs:54`); `TypeDiagnostic` is a
`{message,span,severity}` superset.

**Phase 2 verify:** golden checker tests in `crates/tish_compile/tests/` (good→no diags;
`const x: number = "s"`→one diag at exact span); zero-false-positive gate over `examples/` +
`tests/` before any default-on/`--checked`-blocking change; regression that a program which used
to compile to a `panic!("expected number")` site now fails at compile time under `--checked`.

### Phase 3 — Full TS-like surface + true machine-code AOT

Each feature = parser + `Ty` + lowering + checker rules + **`tree-sitter-tish/grammar.js`**
update (LANGUAGE.md:202 requires keeping the editor grammar in sync). Every unsupported case
falls back to `Value` — never a miscompile.

| # | Milestone | Core change |
|---|---|---|
| **M3.1** | Generics parse + represent | `<T>` in `parse_type_annotation` (disambiguate `<`/`>` by type-position only); `TypeAnnotation::Generic`, `Ty::Param/Named{args}`; checker infers type args via the `Ty::Var` unification engine |
| **M3.2** | **Monomorphization** | new `mono.rs` (after check, before lower): collect concrete instantiations, clone-and-specialize per arg-set; `Array<number>`→`Vec<f64>` (no `Value`); dedup identical instantiations by canonical key |
| **M3.3** | Optional `T?`, literal types, tuples, `as` | parser + `Ty` + native lowering; add `RustType::Tuple(Vec<RustType>)`→Rust tuples; `T?`→`Option<T>`; literals power discriminants |
| **M3.4** | **Discriminated unions + narrowing** | stop collapsing non-null unions to `Value`: lower `A|B` to a generated Rust `enum`; flow-sensitive narrowing in checker (`typeof`/discriminant/`!== null`) so narrowed branches lower native (reuse typed-member fast path); conservative bail to `Value` on escape |
| **M3.5** | Interfaces | `interface` keyword + `Statement::Interface`; structural match reuses M2.3 `is_assignable`; lowers to the same `TishStruct_*` path as object aliases; `extends`=field-set intersection |
| **M3.6** | *(optional)* typed-IR Cranelift AOT | only if removing rustc-in-the-loop is required; generalize `tish_vm/src/jit.rs` (already lowers f64 slot-based code via `JITModule`) to the monomorphized typed IR, behind `--native-backend cranelift-aot`; Rust backend stays the default + correctness oracle |

**True-AOT recommendation.** Make the **typed Rust backend the canonical machine-AOT path.** Once
generics monomorphize (M3.2), unions become enums (M3.4), and tuples/structs lower natively
(M3.3/M3.5), the emitted Rust contains `f64`/`Vec<f64>`/structs/enums and **no `Value`** except
at explicit `any`/FFI boundaries — and rustc optimizes it to hand-written-Rust quality. The
Cranelift/LLVM stubs cannot reach this without re-implementing every lowering as CLIF; pursue
M3.6 only if the rustc/cargo dependency and build latency are unacceptable, and even then as a
"hot typed core in CLIF, rest via runtime" hybrid, not a wholesale replacement.

---

## Cross-cutting risks

1. **Native ident in a `Value` position** (`codegen.rs:2593`) — must auto-box; audit before M1.
2. **Boundary coercions** — always go through `from_value_expr`/`to_value_expr`; never hand-roll.
3. **Mutation/escape reverting a typed var to `Value`** — extend the existing conservative-bail
   predicates (`infer.rs:389`); don't bypass them.
4. **Closures capturing typed vars** — reuse the native-VarDecl `RefCell` capture path or revert
   to `Value`.
5. **Monomorphization code-size blowup** — dedup instantiations, cap depth, fall back to
   `Value`-erased generics beyond a threshold.
6. **Structural-vs-nominal mismatch** — checker is structural but codegen emits nominal
   `TishStruct_*`; on cross-struct structural assignment, emit a field-wise conversion (reuse the
   object-literal→struct path) or intern identical shapes.
7. **`<`/`>` parser ambiguity** — only parse generic args in type position; add targeted tests
   (`a < b > c` vs `Array<Map<string,number>>`).
8. **Three type representations drifting** (`TypeAnnotation`/`Ty`/`RustType`) — single conversion
   fns + the permanent parity property test.
9. **tree-sitter grammar drift** — "update grammar.js + queries" is a checklist item in every
   surface-changing milestone; CI parse-corpus gate.
10. **Dark-ship discipline** — aggressive inference/new lowering gated by env flag or `--checked`;
    full corpus byte-identical with flags off at every step.

## Verification strategy (overall)

- **Differential output:** generated Rust / runtime behavior unchanged for supported programs at
  every milestone (additive-only, the discipline `infer.rs` and `jit.rs` already follow).
- **No-`Value` assertions:** grep generated Rust for `Value` on typed-core fixtures to prove
  boxing is eliminated where types are known (Phase 3).
- **CLI+LSP parity:** same source → same diagnostics from `tish build --checked` and
  `check_source`.
- **Cross-runtime parity:** `just parity` (interpreter / VM / native / Node) on each milestone.
- **Benchmarks:** `examples/matmul` ms before/after M1+M4 as the headline coverage metric.

## Critical files

- `crates/tish_compile/src/types.rs` — `RustType`, `push_fun_param_scope:384`,
  `result_type_of_binop:142`, `from/to_value_expr:227/278`; add `FnSigTable`, `RustType::Tuple`.
- `crates/tish_compile/src/codegen.rs` — FunDecl/closure emit (`2069`), param binding (`2303`),
  `Return` (`1906`), `emit_typed_expr` (`5127`), `emit_native_expr` (`5009`), Member fast path
  (`3124`), `value_call` (`3104`), pipeline hook (`671`), struct/enum emit (`864`), Ident
  auto-box (`2593`).
- `crates/tish_compile/src/infer.rs` — `infer_expr_type:71`, `infer_program:123`, FunDecl
  inference (`656`), escape-safety template (`389`).
- **New:** `crates/tish_compile/src/ty.rs` (Ty IR), `check.rs` (checker), `mono.rs`
  (monomorphization), `crates/tish_compile/tests/check_*.rs`.
- `crates/tish_ast/src/ast.rs:11` (`TypeAnnotation` + new variants/statements),
  `crates/tish_parser/src/parser.rs:484` (surface syntax), `crates/tish_lexer/src/token.rs`
  (`as`/`interface` keywords).
- `crates/tish_lsp/src/main.rs:124` (type diagnostics + nav), `crates/tish/src/main.rs`
  (`--checked` flag), `tree-sitter-tish/grammar.js`, `docs/LANGUAGE.md:200`.
- *(optional AOT)* `crates/tish_vm/src/jit.rs` (reuse), `crates/tish_cranelift/src/` (new `aot.rs`).
