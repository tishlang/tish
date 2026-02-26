# Tish vs ECMA-262 Alignment

This document maps Tish behavior to ECMA-262 and test262. Each concept has a decision: **Follow**, **Omit**, or **Simplify**.

## Spec Clauses (§)

| Clause | Decision | Notes |
|--------|----------|-------|
| §5 Notational | Follow | |
| §6 Types | Follow | Undefined→Null, Boolean, Number, String, Object. No Symbol/BigInt in MVP. |
| §7 Type conversion | Simplify | ToBoolean, ToNumber, ToString only as needed. No loose equality. |
| §7.2 Testing | Follow | Strict Equality only |
| §7.3 Operations on objects | Simplify | GetV, HasProperty, Call, CreateDataProperty. No Construct/private in MVP. |
| §8 Executable code | Follow | Lexical envs, execution context. Omit Realms, RunJobs. |
| §9 Ordinary/exotic objects | Simplify | Ordinary only; no Proxy |
| §10 Source code | Follow | + indent/tab normalization |
| §11 Lexical grammar | Follow | + `fun`/`any`, optional braces |
| §12–15 Expressions, Statements | Follow subset | No `this`, `with`, `class` |
| §16 Errors | Follow | throw, try/catch |
| §17 Literals | Follow | number, string, boolean, null, [], {}. Omit template/BigInt. |
| §18 Global | Follow | Single global |
| §19 Fundamental | Follow | Object, Function, Error. Symbol Omit. |
| §20–21 Numbers, Math, String | Follow subset | BigInt, Date, RegExp Omit or optional |
| §22–24 Array | Follow (simplify) | TypedArray, Map, Set, JSON Omit or optional |
| §25 Control abstraction | Simplify | Iteration follow; Generator, Promise, Async Omit in MVP |
| §26 Reflection | Omit | Proxy, Reflect |
| Annex B, D | Omit | Legacy escapes, `__proto__`, etc. |

## test262/language

- **block-scope** — Follow
- **comments** — Follow
- **computed-property-names** — Follow (MemberProp::Expr)
- **keywords** — + `fun`, `any`
- **source-text** — + tab/space normalization
- **arguments-object** — Omit
- **eval-code** — Omit
- **directive-prologue** — Omit (strict-only)
- **destructuring** — Simplify or defer

## test262/language/expressions

- **addition** — Follow (numeric; string concat for strings)
- **array** — Follow
- **arrow-function** — as `fun`
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
- **typeof** — Follow
- **async, await, generators, yield** — Omit
- **class, new, super, this** — Omit
- **delete, in, instanceof** — Omit or Simplify
- **dynamic-import, import.meta** — Omit
- **tagged-template, template-literal** — Omit

## test262/language/statements

- **block** — Follow
- **break, continue** — Follow
- **const/let** — as `any`
- **for** — Follow (C-style)
- **if, return, while** — Follow
- **variable** — as `any`
- **function** — as `fun`
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
- **Promise, Proxy, Reflect, Symbol** — Omit
- **RegExp** — Omit or optional

## Where Tish Differs from JavaScript

1. **`fun` / `any`** — Replaces `function` / `let`.
2. **Optional braces** — Indentation can define blocks; no Python-style mixing errors.
3. **Tab/space agnostic** — Both normalized; no “war.”
4. **No `this`** — Use explicit parameters.
5. **No prototypes** — Plain records and arrays.
6. **Strict equality only** — No `==` or implicit coercion.
7. **No `eval`, `with`** — Omitted for security and compileability.
8. **No `var`** — Block-scoped `any` only.
