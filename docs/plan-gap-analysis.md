# Tish Plan Gap Analysis

Audit of plan vs implementation. Last updated: 2026-02-27.

## Breaking Change: `any` → `let`/`const`

As of 2026-02-26, the `any` keyword has been replaced with `let`/`const` to align with JavaScript:

```tish
// Old (deprecated)
any x = 5

// New
let x = 5       // mutable binding
const y = 10    // immutable binding (error on reassignment)
```

**Benefits:**
- Familiar to JS/TS developers
- Enables compiler optimizations for const bindings
- Better native code generation (`let` vs `let mut` in Rust)

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
| computed-property-names | Follow | ⚠️ Partial (access only, not in literals) |
| addition, array, assignment, call | Follow | ✓ |
| coalesce (`??`) | Follow | ✓ |
| conditional (`? :`) | Follow | ✓ |
| division, multiplication, modulus, exponentiation | Follow | ✓ |
| bitwise | Follow | ✓ |
| logical-and/or/not | Follow | ✓ |
| member, optional-chaining | Follow | ✓ |
| object | Follow | ✓ |
| strict-equals | Follow | ✓ |
| increment/decrement (postfix & prefix) | Follow | ✓ |
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
| **compound assignment** (`+=`, `-=`, `*=`, `/=`, `%=`) | Not in plan | ✓ Added |
| **type annotations** (optional, parsed not enforced) | Not in plan | ✓ Added |

### Plan Section 7 (concrete MVP tests)
| Test | .tish | .js |
|------|-------|-----|
| Nested loops | nested_loops.tish | nested_loops.js |
| Variable scopes | scopes.tish | scopes.js |
| Optional braces | optional_braces.tish, optional_braces_braced.tish | ✓ |
| Tab vs space | tab_indent.tish, space_indent.tish | ✓ |
| fun and any | fun_any.tish | fun_any.js |
| Strict equality | strict_equality.tish | strict_equality.js |
| **Objects (comprehensive)** | objects.tish, objects_perf.tish | objects.js, objects_perf.js |
| **Compound assignment** | compound_assign.tish | compound_assign.js |
| **Type annotations** | types.tish | types.js |
| **Mutation** | mutation.tish | mutation.js |
| **Array methods** | array_methods.tish | array_methods.js |
| **String methods** | string_methods.tish | string_methods.js |

**Total: 36 .tish / 38 .js tests**

---

## Missing features

### Plan "Follow" but not implemented

| Feature | Plan ref | Effort | Notes |
|---------|----------|--------|-------|
| **Rest parameters** | 3.1.2 rest-parameters Follow | ✓ Implemented | |
| **Static import/export** | 3.1.2, §4 "Simple modules" | Large | §7 says "no import in MVP"; deferred |
| **decodeURI/encodeURI** | 3.1.5 Omit or Follow | ✓ Implemented | |
| **JSON** | 3.1.5 Optional | ✓ Implemented | JSON.parse, JSON.stringify |

### Critical gaps (not in plan but essential for JS compatibility)

| Feature | Current | Gap | Effort |
|---------|---------|-----|--------|
| **Property assignment** | ✓ Implemented | `obj.x = val` and `arr[i] = val` now work | — |
| **Mutable arrays/objects** | ✓ Implemented | `Rc<RefCell<...>>` enables mutation | — |
| **Computed property names** | Dynamic access only | `{ [expr]: val }` in literals not supported | Small |

### Plan "Omit or Simplify" — optional

| Feature | Plan ref | Notes |
|---------|----------|-------|
| **in operator** | 3.1.3 in/instanceof | ✓ Implemented — `"x" in obj` |
| **instanceof** | 3.1.3 | Omit (no classes) |
| **delete** | 3.1.3 | Omit or Simplify |
| **destructuring** | 3.1.2 | Simplify or defer |

### Builtins "Follow (simplify)" — partial

