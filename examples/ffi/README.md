# tish FFI example — a native Rust extension over the C ABI

Demonstrates tish's C-ABI FFI (Workstream B): a native extension compiled as a **cdylib**, loaded
from tish source via an `ffi:` import, and called like a normal module.

## Run it

```sh
./examples/ffi/build.sh          # builds mathext -> examples/ffi/mathext.lib
tish run examples/ffi/demo.tish
```

Expected output:

```
=== tish FFI demo — native Rust extension over the C ABI ===
hypot(3, 4)    = 5
factorial(6)   = 720
greet('tish')  = Hello, tish! (from native Rust)
summary([..])  = sum=200 mean=40 max=100 count=5
=== all native calls returned through the FFI ===
```

## What it shows

`mathext/src/lib.rs` is a Rust cdylib whose exports use ONLY the host's `tish_value_*` C-ABI
accessors — they never name tish's Rust `Value` type. Each export is an
`extern "C" fn(args: *const TishValueRef, argc) -> TishValueRef`, and `tish_module_register`
returns the name→fn table the host loads with `tish_ffi::load_module`. The CLI resolves an `ffi:<path>`
import (relative to the importing file), loads the cdylib, and registers its exports as a native
module — so `import { hypot, … } from "ffi:./mathext.lib"` just works on `tish run` (the VM backend).

Marshaling demonstrated: **numbers** (`hypot`, `factorial`), **strings** (`greet`), **arrays**
(`summary` reads an array argument).

## Two ways to write an extension

- **Linked** (this example): the cdylib links `tishlang_ffi` for the accessors. Simplest, but it must
  match the host's `tish_core` **byte layout** — the *same dependency versions* (so it's a workspace
  member) AND the *same value-affecting features* (so `features = ["send-values", "regex"]`, matching
  the shipped `tish`'s `full`). Numbers/strings/arrays are robust; object storage is an `IndexMap`
  whose layout is feature-sensitive, so returning objects across the boundary needs an exact match
  (this example returns a string from `summary` instead).
- **Decoupled** (the production model, next FFI step): the cdylib only *declares* the `tish_value_*`
  accessors `extern "C"` and does NOT link `tish_core`; the host exports them (`-rdynamic`). There is
  then a single `tish_core`, so no layout matching is required and any feature config / dep version
  works — including objects.
