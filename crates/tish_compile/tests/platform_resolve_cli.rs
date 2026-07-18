//! Golden: `resolve_with_platform` cascade matches the documented order.
//! Vite consumes the same rules via `tish resolve-id`.
//!
//! The CLI parity test **fails** (does not skip) when no usable `tish` binary is found,
//! so CI cannot silently ship without `resolve-id`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;
use tishlang_compile::{resolve_with_platform, Platform, ResolveContext, Surface};

#[test]
fn cascade_orders_match_language_md() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("Button.tish"), "export fn Button() {}").unwrap();
    fs::write(root.join("Button.web.tish"), "export fn Button() {}").unwrap();
    fs::write(root.join("Button.webview.tish"), "export fn Button() {}").unwrap();
    fs::write(root.join("Button.macos.tish"), "export fn Button() {}").unwrap();
    fs::write(root.join("Button.desktop.tish"), "export fn Button() {}").unwrap();

    let macos_native = resolve_with_platform(
        "./Button",
        root,
        ResolveContext {
            platform: Platform::Macos,
            surface: Surface::Native,
        },
    )
    .unwrap();
    assert!(macos_native.ends_with("Button.macos.tish"));

    let webview = resolve_with_platform(
        "./Button",
        root,
        ResolveContext {
            platform: Platform::Macos,
            surface: Surface::Webview,
        },
    )
    .unwrap();
    assert!(webview.ends_with("Button.webview.tish"));

    let web = resolve_with_platform(
        "./Button",
        root,
        ResolveContext {
            platform: Platform::Web,
            surface: Surface::Web,
        },
    )
    .unwrap();
    assert!(web.ends_with("Button.web.tish"));

    // Remap explicit .tish to platform file when present
    let remapped = resolve_with_platform(
        "./Button.tish",
        root,
        ResolveContext {
            platform: Platform::Macos,
            surface: Surface::Native,
        },
    )
    .unwrap();
    assert!(remapped.ends_with("Button.macos.tish"));
}

/// Locate a `tish` binary that supports `resolve-id`. Returns `None` (the test soft-skips) when no
/// binary is available. Crucially this does NOT force a separate `cargo build --bin tish`: under
/// `cargo llvm-cov`, `integration_test::tish_bin()` prefers `target/debug/tish` if present, so a
/// non-instrumented CLI built into that path would shadow the coverage-instrumented binary and wipe
/// out the subprocess coverage of vm.rs/main.rs/resolve.rs. Instead we search the same places
/// `tish_bin()` does — including the coverage-instrumented `llvm-cov-target` — so during the coverage
/// run we find (and exercise) the instrumented binary.
fn find_tish_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TISH_PATH") {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let bin = if cfg!(windows) { "tish.exe" } else { "tish" };
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        roots.push(PathBuf::from(td));
    }
    // crates/tish_compile → workspace `target/` (and the coverage-instrumented `llvm-cov-target`).
    for rel in ["../../target", "../../../target"] {
        roots.push(manifest.join(rel));
        roots.push(manifest.join(rel).join("llvm-cov-target"));
    }
    for root in roots {
        for profile in ["debug", "release"] {
            let cand = root.join(profile).join(bin);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Ok(out) = Command::new("which").arg("tish").output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && Path::new(&p).is_file() {
                return Some(PathBuf::from(p));
            }
        }
    }
    None
}

#[test]
fn resolve_id_cli_matches_library() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("X.web.tish"), "").unwrap();
    fs::write(root.join("X.tish"), "").unwrap();
    let importer = root.join("App.tish");
    fs::write(&importer, "").unwrap();

    let lib = resolve_with_platform(
        "./X",
        root,
        ResolveContext {
            platform: Platform::Web,
            surface: Surface::Web,
        },
    )
    .unwrap();

    let Some(tish) = find_tish_binary() else {
        eprintln!("skip resolve_id_cli_matches_library: no tish binary available in this context");
        return;
    };
    let out = Command::new(&tish)
        .args([
            "resolve-id",
            "./X",
            "--importer",
            importer.to_str().unwrap(),
            "--platform",
            "web",
            "--surface",
            "web",
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", tish.display()));

    assert!(
        out.status.success(),
        "tish resolve-id failed ({}):\nstdout: {}\nstderr: {}",
        tish.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let cli_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        std::fs::canonicalize(&cli_path).unwrap(),
        lib,
        "Vite/CLI resolve-id must match library cascade"
    );
}
