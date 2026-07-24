//! Build native binary via cargo (interim path until Cranelift backend is ready).

use std::fs;
use std::path::Path;

use tishlang_compile::ResolvedNativeModule;

use crate::config::{NativeArtifact, NativeBuildConfig};

/// `tishlang_runtime` Cargo feature names (subset of CLI / compile feature names).
const RUNTIME_CARGO_FEATURES: &[&str] = &[
    "http",
    "http-hyper",
    "http-io-uring",
    "fs",
    "process",
    "regex",
    "ws",
    "tty",
    "pty",
];

/// Map CLI/compile features to flags passed to `tishlang_runtime` in the temp crate's Cargo.toml.
/// `full` enables every optional runtime capability (matches `tish build --feature full` / LANGUAGE.md).
fn runtime_features_for_cargo(features: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for f in features {
        if f == "full" {
            for name in RUNTIME_CARGO_FEATURES {
                if !out.iter().any(|x: &String| x == *name) {
                    out.push((*name).to_string());
                }
            }
            continue;
        }
        if RUNTIME_CARGO_FEATURES.contains(&f.as_str()) && !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

/// `[profile.release]` for nested `cargo build` of generated crates.
fn nested_release_profile_toml() -> &'static str {
    if std::env::var("TISH_FAST_NATIVE_BUILD").as_deref() == Ok("1") {
        r#"[profile.release]
opt-level = 1
lto = false
codegen-units = 16
incremental = true
strip = false
debug = 0
panic = "abort"
"#
    } else {
        r#"[profile.release]
# Reduce binary size: strip symbols, abort on panic (no unwinding), single codegen unit
strip = true
panic = "abort"
codegen-units = 1
lto = "fat"
"#
    }
}

/// Inject `mod generated_native;` after the crate attribute so the binary crate can call `crate::generated_native::…`.
fn inject_generated_native_mod(rust_code: &str) -> String {
    if let Some(pos) = rust_code.find("\n\n") {
        let (a, b) = rust_code.split_at(pos + 2);
        format!("{}mod generated_native;\n{}", a, b)
    } else {
        format!("{}\n\nmod generated_native;\n", rust_code)
    }
}

/// Whether to embed mimalloc as the `#[global_allocator]` of rust-AOT BINARY output. tish workloads
/// are allocation-bound (a sampling profile of object/array code spends most time in malloc/free — see
/// `docs/perf.md`); mimalloc gives ~20% on object/array/bundle code, the same lever as the `tish` CLI's
/// own `fast-alloc` and the reason JSC ships bmalloc. Default ON; `TISH_NATIVE_FAST_ALLOC=0` opts out
/// (e.g. a target whose C toolchain can't build mimalloc). Callers also skip it for staticlib output (a
/// library does not own the final program's allocator) and cross builds (avoid cross-compiling C).
fn fast_alloc_enabled() -> bool {
    std::env::var("TISH_NATIVE_FAST_ALLOC")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Insert a mimalloc `#[global_allocator]` into the generated crate root, after the leading
/// `#![allow(...)]` inner attribute (mirrors [`inject_generated_native_mod`]; an inner attribute must
/// precede any item, and the codegen emits exactly one — `#![allow(unused, non_snake_case)]`).
fn inject_global_allocator(rust_code: &str) -> String {
    const STMT: &str =
        "#[global_allocator]\nstatic TISH_GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;\n\n";
    if let Some(pos) = rust_code.find("\n\n") {
        let (a, b) = rust_code.split_at(pos + 2);
        format!("{a}{STMT}{b}")
    } else {
        format!("{rust_code}\n\n{STMT}")
    }
}

pub(crate) fn rust_code_needs_tokio(rust_code: &str) -> bool {
    rust_code.contains("#[tokio::main]") || rust_code.contains("tokio::runtime::Runtime")
}

pub fn build_via_cargo(
    rust_code: &str,
    native_modules: Vec<ResolvedNativeModule>,
    output_path: &Path,
    features: &[String],
    extra_dependencies_toml: &str,
    generated_native_rs: Option<&str>,
    project_root: Option<&Path>,
) -> Result<(), String> {
    build_via_cargo_with_config(
        rust_code,
        native_modules,
        output_path,
        features,
        extra_dependencies_toml,
        generated_native_rs,
        project_root,
        &NativeBuildConfig::desktop(),
    )
}

#[allow(clippy::too_many_arguments)] // orthogonal cargo build inputs; bundling would just relocate the same fields
pub fn build_via_cargo_with_config(
    rust_code: &str,
    native_modules: Vec<ResolvedNativeModule>,
    output_path: &Path,
    features: &[String],
    extra_dependencies_toml: &str,
    generated_native_rs: Option<&str>,
    project_root: Option<&Path>,
    build_config: &NativeBuildConfig,
) -> Result<(), String> {
    if build_config.artifact == NativeArtifact::GbaRom {
        return build_gba_rom(
            rust_code,
            native_modules,
            output_path,
            extra_dependencies_toml,
            generated_native_rs,
            project_root,
        );
    }

    let out_stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let cargo_name = tishlang_build_utils::cargo_target_name(out_stem);
    let build_dir = tishlang_build_utils::create_build_dir("tish_build", out_stem)?;

    let runtime_path = tishlang_build_utils::find_runtime_path_for_project(project_root)?;

    let runtime_features = runtime_features_for_cargo(features);
    let runtime_refs: Vec<&str> = runtime_features.iter().map(String::as_str).collect();
    let features_str = if runtime_refs.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", runtime_refs)
    };

    let needs_tokio = rust_code_needs_tokio(rust_code);
    let tokio_dep = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n"
    } else {
        ""
    };

    let native_deps: String = native_modules
        .iter()
        .filter(|m| m.use_path_dependency)
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    let mut more_deps = String::new();
    more_deps.push_str(tokio_dep);
    if !native_deps.is_empty() {
        more_deps.push_str(&format!("\n{}", native_deps));
    }
    if !extra_dependencies_toml.trim().is_empty() {
        more_deps.push_str(&format!("\n{}", extra_dependencies_toml));
    }

    let rust_main = if generated_native_rs.is_some() {
        inject_generated_native_mod(rust_code)
    } else {
        rust_code.to_string()
    };

    // mimalloc as the program's global allocator — binary output only (a staticlib does not own the
    // allocator), native only (don't cross-compile mimalloc's C). Adds one cached dep + a global_alloc
    // statement; semantically transparent. `TISH_NATIVE_FAST_ALLOC=0` opts out.
    let use_fast_alloc = fast_alloc_enabled()
        && build_config.artifact != NativeArtifact::StaticLib
        && build_config.cargo_target.is_none();
    if use_fast_alloc {
        more_deps.push_str("\nmimalloc = \"0.1\"\n");
    }
    let rust_main = if use_fast_alloc {
        inject_global_allocator(&rust_main)
    } else {
        rust_main
    };

    let tish_ui_path = std::path::Path::new(&runtime_path)
        .parent()
        .ok_or_else(|| "invalid tishlang_runtime path (no parent)".to_string())?
        .join("tish_ui");
    let ui_dep = if rust_code.contains("tishlang_ui") {
        format!(
            "\ntishlang_ui = {{ path = {:?}, default-features = false, features = [\"runtime\"] }}\n",
            tish_ui_path.display().to_string().replace('\\', "/")
        )
    } else {
        String::new()
    };

    let profile = nested_release_profile_toml();
    let src_file = if build_config.artifact == NativeArtifact::StaticLib {
        "lib.rs"
    } else {
        "main.rs"
    };
    let crate_section = if build_config.artifact == NativeArtifact::StaticLib {
        format!(
            r#"[lib]
name = "{}"
crate-type = ["staticlib"]
path = "src/lib.rs"

"#,
            cargo_name
        )
    } else {
        format!(
            r#"[[bin]]
name = "{}"
path = "src/main.rs"

"#,
            cargo_name
        )
    };
    let cargo_toml = format!(
        r#"[package]
name = "tish_output"
version = "0.1.0"
edition = "2021"

{}{}
[dependencies]
tishlang_runtime = {{ path = {:?}{} }}
{}{}"#,
        crate_section, profile, runtime_path, features_str, more_deps, ui_dep
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;
    if let Some(gen) = generated_native_rs {
        fs::write(build_dir.join("src/generated_native.rs"), gen)
            .map_err(|e| format!("Cannot write generated_native.rs: {}", e))?;
    }
    fs::write(build_dir.join("src").join(src_file), rust_main)
        .map_err(|e| format!("Cannot write {}: {}", src_file, e))?;

    let workspace_target = Path::new(&runtime_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|ws| ws.join("target"));
    let target_dir = workspace_target.filter(|p| p.exists());
    let cross = build_config.cargo_target.as_deref();
    let release_sub = if let Some(triple) = cross {
        format!("{triple}/release")
    } else {
        "release".to_string()
    };
    let binary_dir = target_dir
        .as_ref()
        .map(|t| t.join(&release_sub))
        .unwrap_or_else(|| build_dir.join("target").join(&release_sub));

    tishlang_build_utils::run_cargo_build(&build_dir, target_dir.as_deref(), cross)?;

    let artifact = if build_config.artifact == NativeArtifact::StaticLib {
        tishlang_build_utils::find_release_staticlib(&binary_dir, &cargo_name)?
    } else {
        tishlang_build_utils::find_release_binary(&binary_dir, &cargo_name)?
    };
    let target = if build_config.artifact == NativeArtifact::StaticLib {
        if output_path.extension().is_some_and(|e| e == "a") {
            output_path.to_path_buf()
        } else if output_path.to_string_lossy().ends_with('/') || output_path.is_dir() {
            output_path.join(format!("lib{out_stem}.a"))
        } else {
            output_path.with_extension("a")
        }
    } else {
        tishlang_build_utils::resolve_output_path(output_path, out_stem)
    };
    tishlang_build_utils::copy_binary_to_output(&artifact, &target)?;

    cleanup_build_dir(&build_dir);
    Ok(())
}

/// Build a Game Boy Advance ROM from generated `#![no_std]` Rust.
///
/// Emits an agb-style cargo project (nightly + build-std + `thumbv4t-none-eabi` +
/// `gba.ld`) that links the `tishlang_runtime_gba` facade under the `tishlang_runtime`
/// name (the `package =` rename), builds it, then runs `agb-gbafix` on the ELF.
///
/// Runs its own `cargo build` (NOT `run_cargo_build`) so the `.cargo/config.toml`
/// rustflags (`-Tgba.ld`, `-Ctarget-cpu=arm7tdmi`) apply — `run_cargo_build` sets a
/// `RUSTFLAGS` env that would shadow them.
fn build_gba_rom(
    rust_code: &str,
    native_modules: Vec<ResolvedNativeModule>,
    output_path: &Path,
    extra_dependencies_toml: &str,
    generated_native_rs: Option<&str>,
    project_root: Option<&Path>,
) -> Result<(), String> {
    use std::process::Command;

    // Locate the facade crate (sibling of tishlang_runtime in the tish workspace).
    let runtime_path = tishlang_build_utils::find_runtime_path_for_project(project_root)?;
    let facade_path = Path::new(&runtime_path)
        .parent()
        .ok_or_else(|| "invalid tishlang_runtime path (no parent)".to_string())?
        .join("tish_runtime_gba");
    if !facade_path.exists() {
        return Err(format!(
            "GBA runtime facade not found at {} (expected the tishlang_runtime_gba crate)",
            facade_path.display()
        ));
    }

    let out_stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tish_out");
    let cargo_name = tishlang_build_utils::cargo_target_name(out_stem);

    // Persistent build dir under the project so the (expensive) build-std recompile
    // is cached across runs; the generated Rust stays inspectable.
    let base = project_root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let build_dir = base.join(".tish").join("gba").join(out_stem);
    fs::create_dir_all(build_dir.join("src"))
        .map_err(|e| format!("Cannot create GBA build dir: {}", e))?;
    fs::create_dir_all(build_dir.join(".cargo"))
        .map_err(|e| format!("Cannot create .cargo dir: {}", e))?;

    // Path dependencies for any `cargo:` native modules the program imported.
    let native_deps: String = native_modules
        .iter()
        .filter(|m| m.use_path_dependency)
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    // `cargo:` module path deps arrive via `extra_dependencies_toml`
    // (`tish.rustDependencies`); `native_modules` covers any that use a path dep
    // resolved another way. Both are appended to `[dependencies]`.
    let mut more_deps = native_deps;
    if !extra_dependencies_toml.trim().is_empty() {
        more_deps.push('\n');
        more_deps.push_str(extra_dependencies_toml);
    }

    let facade = facade_path.display().to_string().replace('\\', "/");
    let cargo_toml = format!(
        r#"[package]
name = "tish_output"
version = "0.1.0"
edition = "2021"

# Standalone workspace so a parent Cargo workspace (e.g. the tish-gba repo, when
# the build dir lives under it) doesn't try to absorb this generated crate.
[workspace]

[[bin]]
name = "{cargo_name}"
path = "src/main.rs"

[dependencies]
tishlang_runtime = {{ package = "tishlang_runtime_gba", path = {facade:?} }}
agb = "0.25.0"
{more_deps}
[profile.dev]
opt-level = 3
debug = true

[profile.dev.build-override]
opt-level = 3

[profile.release]
opt-level = 3
lto = "fat"
debug = true

[profile.release.build-override]
opt-level = 3
"#,
    );
    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;

    fs::write(
        build_dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"nightly\"\ncomponents = [\"rust-src\"]\n",
    )
    .map_err(|e| format!("Cannot write rust-toolchain.toml: {}", e))?;

    // build-std + target + gba.ld rustflags (agb ships gba.ld via its build.rs).
    fs::write(
        build_dir.join(".cargo").join("config.toml"),
        r#"[unstable]
build-std = ["core", "alloc"]
build-std-features = ["compiler-builtins-mem"]

[build]
target = "thumbv4t-none-eabi"

[target.thumbv4t-none-eabi]
runner = ["mgba-qt", "-C", "logToStdout=1", "-C", "logLevel.gba.debug=127"]
"#,
    )
    .map_err(|e| format!("Cannot write .cargo/config.toml: {}", e))?;

    let rust_main = if generated_native_rs.is_some() {
        inject_generated_native_mod(rust_code)
    } else {
        rust_code.to_string()
    };
    if let Some(gen) = generated_native_rs {
        // The `cargo:` wrapper emits the std header (`std::cell/rc/sync`); map it
        // to the no_std facade equivalents (`alloc` is crate-wide via main.rs's
        // `extern crate alloc`). The module body is otherwise Value-only.
        let gen = gen
            .replace("use std::cell::RefCell;", "use core::cell::RefCell;")
            .replace("use std::rc::Rc;", "use alloc::rc::Rc;")
            .replace("use std::sync::Arc;", "use tishlang_runtime::Arc;");
        fs::write(build_dir.join("src").join("generated_native.rs"), gen)
            .map_err(|e| format!("Cannot write generated_native.rs: {}", e))?;
    }
    fs::write(build_dir.join("src").join("main.rs"), rust_main)
        .map_err(|e| format!("Cannot write main.rs: {}", e))?;

    // Pass the GBA linker/target rustflags via the RUSTFLAGS env (which REPLACES,
    // not joins, any `[target.*].rustflags` from config files). This matters when
    // the build dir sits inside a project that already has a `.cargo/config.toml`
    // with `-Tgba.ld` — cargo *joins* rustflags arrays across config files, so a
    // config-file approach would pass `-Tgba.ld` twice ("region already defined").
    // build-std still comes from the generated `[unstable]` config (not rustflags).
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&build_dir)
        .env(
            "RUSTFLAGS",
            "-Clink-arg=-Tgba.ld -Ctarget-cpu=arm7tdmi -Cforce-frame-pointers=yes",
        )
        .env("CARGO_TERM_COLOR", "always")
        .status()
        .map_err(|e| format!("Failed to run cargo for GBA build: {}", e))?;
    if !status.success() {
        return Err(format!(
            "GBA cargo build failed (see output above). Build dir: {}",
            build_dir.display()
        ));
    }

    // ELF → .gba via agb-gbafix.
    let elf = build_dir
        .join("target")
        .join("thumbv4t-none-eabi")
        .join("release")
        .join(&cargo_name);
    if !elf.exists() {
        return Err(format!("GBA ELF not found at {}", elf.display()));
    }
    let rom = if output_path.extension().is_some_and(|e| e == "gba") {
        output_path.to_path_buf()
    } else {
        output_path.with_extension("gba")
    };
    let gbafix = Command::new("agb-gbafix")
        .arg(&elf)
        .arg("-o")
        .arg(&rom)
        .status()
        .map_err(|e| {
            format!(
                "Failed to run agb-gbafix ({}). Install it with: cargo install agb-gbafix",
                e
            )
        })?;
    if !gbafix.success() {
        return Err("agb-gbafix failed to produce the ROM".to_string());
    }
    Ok(())
}

