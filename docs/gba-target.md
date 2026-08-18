# The GBA target — separation of concerns (compiler side)

How Game Boy Advance support is layered so that **this repo stays a target-agnostic
language**, and GBA/agb specifics are pushed to the edge. Read this before adding or
moving GBA code in the tish repo. The framework side (bindings + engine + games) is
documented in the **tish-gba** repo's `ARCHITECTURE.md`; the compiler↔framework wire
format is pinned in that repo's `CONTRACT.md`.

## The four layers

```
  ── tish repo (this repo) ─────────────────────────────────────────────┐
  │                                                                      │
  │  ①  tish_compile / tish_native / tish (CLI)   — the COMPILER.        │
  │       Target-agnostic. Emits Rust source as strings. Knows how to    │
  │       DRIVE a GBA build (scaffold + toolchain), but links no agb.    │
  │                                                                      │
  │  ②  tish_core / tish_builtins  — the LANGUAGE RUNTIME primitives.    │
  │       Generic. `portable` feature = no_std+alloc for ANY embedded    │
  │       target (GBA is the first). No agb types anywhere.              │
  │                                                                      │
  │  ③  tish_runtime_gba  — the GBA RUNTIME FACADE. THE BOUNDARY.        │
  │       The ONE crate that depends on `agb`. no_std. Excluded from     │
  │       the workspace. Versions in lockstep with the compiler.         │
  └──────────────────────────────────────────────────────────────────────┘
                                    │  links (path dep, package-renamed to `tishlang_runtime`)
  ── tish-gba repo ─────────────────▼────────────────────────────────────┐
  │  ④  tish-agb (idiomatic agb bindings) · tish-gba-game-engine ·        │
  │     packages/ (tish sugar) · examples/ (games)                        │
  └──────────────────────────────────────────────────────────────────────┘
```

The design goal: **as little GBA/agb-specific code in this repo as possible**, and
whatever must be here is either (a) confined to the facade `tish_runtime_gba`, or
(b) inert unless you actually build with `--target gba`.

## The hard invariant

> **No compiler or core crate depends on `agb`.** The only crate with an `agb`
> dependency is `tish_runtime_gba`, and it is `exclude`d from the workspace
> (root `Cargo.toml`). A serverless / desktop / web build of tish links zero
> agb / GBA code.

