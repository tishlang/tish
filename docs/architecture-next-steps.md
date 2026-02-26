# Tish Architecture: Shared Core Refactor

Technical document detailing the shared core refactor and type system consolidation.

**Status: Phase 1-3 COMPLETE** (as of Feb 2026)

## Implementation Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1: Create tish_core | ✅ Complete | Created unified Value, ops, json, uri modules |
| Phase 2: Migrate tish_runtime | ✅ Complete | Now re-exports from tish_core |
| Phase 3: Migrate tish_eval | ✅ Complete | Uses tish_core for URI encoding |
| Phase 4: Update tish_compile | ✅ Complete | Uses tish_runtime (which uses tish_core) |
| Phase 5: Cleanup | ✅ Complete | tish_hello_build deleted |

## Current Crate Structure

```
crates/
├── tish_core/       # NEW: Shared Value type, ops, JSON, URI (standalone)
├── tish_lexer/      # Lexer with indent normalization (standalone)
├── tish_ast/        # AST types (standalone)
├── tish_parser/     # Parser (depends on: tish_lexer, tish_ast)
├── tish_eval/       # Tree-walk interpreter (depends on: tish_ast, tish_parser, tish_core)
├── tish_runtime/    # Runtime for compiled code (depends on: tish_core)
├── tish_compile/    # Compiler AST→Rust (depends on: tish_ast, tish_runtime)
└── tish/            # CLI (depends on: all above)
```

## Historical Analysis (Pre-Refactor)

### Identified Problems

#### 1. Duplicated Value Types

Two separate `Value` enums with different representations:

| Location | Representation |
|----------|----------------|
| `tish_eval/src/value.rs` | Enum with native function variants (`NativeConsoleLog`, etc.) |
| `tish_runtime/src/lib.rs` | Enum with `Function(NativeFn)` closure |

**Impact**: Same logic implemented twice, potential semantic drift.

#### 2. Duplicated Operation Logic

Binary operations (`+`, `-`, `*`, `/`, etc.) are implemented twice:

- `tish_eval/src/eval.rs` → `eval_binop()` method
- `tish_compile/src/codegen.rs` → `emit_binop()` generates inline Rust

**Lines duplicated**: ~100 lines of logic per location.

#### 3. Duplicated Utility Functions

| Function | tish_eval | tish_runtime |
|----------|-----------|--------------|
| `is_truthy()` | ✓ | ✓ |
| `strict_eq()` | ✓ | ✓ |
| `to_string()` / `to_display_string()` | ✓ | ✓ |
| `json_parse()` / `json_stringify()` | ✓ (~200 lines) | ✓ (~200 lines) |
| `percent_encode()` / `percent_decode()` | ✓ (~50 lines) | ✓ (~50 lines) |
| Math functions | ✓ (inline) | ✓ |
| parseInt/parseFloat | ✓ (inline) | ✓ |

**Total duplicated code**: ~500+ lines.

#### 4. Orphaned Code

- `tish_hello_build/` - Orphaned test directory referencing non-existent `print` function
- `TokenKind::Eof` - Defined but never emitted

---

## Proposed Solution: Shared Core

### New Crate: `tish_core`

Create a shared crate containing:

1. **Unified Value type** - Single source of truth
2. **Operation implementations** - `add()`, `sub()`, `mul()`, etc.
3. **Type utilities** - `is_truthy()`, `strict_eq()`, `type_name()`
4. **JSON serialization** - `json_parse()`, `json_stringify()`
5. **URI encoding** - `percent_encode()`, `percent_decode()`
6. **Native function implementations** - parseInt, parseFloat, Math.*, etc.

### Revised Dependency Graph

```
Before:
tish_eval ──────────────────────────────────┐
                                            │ (duplicated logic)
tish_runtime ───────────────────────────────┘

After:
                    ┌─────────────┐
                    │ tish_core   │  ← NEW: shared types & operations
                    └──────┬──────┘
                           │
            ┌──────────────┴──────────────┐
            │                             │
     ┌──────▼──────┐              ┌───────▼───────┐
     │ tish_eval   │              │ tish_runtime  │
     │ (interpret) │              │ (compiled)    │
     └─────────────┘              └───────────────┘
```

### Value Enum Design

```rust
// crates/tish_core/src/value.rs

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Runtime value for Tish programs.
/// Used by both interpreter and compiled code.
#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<Vec<Value>>),
    Object(Rc<HashMap<Arc<str>, Value>>),
    /// Native function callable at runtime.
    /// Signature: fn(args) -> Result<Value, String>
    NativeFunction(NativeFn),
}

/// Native function type - works for both interpreter and compiled.
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>;
```

### Operation Design

```rust
// crates/tish_core/src/ops.rs

use crate::value::Value;

/// Binary addition with strict type checking (no implicit coercion).
pub fn add(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
        (Value::String(a), Value::String(b)) => {
            Ok(Value::String(format!("{}{}", a, b).into()))
        }
        _ => Err(format!(
            "TypeError: cannot add {} and {}",
            left.type_name(),
            right.type_name()
        ))
    }
}

// ... sub, mul, div, mod, pow, bitwise ops, comparisons
```

