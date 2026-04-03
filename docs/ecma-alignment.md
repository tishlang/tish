# Tish vs ECMA-262 Alignment

This document maps Tish behavior to ECMA-262 and test262. Each concept has a decision: **Follow**, **Omit**, or **Simplify**.

## Spec Clauses (§)

| Clause | Decision | Notes |
|--------|----------|-------|
| §5 Notational | Follow | |
| §6 Types | Follow | Undefined→Null; `typeof null` is `"null"`. Boolean, Number, String, Object. No Symbol/BigInt in MVP. |
| §7 Type conversion | Simplify | ToBoolean, ToNumber, ToString only as needed. No loose equality. |
| §7.2 Testing | Follow | Strict Equality only |
| §7.3 Operations on objects | Simplify | GetV, HasProperty, Call, CreateDataProperty. `new` is parsed and lowered to host `construct` (not full ECMA `[[Construct]]` on VM / interpreter / native Rust; JS compile target emits native `new`). |
| §8 Executable code | Follow | Lexical envs, execution context. Omit Realms, RunJobs. |
| §9 Ordinary/exotic objects | Simplify | Ordinary only; no Proxy |
| §10 Source code | Follow | + indent/tab normalization |
| §11 Lexical grammar | Follow | + `fn`/`function`, optional braces |
| §12–15 Expressions, Statements | Follow subset | No `this`, `with`, `class`. `new` expression: Simplify (see §7.3). |
| §16 Errors | Follow | throw, try/catch |
| §17 Literals | Follow | number, string, boolean, null, [], {}. Omit template/BigInt. |
| §18 Global | Follow | Single global |
| §19 Fundamental | Follow | Object, Function, Error. Symbol Omit. |
| §20–21 Numbers, Math, String | Follow subset | BigInt, Date, RegExp Omit or optional |
| §22–24 Array | Follow (simplify) | TypedArray, Map, Set, JSON Omit or optional |
| §25 Control abstraction | Simplify | Iteration follow; async/await Follow (simplify); Generator Omit; Promise Follow (ECMA-262 §27.2) |
| §26 Reflection | Omit | Proxy, Reflect |
| Annex B, D | Omit | Legacy escapes, `__proto__`, etc. |

## test262/language

- **block-scope** — Follow
- **comments** — Follow
- **computed-property-names** — Follow (MemberProp::Expr)
- **keywords** — + `fn`/`function`, `let`/`const`
- **source-text** — + tab/space normalization
- **arguments-object** — Omit
- **eval-code** — Omit
- **directive-prologue** — Omit (strict-only)
- **destructuring** — Simplify or defer

## test262/language/expressions