This is enforced structurally today (verified: `tish_compile`, `tish_native`, and
the `tish` CLI have no agb in their `Cargo.toml`s) and is the line any change must
preserve. If you find yourself wanting to `use agb::…` outside the facade, that code
belongs in the facade (or, for hardware bindings, in the tish-gba repo's `tish-agb`).

## What is generic (not GBA-specific)

- **The `portable` feature** on `tish_core` / `tish_builtins` (`#![cfg_attr(feature =
  "portable", no_std)]`) is a generic **no_std + alloc** build for embedded targets. It
  swaps std-only deps (ahash/arcstr) and primitives (`std::sync::Arc`, atomics,
  `thread_local!`) for atomics-free, alloc-based equivalents via `compat.rs`, and reads
  runtime `install_clock` / `install_rng` **hooks** the target fills in. It contains **no
  agb types** — GBA is named only as the motivating example. Default builds (`std`) are
  unaffected. `portable` requires `default-features = false` (a `compile_error!` catches
  the `portable + std` slip; another catches `portable + send-values`).
- **The import-scheme registry** (`tish_compile/src/schemes.rs`) is a generic extension
  seam. `SchemeRegistry::builtin()` ships **zero** schemes — tish core knows nothing about
  `asset:`/`sheet:`/`map:`. Those are contributed by `tish-agb`'s `tish.schemes.json`,
  auto-discovered from a game's `rustDependencies`. New asset kinds need zero core edits.
- **Narrow integer widths** (`i8/u8/i16/u16/u32`) and the **struct-field-write / `emit_f64`**
  fast paths are generic typed-lowering improvements, active on every native target.

## The GBA footprint that IS in this repo

All of it is either string emission (no agb link) or the build scaffold, and all of it
is gated behind `emit_mode == NativeEmitMode::Gba` / `--target gba` — unreachable for a
serverless/desktop compile.

| Where | What | Notes |
|---|---|---|
| `tish_compile/src/lib.rs` | `NativeEmitMode::Gba` variant | An enum discriminant. |
| `tish_compile/src/codegen.rs` | ~24 `emit_mode == Gba` sites: no_std header, the `#[agb::entry] agb_main` entry, `gba_no_std_rewrite` (std→core post-pass), scheme-module emission, perf-pass/PropIC/OnceLock gating | All **string** emission — no agb dependency. |
| `tish_compile/src/types.rs` + `codegen.rs` | `RustType::Fixed` + the `fixed` lowering (emits the `tishlang_runtime::Fixed` alias) | Activates only on a `fixed` annotation. Emits the facade alias, not `agb::` directly. |
| `tish_native/src/{config.rs,build.rs}` | `NativeBuildConfig::gba()`, `build_gba_rom` (thumbv4t scaffold, `gba.ld`, `agb-gbafix`) | The build driver. Shells out; links no agb into the compiler. The agb **version** is read from the facade's `Cargo.toml` (`read_facade_agb_version`), not hardcoded — one source of truth. |
| `tish/src/main.rs` | `--target gba` CLI handling | Selects `NativeBuildConfig::gba()`. |

### Known couplings (candidates to push further out)

These are the few spots where agb-specific knowledge sits outside the facade. They are
documented here so the boundary is honest; none breaks the "no agb dependency" invariant
(they are strings / build config), and each has a note on whether it's intentional:

1. **`#[agb::entry] fn agb_main(gba: agb::Gba)`** emitted as string literals in `codegen.rs`.
   *Intentional for now* — it's the ROM entry the facade's `gba::init/halt` plug into. A
   future cleanup could move it behind a facade macro so the compiler emits
   `tishlang_runtime::agb_main!(run)` and the agb API names live in the facade.
2. **Q24.8 fixed-point representation** (`*256`, `from_raw`, `to_raw`) in the `fixed`↔`Value`
   lowering. It routes through the `tishlang_runtime::Fixed` alias (not `agb::`), but the
   *format* is agb_fixnum's. The compile-time literal fold needs the format; the runtime
   boundary conversions could later defer to facade helpers (`fixed_from_f64` / `fixed_to_f64`).
3. **The scheme emit-target key is the literal `"gba"`** (`codegen.rs`, `scheme_target_emit`).
   The registry is generic; only this consumer assumes the target is named `gba`. Trivially
   parameterizable from the emit mode when a second target appears.
4. **Toolchain specifics** (`-Tgba.ld`, `-Ctarget-cpu=arm7tdmi`, `mgba-qt`, `agb-gbafix`) in
   `build_gba_rom`. Inherent to "produce a GBA ROM" — this is the compiler's build-driver job.

## Server-stability note

tish is used mainly for serverless functions / desktop / web, not games. Because agb is
not a dependency of any compiled-in crate and the GBA emit branches are `emit_mode`-gated,
the **runtime and dependency risk to non-GBA builds is nil**. The one residual is *shared
build surface*: the GBA emit code compiles into the mainline binary, so a GBA-code compile
error would break everyone's `tish_compile` build.

### Why there is no separate `gba` Cargo feature (decision)

There is exactly **one** embedded carve-out — the **`portable`** feature on
`tish_core`/`tish_builtins`, which makes the *runtime* no_std. That is the single "mode",
and it lives on the runtime crates (consumed only by the facade). We deliberately do **not**
add a second, `gba`, feature to gate the compiler's emit mode, because that code (a) has no
agb dependency and (b) is already `emit_mode == Gba`-gated, so it never runs for — and adds
no agb/link risk to — a serverless build. A parallel feature flag would be redundant surface
for a marginal, maintenance-only concern. If the shared-build-surface residual ever bites,
the fix is a **CI job** that builds/tests the non-GBA paths (cheap, keeps the shared codegen
honest), not another feature. See the tish-gba repo's `docs/gba-in-tish-core.md` for the full
risk write-up; this document is the primary separation reference.
