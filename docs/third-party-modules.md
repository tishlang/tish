# Third-Party Native Modules for Tish

This document specifies the formal requirements for third-party native modules (e.g. `tish-polars`, `tish-egui`) that extend Tish with Rust-based capabilities.

## Overview

Third-party modules integrate with Tish via:

- **Interpreter:** `TishNativeModule` trait; globals registered at startup
- **Compiled:** Import-driven; `import { Egui } from 'tish:egui'` resolves the package and adds it as a Cargo dependency. No `tish_runtime` glue.
- **Package:** npm `package.json` with `tish.module`, `tish.crate`, `tish.export`; resolved from `node_modules` or workspace layout

## 1. Trait Contract

### TishNativeModule (Interpreter)

All native modules must implement `TishNativeModule` from `tish_eval`:

```rust
pub trait TishNativeModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn register(&self) -> HashMap<Arc<str>, Value>;
}
```

- `name()` — Module identifier (e.g. `"Polars"`).
- `register()` — Returns `HashMap<global_name, Value>` of globals to inject.
- Must be `Send + Sync` for thread safety.

### Opaque Types

For values that wrap Rust types (e.g. DataFrames):

- Implement `tish_core::TishOpaque` for method dispatch.
- Expose via `Value::Opaque(Arc::new(your_type))`.
- Methods are invoked via `get_method`; return `Value` or call `NativeFn` callbacks.

### Native Functions

- Use `Value::Native(fn_ptr)` or `EvalValue::Native(fn_ptr)` for callbacks.
- Native functions receive `Value` arguments and return `Value` or `Result<Value, String>`.

## 2. Version Compatibility

- **Minimum tish version:** Document in module's README or `peerDependencies`.
- **Rust edition:** `2021`.
- **Cargo resolver:** `"2"` when using workspace.
- **MSRV:** Match Tish's minimum supported Rust version.

## 3. Dependency Contract

### Required Crates (Interpreter Path)

- `tish_core` — Core types (`Value`, `TishOpaque`, `NativeFn`).
- `tish_eval` — `TishNativeModule`, `Evaluator`, `Value`.
- `tish_parser` — If parsing Tish source (optional).

### Feature Alignment

Third-party `tish_eval` features must match what the module uses:

- `http` — For `fetch`, timers, `serve`.
- `fs` — For `readFile`, `writeFile`, etc.
- `process` — For `process.env`, etc.
- `regex` — For `RegExp`, etc.

### Compiled Path

For compiled output support:

1. Implement an export function (e.g. `egui_object()`) that returns `tish_core::Value` (a module object built with `tish_core::tish_module!` or manually).
2. Add `package.json` with `tish.module: true`, `tish.crate`, `tish.export` (the Rust function name).
3. Tish resolves `tish:egui` etc. via package lookup, generates `Cargo.toml` with the module crate as a dependency, and emits calls like `tish_egui::egui_object()`.
4. No changes to `tish_runtime` or `tish_compile` — modules are fully external.

## 4. Package Layout (Standalone)

### Cargo Layout

- **Package name:** `tish-<domain>` (e.g. `tish-polars`).
- **Feature naming:** Match tish's feature name (e.g. `polars`).
- **Path assumption:** Standalone modules may use `path = "../tish/crates/..."`; document layout in README.

### npm Package Layout

```
tish-polars/
├── package.json       # name, version, tish.module, tish.feature, tish.crate
├── Cargo.toml
├── src/
│   └── lib.rs
```

**package.json (tish module):**

```json
{
  "name": "tish-polars",
  "version": "0.1.0",
  "tish": {
    "module": true,
    "crate": "tish-polars",
    "export": "polars_object"
  }
}
```

- `tish.module: true` — Identifies this package as a tish native module.
- `tish.crate` — Cargo crate name (used for the generated Cargo dependency).
- `tish.export` — Rust function name that returns the module `Value` (e.g. `polars_object`). Default: `{module}_object` (e.g. `egui` → `egui_object`).

## 5. Security and Stability

- **Transitive deps:** Avoid pulling heavy or unstable dependencies without justification.
- **Pinning:** Recommend caret or exact pins for tish crates in third-party `Cargo.toml`.
- **No dynamic loading:** Extensions are statically linked; no `.so` plugins at runtime.

## 6. Registry and Resolution

- **npm:** Tish native modules are published to npm as regular packages.
- **Convention:** `tish-*` prefix or `tish.module: true` identifies native modules.
- **Resolution:** Use `dependencies` in `package.json`; `npm install` populates `node_modules`. tish tooling maps `tish-*` deps to Cargo features.