| Feature | Plan ref | Current | Gap |
|---------|----------|---------|-----|
| **Boolean** | 3.1.5 | bool literals | No Boolean(x) constructor |
| **String** | 3.1.5 | strings, .length, methods | ✓ `.indexOf()`, `.includes()`, `.slice()`, `.substring()`, `.split()`, `.trim()`, `.toUpperCase()`, `.toLowerCase()`, `.startsWith()`, `.endsWith()`, `.replace()`, `.replaceAll()`, `.charAt()`, `.charCodeAt()`, `.repeat()`, `.padStart()`, `.padEnd()` |
| **Array** | 3.1.5 | arrays, .length, mutation, methods | ✓ `.push()`, `.pop()`, `.shift()`, `.unshift()`, `.indexOf()`, `.includes()`, `.join()`, `.reverse()`, `.slice()`, `.concat()` |
| **Error/NativeErrors** | 3.1.5 | throw/catch work | No Error constructor, no .message |

### Other features (not in plan MVP)

| Feature | Status | Notes |
|---------|--------|-------|
| **Prefix ++/--** | ✓ Implemented | |
| **Compound assignment** | ✓ Implemented | `+=`, `-=`, `*=`, `/=`, `%=` |
| **Logical assignment** | Not implemented | `&&=`, `\|\|=`, `??=` |
| **Spread operator** | Not implemented | `...arr` |

---

## Semantic differences from JavaScript

| Behavior | JavaScript | Tish | Rationale |
|----------|-----------|------|-----------|
| **No undefined** | `undefined` type exists | `null` only | Simplification |
| **Optional chaining on null** | Returns `undefined` | Returns `null` | Follows from above |
| **Loose equality** | `==` with coercion | Not supported (error) | By design |
| **Type coercion** | Implicit in many ops | No implicit coercion | By design |

---

## Test coverage

- **Full-stack parse**: all 31 .tish ✓
- **Interpreter run**: all 31 .tish ✓
- **Interpreter vs native**: Most files pass (some differences in compiled output)
- **Performance Tish vs JS**: 31 pairs in run_performance_manual.sh ✓

---

## Recommended next steps (by priority)

### Completed
1. ✓ **Rest parameters** — Implemented
2. ✓ **JSON.parse / JSON.stringify** — Implemented
3. ✓ **decodeURI / encodeURI** — Implemented
4. ✓ **Prefix ++/--** — Implemented
5. ✓ **in operator** — Implemented
6. ✓ **console object** — Implemented (log, info, debug, warn, error with log levels)
7. ✓ **Compound assignment** — Implemented (`+=`, `-=`, `*=`, `/=`, `%=`)

### High priority remaining
8. ✓ **Property/index assignment** (`obj.x = val`, `arr[i] = val`) — Implemented
9. ✓ **Mutable arrays** — `Rc<RefCell<Vec>>` implemented
10. ✓ **String methods** — 16 methods implemented
11. ✓ **Array methods** — 10 methods implemented
12. **Computed property names** in object literals

### Lower priority
13. **Object methods** (`Object.keys()`, `Object.values()`)
14. **Higher-order array methods** (`.map()`, `.filter()`, `.reduce()`, `.find()`, `.some()`, `.every()`)
15. **Error constructor**
16. **Logical assignment** (`&&=`, `||=`, `??=`)

---

## Recent Changes

### Array and String Methods (2026-02-27)

Added comprehensive built-in methods for arrays and strings:

**Array methods:**
| Method | Description |
|--------|-------------|
| `.push(val, ...)` | Add elements to end, returns new length |
| `.pop()` | Remove and return last element |
| `.shift()` | Remove and return first element |
| `.unshift(val, ...)` | Add elements to beginning, returns new length |
| `.indexOf(val)` | Find index of element, or -1 |
| `.includes(val)` | Check if element exists |
| `.join(sep)` | Join elements with separator |
| `.reverse()` | Reverse array in place |
| `.slice(start, end)` | Extract portion (non-mutating) |
| `.concat(arr, ...)` | Combine arrays |

