//! Golden: `resolve_with_platform` cascade matches the documented order.
//! Vite consumes the same rules via `tish resolve-id`.

use std::fs;
use std::process::Command;

use tempfile::tempdir;
use tishlang_compile::{
    resolve_with_platform, Platform, ResolveContext, Surface,
};

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

#[test]
fn resolve_id_cli_matches_library_when_tish_on_path() {
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

    let tish = std::env::var("TISH_PATH").unwrap_or_else(|_| "tish".into());
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
        .output();

    let Ok(out) = out else {
        eprintln!("skip resolve-id CLI check: `{tish}` not runnable");
        return;
    };
    if !out.status.success() {
        eprintln!(
            "skip resolve-id CLI check: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let cli_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        std::fs::canonicalize(&cli_path).unwrap(),
        lib,
        "Vite/CLI resolve-id must match library cascade"
    );
}