/// Remove a finished build's per-PID source directory (#384). Called only on SUCCESS, after the
/// binary is copied out — the dir is then disposable (it holds only the generated crate source;
/// compiled artifacts live in the shared workspace `target/`). Left in place on any earlier error so a
/// failed build can be inspected, and preserved entirely when `TISH_KEEP_BUILD_DIR=1` (e.g. to read the
/// generated `main.rs`). Best-effort: a failed removal is ignored.
pub(crate) fn cleanup_build_dir(build_dir: &Path) {
    if std::env::var("TISH_KEEP_BUILD_DIR").as_deref() == Ok("1") {
        return;
    }
    let _ = std::fs::remove_dir_all(build_dir);
}

/// Build several native binaries in **one** nested Cargo project (shared `tishlang_runtime` compile).
///
/// `bins` order must match `outputs`: each `(stem, rust_code, generated_native_rs)` pairs with
/// `outputs[i].0` (entry path — used only for validation) and `outputs[i].1` (final binary path).
#[allow(clippy::too_many_arguments)] // orthogonal batch-build inputs (bins/outputs/modules/flags)
pub(crate) fn build_many_via_cargo(
    bins: Vec<(String, String, Option<String>)>,
    native_modules: Vec<ResolvedNativeModule>,
    features: &[String],
    extra_dependencies_toml: &str,
    needs_tokio: bool,
    needs_ui: bool,
    outputs: &[(&Path, &Path)],
    project_root: Option<&Path>,
) -> Result<(), String> {
    if bins.len() != outputs.len() {
        return Err(format!(
            "build_many_via_cargo: bins ({}) != outputs ({})",
            bins.len(),
            outputs.len()
        ));
    }
    for (i, (stem, _, _)) in bins.iter().enumerate() {
        let entry = outputs[i].0;
        let expect = entry.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if expect != stem {
            return Err(format!(
                "build_many_via_cargo: stem mismatch at {}: {} vs {}",
                i, stem, expect
            ));
        }
    }

    let batch_id = format!("many_{}", std::process::id());
    let build_dir = tishlang_build_utils::create_build_dir("tish_build_many", &batch_id)?;

    let runtime_path = tishlang_build_utils::find_runtime_path_for_project(project_root)?;

    let runtime_features = runtime_features_for_cargo(features);
    let runtime_refs: Vec<&str> = runtime_features.iter().map(String::as_str).collect();
    let features_str = if runtime_refs.is_empty() {
        String::new()
    } else {
        format!(", features = {:?}", runtime_refs)
    };

    let tokio_dep = if needs_tokio {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n"
    } else {
        ""
    };

    let native_deps: String = native_modules
        .iter()
        .filter(|m| m.use_path_dependency)
        .map(|m| {
            let path = m.crate_path.display().to_string().replace('\\', "/");
            format!("{} = {{ path = {:?} }}\n", m.package_name, path)
        })
        .collect();

    let mut more_deps = String::new();
    more_deps.push_str(tokio_dep);
    if !native_deps.is_empty() {
        more_deps.push_str(&format!("\n{}", native_deps));
    }
    if !extra_dependencies_toml.trim().is_empty() {
        more_deps.push_str(&format!("\n{}", extra_dependencies_toml));
    }
    // mimalloc global allocator for every binary in the batch (all are executables, always native here).
    let use_fast_alloc = fast_alloc_enabled();
    if use_fast_alloc {
        more_deps.push_str("\nmimalloc = \"0.1\"\n");
    }

    let tish_ui_path = std::path::Path::new(&runtime_path)
        .parent()
        .ok_or_else(|| "invalid tishlang_runtime path (no parent)".to_string())?
        .join("tish_ui");
    let ui_dep = if needs_ui {
        format!(
            "\ntishlang_ui = {{ path = {:?}, default-features = false, features = [\"runtime\"] }}\n",
            tish_ui_path.display().to_string().replace('\\', "/")
        )
    } else {
        String::new()
    };

    let mut bin_tables = String::new();
    for (stem, rust_code, generated_native_rs) in &bins {
        let bin_dir = build_dir.join("src/bin").join(stem);
        fs::create_dir_all(&bin_dir).map_err(|e| format!("create bin dir: {}", e))?;

        let rust_main = if generated_native_rs.is_some() {
            inject_generated_native_mod(rust_code)
        } else {
            rust_code.clone()
        };
        let rust_main = if use_fast_alloc {
            inject_global_allocator(&rust_main)
        } else {
            rust_main
        };

        fs::write(bin_dir.join("main.rs"), rust_main)
            .map_err(|e| format!("write main.rs for {}: {}", stem, e))?;
        if let Some(gen) = generated_native_rs {
            fs::write(bin_dir.join("generated_native.rs"), gen)
                .map_err(|e| format!("write generated_native.rs for {}: {}", stem, e))?;
        }

        bin_tables.push_str(&format!(
            r#"[[bin]]
name = "{stem}"
path = "src/bin/{stem}/main.rs"

"#
        ));
    }

    let profile = nested_release_profile_toml();
    let cargo_toml = format!(
        r#"[package]
name = "tish_output_many"
version = "0.1.0"
edition = "2021"

{}{}
[dependencies]
tishlang_runtime = {{ path = {:?}{} }}
{}{}"#,
        bin_tables, profile, runtime_path, features_str, more_deps, ui_dep
    );

    fs::write(build_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Cannot write Cargo.toml: {}", e))?;

    let workspace_target = Path::new(&runtime_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|ws| ws.join("target"));
    let target_dir = workspace_target.filter(|p| p.exists());
    let binary_dir = target_dir
        .as_ref()
        .map(|t| t.join("release"))
        .unwrap_or_else(|| build_dir.join("target").join("release"));

    tishlang_build_utils::run_cargo_build(&build_dir, target_dir.as_deref(), None)?;

    for i in 0..bins.len() {
        let stem = bins[i].0.as_str();
        let output_path = outputs[i].1;
        let binary = tishlang_build_utils::find_release_binary(binary_dir.as_path(), stem)?;
        let target = tishlang_build_utils::resolve_output_path(output_path, stem);
        tishlang_build_utils::copy_binary_to_output(&binary, &target)?;
    }

    cleanup_build_dir(&build_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::runtime_features_for_cargo;

    /// #384: `cleanup_build_dir` removes a finished build's dir by default, and preserves it under
    /// `TISH_KEEP_BUILD_DIR=1`.
    #[test]
    fn cleanup_build_dir_removes_by_default_and_keeps_on_flag() {
        use std::path::PathBuf;
        // Scratch under the workspace target/ (never /tmp), unique per case.
        let base: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp_cleanup_test");
        let removed = base.join("to_remove");
        let kept = base.join("to_keep");
        for d in [&removed, &kept] {
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(d.join("src/main.rs"), "fn main(){}").unwrap();
        }

        std::env::remove_var("TISH_KEEP_BUILD_DIR");
        super::cleanup_build_dir(&removed);
        assert!(!removed.exists(), "default should remove the build dir");

        std::env::set_var("TISH_KEEP_BUILD_DIR", "1");
        super::cleanup_build_dir(&kept);
        assert!(kept.exists(), "TISH_KEEP_BUILD_DIR=1 should preserve the build dir");
        std::env::remove_var("TISH_KEEP_BUILD_DIR");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn runtime_features_full_expands() {
        let f = runtime_features_for_cargo(&["full".to_string()]);
        assert!(f.contains(&"http".to_string()));
        assert!(f.contains(&"fs".to_string()));
        assert!(f.contains(&"process".to_string()));
        assert!(f.contains(&"regex".to_string()));
        assert!(f.contains(&"ws".to_string()));
    }

    #[test]
    fn runtime_features_merges_full_and_specific() {
        let f = runtime_features_for_cargo(&["full".to_string(), "http".to_string()]);
        // `full` expands to every RUNTIME_CARGO_FEATURES entry; redundant `http` must not duplicate.
        assert_eq!(f.len(), super::RUNTIME_CARGO_FEATURES.len());
        assert_eq!(f.iter().filter(|x| *x == "http").count(), 1);
    }
}