**String methods:**
| Method | Description |
|--------|-------------|
| `.indexOf(str)` | Find index of substring, or -1 |
| `.includes(str)` | Check if substring exists |
| `.slice(start, end)` | Extract portion |
| `.substring(start, end)` | Extract portion (like slice) |
| `.split(sep)` | Split into array |
| `.trim()` | Remove whitespace |
| `.toUpperCase()` | Convert to uppercase |
| `.toLowerCase()` | Convert to lowercase |
| `.startsWith(str)` | Check prefix |
| `.endsWith(str)` | Check suffix |
| `.replace(search, rep)` | Replace first occurrence |
| `.replaceAll(search, rep)` | Replace all occurrences |
| `.charAt(idx)` | Get character at index |
| `.charCodeAt(idx)` | Get char code at index |
| `.repeat(n)` | Repeat string n times |
| `.padStart(len, pad)` | Pad at start |
| `.padEnd(len, pad)` | Pad at end |

All methods work in both interpreter and compiled modes.

### Mutable Objects and Arrays (2026-02-27)

Property and index assignment now work:

```tish
// Object property assignment
let obj = { x: 1 }
obj.x = 10
obj.newProp = "hello"

// Array index assignment  
let arr = [1, 2, 3]
arr[1] = 20
arr[5] = 100  // extends array with nulls

// Object bracket notation
let data = {}
data["key"] = "value"
```

**Implementation details:**
- Changed `Rc<Vec>` to `Rc<RefCell<Vec>>` for arrays
- Changed `Rc<HashMap>` to `Rc<RefCell<HashMap>>` for objects
- Added `Expr::MemberAssign` and `Expr::IndexAssign` AST nodes
- Array index assignment auto-extends array (fills gaps with null)

### Type Annotations (2026-02-27) — Phase 2

Added optional TypeScript-style type annotations. Types are parsed and stored in the AST but not enforced during evaluation (gradual typing).

```tish
let x: number = 42
const name: string = "hello"
let nums: number[] = [1, 2, 3]

fun add(a: number, b: number): number {
    return a + b
}

// Object types
let person: { name: string, age: number } = { name: "Alice", age: 30 }

// Union types
let value: number | string = 42
```

**Supported types:**
- Primitives: `number`, `string`, `boolean`, `null`, `void`
- Arrays: `T[]`
- Objects: `{ key: Type, ... }`
- Unions: `T | U`
- Function types (syntax only, not yet used)

**Next phases:**
- Phase 3: Type inference engine
- Phase 4: Type checking with errors

### Compound Assignment (2026-02-26)

Added compound assignment operators with full JS parity:

| Operator | Example | Behavior |
|----------|---------|----------|
| `+=` | `x += 5` | `x = x + 5` |
| `-=` | `x -= 3` | `x = x - 3` |
| `*=` | `x *= 2` | `x = x * 2` |
| `/=` | `x /= 4` | `x = x / 4` |
| `%=` | `x %= 3` | `x = x % 3` |

Works with:
- Number arithmetic
- String concatenation (`s += " World"`)
- Chained assignment (`p += q -= 2`)

### Comprehensive Object Tests (2026-02-26)

Added `objects.tish` and `objects_perf.tish` with:
- Nested objects (deep property chains)
- Dynamic property access
- Objects with mixed value types
- Objects as function parameters/returns
- Optional chaining on objects
- `in` operator performance
- Reference equality testing

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

See `docs/architecture-next-steps.md` for the completed shared core refactor (Phases 1-5 complete).

### Current Crate Structure

```
crates/
├── tish_core/       # Shared Value type, ops, JSON, URI (standalone)
├── tish_lexer/      # Lexer with indent normalization (standalone)
├── tish_ast/        # AST types (standalone)
├── tish_parser/     # Parser (depends on: tish_lexer, tish_ast)
├── tish_eval/       # Tree-walk interpreter (depends on: tish_ast, tish_parser, tish_core)
├── tish_runtime/    # Runtime for compiled code (depends on: tish_core)
├── tish_compile/    # Compiler AST→Rust (depends on: tish_ast, tish_runtime)
└── tish/            # CLI (depends on: all above)
```
