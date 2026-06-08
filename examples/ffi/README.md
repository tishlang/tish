# tish FFI examples — native Rust extensions over the C ABI

Demonstrates tish's C-ABI FFI (Workstream B): native extensions compiled as **cdylibs**, loaded from
tish source via an `ffi:` import, and called like normal modules. Two extensions show the two ways to
write one — a **linked** extension (`mathext`) and a **decoupled** extension (`statext`).

## Run it

```sh
./examples/ffi/build.sh          # builds mathext.lib (linked) + statext.lib (decoupled)
tish run examples/ffi/demo.tish
```

Expected output:

```
=== tish FFI demo — native Rust extensions over the C ABI ===
hypot(3, 4)    = 5
factorial(6)   = 720
greet('tish')  = Hello, tish! (from native Rust)
summary([..])  = sum=200 mean=40 max=100 count=5
stats([..])    -> sum = 200  mean = 40  max = 100  count = 5
=== all native calls returned through the FFI ===
```

## What it shows

Each export is an `extern "C" fn(args: *const TishValueRef, argc) -> TishValueRef` that touches tish
values ONLY through the host's `tish_value_*` C-ABI accessors — it never names tish's Rust `Value`
type. `tish_module_register` returns the name→fn table the host loads with `tish_ffi::load_module`.
The CLI resolves an `ffi:<path>` import (relative to the importing file), loads the cdylib, and
registers its exports as a native module — so `import { hypot, … } from "ffi:./mathext.lib"` just
works on `tish run` (the VM backend).

Marshaling demonstrated end-to-end: **numbers** (`hypot`, `factorial`), **strings** (`greet`),
**arrays** in (`summary`, `stats`), and an **object** out (`stats` → `{sum, mean, max, count}`).

## Two ways to write an extension

- **Linked** — `mathext/` links `tishlang_ffi` for the accessors. Simplest to build, but the cdylib
  must match the host's `tish_core` **byte layout**: the *same dependency versions* (so it's a
  workspace member, built from the repo's `target/`) AND the *same value-affecting features* (so
  `features = ["send-values", "regex"]`, matching the shipped `tish`'s `full`). Numbers/strings/arrays
  are robust; object storage is an `IndexMap` whose layout is feature-sensitive, so a linked extension
  that returns an object must match exactly — `mathext` returns a string from `summary` to stay simple.
- **Decoupled** (the production model) — `statext/` is a standalone cdylib that links **nothing**
  tish-related. It *declares* the `tish_value_*` accessors `extern "C"` and resolves them against the
  host at `dlopen` (the host exports them via `-export_dynamic`/`-rdynamic`; the plugin opts into
  late binding with `-undefined dynamic_lookup` on macOS). Because every value is created and read
  through the host's accessors there is a **single `tish_core`** — no layout matching, any feature
  config / dep version works, **including returning objects**. `statext`'s `stats` returns an object,
  the case the linked model can't do safely. This is the model real extensions should use.