- **addition** — Follow (numeric; string concat for strings)
- **array** — Follow
- **arrow-function** — as `fn`
- **assignment** — Follow (`x = expr`)
- **call** — Follow
- **coalesce** — Follow (`??`)
- **conditional** — Follow (`a ? b : c`)
- **division, multiplication, modulus, exponentiation** — Follow (`**` right-associative)
- **bitwise** — Follow (`&` `|` `^` `~` `<<` `>>`, 32-bit integer semantics)
- **logical-and, logical-or, logical-not** — Follow
- **member, optional-chaining** — Follow (`?.`)
- **object** — Follow (plain `{}`)
- **strict-equals** — Follow (`===`, `!==`)
- **increment/decrement** — Follow (postfix `++`, `--` on identifiers)
- **typeof** — Follow; `typeof null` returns `"null"` (not `"object"`) since Tish has no undefined
- **async, await** — Follow (simplify); `await` works on Promises (`fetch`, `fetchAll`, user `Promise`, etc.)
- **generators, yield** — Omit
- **class, super, this** — Omit
- **new** — Simplify (host `construct` on VM / interpreter / native Rust; native `new` on JS emit); see [LANGUAGE.md](LANGUAGE.md) “Semantics” and the site doc [Tish vs JavaScript](https://tishlang.com/docs/language/vs-javascript)
- **delete, in, instanceof** — Omit or Simplify
- **static import / export** — Simplify (builtins, `tish:*`, multi-file resolver); not arbitrary npm on all targets
- **dynamic-import, import.meta** — Omit
- **tagged-template, template-literal** — Omit

## test262/language/statements

- **block** — Follow
- **break, continue** — Follow
- **const/let** — as `any`
- **for** — Follow (C-style)
- **if, return, while** — Follow
- **variable** — as `any`
- **function** — as `fn` or `function`
- **class, debugger, with** — Omit
- **throw, try** — Follow
- **switch, do-while** — Follow
- **for-in, for-of** — for-of Follow (arrays and strings); for-in Omit

## test262/built-ins

- **Array, Boolean, Number, Math, String** — Follow (simplify)
- **Object** — Follow (plain)
- **console** — Follow (log, info, debug, warn, error with TISH_LOG_LEVEL)
- **JSON** — Follow (parse, stringify)
- **global, Infinity, NaN** — Follow
- **Error, NativeErrors** — Follow or Simplify
- **parseInt, parseFloat, isFinite, isNaN** — Follow
- **decodeURI, encodeURI** — Follow
- **ArrayBuffer, BigInt, Date, Map, Set** — Omit or optional
- **Promise** — Follow (§27.2): `Promise(executor)`, `.then`, `.catch`, `.finally`, `Promise.resolve`, `Promise.reject`, `Promise.all`, `Promise.race`. Host APIs: `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`.
- **Proxy, Reflect, Symbol** — Omit
- **RegExp** — Omit or optional

## Where Tish Differs from JavaScript

1. **`fn` / `function`** — Both supported for function declarations.
2. **Optional braces** — Indentation can define blocks; no Python-style mixing errors.
3. **Tab/space agnostic** — Both normalized; no “war.”
4. **No `this`** — Use explicit parameters.
5. **No prototypes** — Plain records and arrays.
6. **Strict equality only** — No `==` or implicit coercion.
7. **No `eval`, `with`** — Omitted for security and compileability.
8. **No `var`** — Block-scoped `any` only.
9. **`new` expressions** — Supported syntactically; semantics are host-dependent and not full ECMA `[[Construct]]` except on JavaScript compile output.

## Bytecode VM: jump peephole (implementation)

Default `tish run` uses the bytecode VM (`tishlang_vm`), not the tree-walking interpreter. Post-compile **peephole jump chaining** (`crates/tish_bytecode/src/peephole.rs`) may rewrite `Jump` / `JumpIfFalse` offsets to skip chains of jumps.

**Constraint (matches ECMA control flow, not a language deviation):** chaining must follow only **unconditional** `Jump` instructions after the branch target. Treating **`JumpIfFalse` as part of that chain** was incorrect: the falsy target of `||` short-circuit can land immediately before an outer `if` condition’s `JumpIfFalse`, and “following through” that opcode rewrote jumps to the wrong bytecode offset. A second failure mode: other peepholes replace redundant instruction pairs with **`Nop` padding** (same length, no global offset fixup). Jump chaining must **skip leading `Nop` bytes** when resolving a branch target; otherwise one jump can be shortened to a different landing than another jump that still targets the start of the `Nop` run, breaking `||` inside `if (...)`. Symptom: expressions like `a === 1 || b === 2` inside `if (...)` evaluated correctly under `--backend interp` or `--no-optimize`, but wrongly under the default VM with peephole enabled. **Fixed** in `skip_leading_nops` plus `skip_unconditional_jump_chain` / `final_jump_target` (`crates/tish_bytecode/src/peephole.rs`). Regressions: `crates/tish_vm/tests/peephole_jump_chain_logical_or.rs`, `crates/tish/tests/run_optimize_stdout_parity.rs`.

**Native Rust backend (`tish build --native-backend rust`):** emits Rust via `tishlang_compile`; logical `||` / `&&` use Rust’s short-circuiting operators on the generated expressions and are not affected by bytecode peephole. **`cranelift` / `llvm`** backends embed the same bytecode chunk and `tishlang_vm` as `tish run`, so they follow this peephole behavior.

This was **not** a lexer/parser bug; the parser and AST were fine.
