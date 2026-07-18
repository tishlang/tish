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

/// Locate a `tish` binary that supports `resolve-id`.
///
/// Does **not** shell out to `cargo build --bin tish` (under `cargo llvm-cov` that can shadow the
/// instrumented CLI). Searches the same places as `integration_test::tish_bin()`, including
/// `llvm-cov-target`. Callers must fail hard if this returns `None` — CI must not soft-skip.
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

    let tish = find_tish_binary().unwrap_or_else(|| {
        panic!(
            "tish binary with resolve-id required (set TISH_PATH or cargo build -p tishlang --bin tish). \
             Soft-skip is forbidden — Vite and tish build must share resolve rules."
        )
    });
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
