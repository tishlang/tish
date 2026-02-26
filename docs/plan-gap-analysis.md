# Tish Plan Gap Analysis

Audit of plan vs implementation. Last updated: 2026-02-26.

## Implemented ✓

### Plan Section 7 (MVP features)
| Feature | Status |
|---------|--------|
| Numbers, strings, booleans, null | ✓ |
| `any x = expr` (block-scoped) | ✓ |
| `fun name(a, b) { }` and `fun name(a) = expr` | ✓ |
| if/else, while, for C-style | ✓ |
| `for (any x of arr)` | ✓ |
| Nested blocks and loops | ✓ |
| Arrays `[]`, indexing `a[i]` | ✓ |
| Plain objects `{}`, dot/index access | ✓ |
| `===` / `!==`, `&&` \|\| `!`, `??`, `?.` | ✓ |
| Optional braces (indentation) | ✓ |

### Plan Section 3.1 (ECMA checklist)
| Item | Decision | Status |
|------|----------|--------|
| block-scope | Follow | ✓ |
| comments | Follow | ✓ |
| computed-property-names | Follow | ✓ |
| addition, array, assignment, call | Follow | ✓ |
| coalesce (`??`) | Follow | ✓ |
| conditional (`? :`) | Follow | ✓ |
| division, multiplication, modulus, exponentiation | Follow | ✓ |
| bitwise | Follow | ✓ |
| logical-and/or/not | Follow | ✓ |
| member, optional-chaining | Follow | ✓ |
| object | Follow | ✓ |
| strict-equals | Follow | ✓ |
| increment/decrement (postfix) | Follow | ✓ |
| typeof | Follow | ✓ |
| void | Follow | ✓ |
| block, break, continue, for, if, return, while | Follow | ✓ |
| switch, do-while | Follow | ✓ |
| throw, try/catch | Follow | ✓ |
| Array (simplify), Math, String, Object | Follow | ✓ |
| parseInt, parseFloat, isFinite, isNaN | Follow | ✓ |
| Infinity, NaN | Follow | ✓ |
| Math.abs, sqrt, min, max, floor, ceil, round | Follow | ✓ |
| array.length, string.length | Follow | ✓ |

### Plan Section 7 (concrete MVP tests)
| Test | .tish | .js |
|------|-------|-----|
| Nested loops | nested_loops.tish | nested_loops.js |
| Variable scopes | scopes.tish | scopes.js |
| Optional braces | optional_braces.tish, optional_braces_braced.tish | ✓ |
| Tab vs space | tab_indent.tish, space_indent.tish | ✓ |
| fun and any | fun_any.tish | fun_any.js |
| Strict equality | strict_equality.tish | strict_equality.js |

**Total: 25 .tish / 25 .js tests (1:1 parity)**

---

## Missing features

### Plan "Follow" but not implemented

| Feature | Plan ref | Effort |
|---------|----------|--------|
| **Rest parameters** | 3.1.2 rest-parameters Follow | ✓ Implemented |
| **Static import/export** | 3.1.2, §4 "Simple modules" | Large — §7 says "no import in MVP"; deferred |
| **decodeURI/encodeURI** | 3.1.5 Omit or Follow | ✓ Implemented |
| **JSON** | 3.1.5 Optional | ✓ Implemented (JSON.parse, JSON.stringify) |

### Plan "Omit or Simplify" — optional

| Feature | Plan ref | Notes |
|---------|----------|-------|
| **in operator** | 3.1.3 in/instanceof | ✓ Implemented — `"x" in obj` |
| **instanceof** | 3.1.3 | Omit or Simplify |
| **delete** | 3.1.3 | Omit or Simplify |
| ** destructuring** | 3.1.2 | Simplify or defer |

### Builtins "Follow (simplify)" — partial

| Feature | Plan ref | Current | Gap |
|---------|----------|---------|-----|
| **Boolean** | 3.1.5 | bool literals | No Boolean(x) constructor |
| **String** | 3.1.5 | strings, .length | No String(x) constructor, no .slice/.substring |
| **Array** | 3.1.5 | arrays, .length, indexing | No Array(n), .push, .pop |
| **Error/NativeErrors** | 3.1.5 | throw/catch work | No Error constructor, no .message |

### Other missing (not in plan MVP)

| Feature | Notes |
|---------|-------|
| **Prefix ++/--** | ✓ Implemented |
| **Compound assignment** | `+=`, `-=`, `*=`, etc. — not listed in plan |
| **Logical assignment** | `&&=`, `\|\|=`, `??=` — not listed |

---

## Test coverage

- **Full-stack parse**: all 25 .tish ✓
- **Interpreter run**: all 25 .tish ✓
- **Interpreter vs native**: 24 files in subset (all pass except fun_any compile may vary)
- **Performance Tish vs JS**: 25 pairs in run_performance_manual.sh ✓

---

## Recommended next steps (by priority)

1. ✓ **Rest parameters** — Implemented
2. ✓ **JSON.parse / JSON.stringify** — Implemented
3. ✓ **decodeURI / encodeURI** — Implemented
4. ✓ **Prefix ++/--** — Implemented
5. ✓ **in operator** — Implemented
6. ✓ **console object** — Implemented (log, info, debug, warn, error with log levels)

---

## Recent Changes

### Console Object (2026-02-26)

Replaced `print()` with JavaScript-compatible `console` object:

| Method | Description | Output |
|--------|-------------|--------|
| `console.debug(...)` | Debug messages | stdout (hidden by default) |
| `console.info(...)` | Info messages | stdout (hidden by default) |
| `console.log(...)` | General output | stdout |
| `console.warn(...)` | Warnings | stderr |
| `console.error(...)` | Errors | stderr (always shown) |

**Log Level Configuration**: `TISH_LOG_LEVEL` environment variable
- Values: `debug`, `info`, `log` (default), `warn`, `error`
- Default shows: log, warn, error
- Debug shows: all messages

**Runtime Override**: The `console` object can be reassigned in code for custom logging.

---

## Architecture Notes

See `docs/architecture-next-steps.md` for planned refactoring to consolidate duplicated code between `tish_eval` and `tish_runtime`.