### How Interpreter Uses It

```rust
// crates/tish_eval/src/eval.rs

fn eval_binop(&self, l: &Value, op: BinOp, r: &Value) -> Result<Value, EvalError> {
    let result = match op {
        BinOp::Add => tish_core::ops::add(l, r),
        BinOp::Sub => tish_core::ops::sub(l, r),
        BinOp::Mul => tish_core::ops::mul(l, r),
        // ...
    };
    result.map_err(EvalError::Error)
}
```

### How Compiler Uses It

```rust
// crates/tish_compile/src/codegen.rs

fn emit_binop(&self, l: &str, op: BinOp, r: &str) -> String {
    match op {
        BinOp::Add => format!("tish_core::ops::add(&{}, &{})?", l, r),
        BinOp::Sub => format!("tish_core::ops::sub(&{}, &{})?", l, r),
        // ...
    }
}
```

---

## Implementation Plan

### Phase 1: Create tish_core (Foundation)

**Files to create:**
- `crates/tish_core/Cargo.toml`
- `crates/tish_core/src/lib.rs`
- `crates/tish_core/src/value.rs` - Unified Value enum
- `crates/tish_core/src/ops.rs` - Binary/unary operations
- `crates/tish_core/src/types.rs` - type_name(), is_truthy(), strict_eq()

**Estimated changes**: ~300 new lines (extracted from existing code)

### Phase 2: Migrate tish_runtime

**Changes:**
- Remove duplicated `Value` enum from `tish_runtime/src/lib.rs`
- Re-export `tish_core::Value`
- Remove duplicated `is_truthy()`, `strict_eq()`, `to_display_string()`
- Keep console functions (log level logic is runtime-specific)

**Lines removed**: ~200

### Phase 3: Migrate tish_eval

**Changes:**
- Remove `Value` enum from `tish_eval/src/value.rs`
- Replace inline native function handling with `NativeFunction` variant
- Remove duplicated JSON parsing (~200 lines)
- Remove duplicated URI encoding (~50 lines)
- Call `tish_core::ops::*` in `eval_binop()`

**Lines removed**: ~400

### Phase 4: Update tish_compile

**Changes:**
- Update codegen to emit `tish_core::ops::*` calls
- Simplify generated code (no more inline match statements)

**Lines changed**: ~100

### Phase 5: Cleanup

**Changes:**
- Delete `tish_hello_build/` directory
- Remove `TokenKind::Eof` if unused
- Update all documentation

---

## Design Decisions

### 1. No Implicit Type Coercion

Unlike JavaScript, Tish will NOT coerce types implicitly:

```javascript
// ❌ JavaScript behavior (NOT supported)
1 + "2"     // "12"
"5" - 2     // 3
[] + {}     // "[object Object]"

// ✅ Tish behavior
1 + "2"     // TypeError: cannot add number and string
1 + 2       // 3
"a" + "b"   // "ab"
```

**Rationale**: Predictable behavior, same semantics in interpreter and compiled.

### 2. Single NativeFunction Variant

Instead of many `NativeConsole*`, `NativeMath*` variants, use one:

```rust
// Before (current tish_eval)
enum Value {
    // ...
    NativeConsoleLog,
    NativeConsoleWarn,
    NativeMathAbs,
    NativeMathSqrt,
    // ... 15+ variants
}

// After (tish_core)
enum Value {
    // ...
    NativeFunction(NativeFn),  // Single variant for all natives
}
```

**Rationale**: Simpler enum, easier to add new functions, same representation for interpreter and compiled.

### 3. Result-Based Operations

All operations return `Result<Value, String>`:

```rust
pub fn add(l: &Value, r: &Value) -> Result<Value, String>
pub fn sub(l: &Value, r: &Value) -> Result<Value, String>
```

**Rationale**: Explicit error handling, no panics, same behavior everywhere.

---

## Files Affected Summary

| Crate | Files Changed | Lines Added | Lines Removed |
|-------|---------------|-------------|---------------|
| tish_core (NEW) | 4 new files | ~300 | 0 |
| tish_runtime | 1 file | ~20 | ~200 |
| tish_eval | 2 files | ~30 | ~400 |
| tish_compile | 1 file | ~20 | ~100 |
| docs | 3 files | ~50 | ~20 |

**Net change**: ~-300 lines (removal of duplication)

---

## Testing Strategy

1. **Existing tests remain unchanged** - All 29 MVP `.tish` files should pass
2. **Add unit tests for tish_core** - Test each operation in isolation
3. **Interpreter vs Native parity** - Existing integration test validates identical output

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Behavioral regression | Run full test suite after each phase |
| Performance impact | Benchmark before/after (existing `run_performance_manual.sh`) |
| Circular dependencies | tish_core has no dependencies on other tish crates |

---

## Out of Scope

The following are NOT part of this refactor:

- Type inference at compile time
- Unboxed value representations
- Static typing / type annotations
- New language features

These may be considered in future work after the shared core is stable.
