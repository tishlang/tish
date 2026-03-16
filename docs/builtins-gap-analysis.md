# Tish Builtins Gap Analysis

Comprehensive overview of builtins across the Rust implementation vs. the bytecode VM (Cranelift/WASI).

## Why Tests Were Passing Despite Gaps

Integration tests in `crates/tish/tests/integration_test.rs` run each backend (interpreter, native, Cranelift, WASI, JS) and compare stdout to static expected files (`*.tish.expected`). Historically only interpreter vs. native was exercised; Cranelift and WASI parity tests were added so all backends are checked against the same expected output.

## Explicit Tests

### Cranelift Integration Test

- **Name:** `test_mvp_programs_cranelift`
- **Location:** `crates/tish/tests/integration_test.rs`
- **What it runs:** For each file in the curated list: run interpreter (`tish run <file> --backend interp`), compile with `tish compile <file> -o <temp> --native-backend cranelift`, run the binary, assert stdout equality.
- **Test files (curated):** `fn_any`, `strict_equality`, `switch`, `do_while`, `typeof`, `try_catch`, `json`, `math`, `builtins`, `uri`, `inc_dec`, `exponentiation`, `void`, `rest_params`, `arrow_functions`, `array_methods`, `types`. Many other files cause stack-underflow or scope bugs in the Cranelift backend and are excluded until fixed.
- **Maintenance:** When adding new pure-Tish tests that work with Cranelift, add them to the list in the test.

### WASI Integration Test

- **Name:** `test_mvp_programs_wasi`
- **Location:** `crates/tish/tests/integration_test.rs`
- **What it runs:** For each file in the curated list: run interpreter, compile with `tish compile <file> -o <temp> --target wasi`, run with `wasmtime <temp>.wasm`, assert stdout equality.
- **Test files:** Same as Cranelift (WASI uses bytecode VM; same programs work).
- **Skip behavior:** The test **skips** if `wasmtime` is not available (`wasmtime --version` check). If the `wasm32-wasip1` target is not installed, compile fails and that file is skipped with an informative message. CI without wasmtime still passes.

## Architecture

| Component | Uses | Target |
|-----------|------|--------|
| **tish_runtime** | tish_core, tish_builtins | Rust-compiled output (native binary) |
| **tish_vm** | tish_core, tish_builtins | Cranelift, WASI (bytecode) |
| **tish_eval** | Own natives, tish_core | Interpreter |

Shared implementations live in:
- **tish_core**: `Value`, `json_parse`, `json_stringify`, `percent_encode`, `percent_decode`
- **tish_builtins**: array, string, math, object (partial), globals (object_keys, encode_uri, etc.)

## Builtins in tish_runtime (codegen imports)

| Builtin | Source | In tish_vm? |
|---------|--------|-------------|
| console.{debug,info,log,warn,error} | tish_runtime | log, info, warn, error ✓ (no debug) |
| Boolean | tish_runtime::boolean | ✗ |
| decodeURI | tish_runtime::decode_uri → tish_core | ✗ |
| encodeURI | tish_runtime::encode_uri → tish_core | ✗ |
| isFinite | tish_runtime::is_finite | ✗ |
| isNaN | tish_runtime::is_nan | ✗ |
| JSON.parse, JSON.stringify | tish_core | ✓ |
| parseInt, parseFloat | tish_runtime | ✓ |
| Math.abs, sqrt, floor, ceil, round, min, max | tish_builtins::math | ✓ (inline impl, not shared) |
| Math.pow, sin, cos, tan, log, exp, sign, trunc | tish_builtins::math | ✗ |
| Math.random | tish_builtins::math | ✓ |
| Date.now | tish_runtime::date_now | ✓ |
| Array.isArray | tish_runtime::array_is_array | ✗ |
| String.fromCharCode | tish_runtime::string_from_char_code | ✗ |
| Object.assign | tish_runtime | ✓ |
| Object.fromEntries | tish_runtime | ✓ |
| Object.keys | tish_runtime::object_keys | ✗ |
| Object.values | tish_runtime::object_values | ✗ |
| Object.entries | tish_runtime::object_entries | ✗ |
| in operator | tish_runtime::in_operator | (handled in VM op) |

## Missing in tish_vm (causes uri, object_methods, etc. to fail)

1. **encodeURI**, **decodeURI** – use `tish_core::percent_encode` / `percent_decode`
2. **Boolean** – constructor
3. **isFinite**, **isNaN**
4. **Math**: pow, sin, cos, tan, log, exp, sign, trunc
5. **Array.isArray**
6. **String.fromCharCode**
7. **Object.keys**, **Object.values**, **Object.entries**
8. **console.debug** (minor)

## Shared Implementation Strategy

**Goal:** One implementation per builtin; tish_vm and tish_runtime both use it without tish_vm depending on tish_runtime.

### Target Dependency Layout

- **tish_core:** Value, json, uri (percent_encode/percent_decode). No dependency on tish_runtime or tish_vm.
- **tish_builtins:** array, string, math, object, and global-style builtins with signature `(args: &[Value]) -> Value`: object_keys, object_values, object_entries, object_assign, object_from_entries; decode_uri, encode_uri (wrap tish_core); boolean, is_finite, is_nan; array_is_array; string_from_char_code. Depends only on tish_core (and rand for math::random).
- **tish_runtime:** Depends on tish_core + tish_builtins. Re-exports or thin-wraps the above for codegen; keeps console_*, parse_int, parse_float, date_now, number_to_fixed, and any HTTP/fs/process-specific APIs. No dependency on tish_vm.
- **tish_vm:** Depends on tish_core + tish_builtins **only**. No tish_runtime dependency. In `init_globals()`, calls tish_builtins (and tish_core) for encodeURI, decodeURI, Boolean, isFinite, isNaN, Object.keys/values/entries, Array.isArray, String.fromCharCode, and tish_builtins::math for Math.

**Rule:** VM and compiled runtime both consume tish_builtins (and tish_core); tish_vm does not depend on tish_runtime.

### What Stays Where

| Crate | Contents |
|-------|----------|
| **tish_core** | Value, json_parse, json_stringify, percent_encode, percent_decode |
| **tish_builtins** | All pure builtin logic: array/string/math/object methods + global helpers (object_keys, encode_uri, boolean, is_finite, etc.) |
| **tish_runtime** | Compiler-facing API, console (I/O), parse_int/parse_float, Date.now, number_to_fixed, optional HTTP/fs/process; delegates to tish_builtins/tish_core |
| **tish_vm** | Bytecode execution and globals; only depends on tish_builtins + tish_core |

## Dependency Policy

- **Allowed:** tish_vm → tish_core, tish_builtins; tish_runtime → tish_core, tish_builtins; tish_eval → tish_core (and optionally tish_builtins later).
- **Avoid:** tish_vm → tish_runtime; tish_core or tish_builtins → tish_runtime or tish_vm.

## How-to Checklist

1. Add Cranelift integration test and test list ✓
2. Add WASI integration test (with skip when wasmtime unavailable) ✓
3. Move global builtin implementations from tish_runtime into tish_builtins ✓
4. Point tish_runtime at tish_builtins for those builtins ✓
5. Remove tish_runtime from tish_vm and wire tish_vm to tish_builtins ✓
6. Update this doc with any changes as implementation progresses ✓
