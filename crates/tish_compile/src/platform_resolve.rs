//! React Native–style platform/surface file resolution for `.tish` imports.
//!
//! See docs/LANGUAGE.md (platform extensions). Context is process-wide so existing
//! `resolve_project` callers pick up `--platform` / `--surface` / env without signature churn.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
    #[default]
    Unknown,
    Macos,
    Ios,
    Android,
    Windows,
    Linux,
    Web,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    #[default]
    Unknown,
    Native,
    Webview,
    Web,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolveContext {
    pub platform: Platform,
    pub surface: Surface,
}

fn resolve_ctx_lock() -> &'static Mutex<ResolveContext> {
    static RESOLVE_CTX: OnceLock<Mutex<ResolveContext>> = OnceLock::new();
    RESOLVE_CTX.get_or_init(|| Mutex::new(ResolveContext::default()))
}

pub fn set_resolve_context(ctx: ResolveContext) {
    *resolve_ctx_lock().lock().unwrap_or_else(|e| e.into_inner()) = ctx;
}

pub fn resolve_context() -> ResolveContext {
    *resolve_ctx_lock().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn parse_platform(s: &str) -> Option<Platform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "macos" | "darwin" | "osx" => Some(Platform::Macos),
        "ios" => Some(Platform::Ios),
        "android" => Some(Platform::Android),
        "windows" | "win32" | "win" => Some(Platform::Windows),
        "linux" => Some(Platform::Linux),
        "web" => Some(Platform::Web),
        "" | "unknown" | "default" => Some(Platform::Unknown),
        _ => None,
    }
}

pub fn parse_surface(s: &str) -> Option<Surface> {
    match s.trim().to_ascii_lowercase().as_str() {
        "native" => Some(Surface::Native),
        "webview" => Some(Surface::Webview),
        "web" => Some(Surface::Web),
        "" | "unknown" | "default" => Some(Surface::Unknown),
        _ => None,
    }
}

/// Apply CLI/env into the process resolve context.
/// `TISH_PLATFORM` / `TISH_SURFACE` fill gaps when args are omitted.
pub fn apply_resolve_env(platform: Option<&str>, surface: Option<&str>) -> Result<(), String> {
    let mut ctx = resolve_context();
    if let Some(p) = platform {
        ctx.platform = parse_platform(p).ok_or_else(|| format!("unknown --platform '{p}'"))?;
    } else if let Ok(p) = std::env::var("TISH_PLATFORM") {
        if let Some(parsed) = parse_platform(&p) {
            ctx.platform = parsed;
        }
    }
    if let Some(s) = surface {
        ctx.surface = parse_surface(s).ok_or_else(|| format!("unknown --surface '{s}'"))?;
    } else if let Ok(s) = std::env::var("TISH_SURFACE") {
        if let Some(parsed) = parse_surface(&s) {
            ctx.surface = parsed;
        }
    }
    // `--platform web` implies surface web when surface unset
    if ctx.platform == Platform::Web && ctx.surface == Surface::Unknown {
        ctx.surface = Surface::Web;
    }
    set_resolve_context(ctx);
    Ok(())
}

/// Suffix tokens to try (before bare `.tish`), most specific first.
pub fn platform_suffixes(ctx: ResolveContext) -> Vec<&'static str> {
    let mut out = Vec::new();
    match (ctx.platform, ctx.surface) {
        (_, Surface::Web) | (Platform::Web, _) => {
            out.push("web");
        }
        (Platform::Macos, Surface::Native) | (Platform::Macos, Surface::Unknown) => {
            out.extend(["macos", "desktop", "native"]);
        }
        (Platform::Ios, Surface::Native) | (Platform::Ios, Surface::Unknown) => {
            out.extend(["ios", "mobile", "native"]);
        }
        (Platform::Android, Surface::Native) | (Platform::Android, Surface::Unknown) => {
            out.extend(["android", "mobile", "native"]);
        }
        (Platform::Windows, Surface::Native) | (Platform::Windows, Surface::Unknown) => {
            out.extend(["windows", "desktop", "native"]);
        }
        (Platform::Linux, Surface::Native) | (Platform::Linux, Surface::Unknown) => {
            out.extend(["linux", "desktop", "native"]);
        }
        (Platform::Macos, Surface::Webview)
        | (Platform::Windows, Surface::Webview)
        | (Platform::Linux, Surface::Webview)
        | (Platform::Unknown, Surface::Webview) => {
            out.extend(["webview", "web", "desktop"]);
        }
        (Platform::Ios, Surface::Webview) | (Platform::Android, Surface::Webview) => {
            out.extend(["webview", "web", "mobile"]);
        }
        _ => {}
    }
    out
}

/// After normalizing a virtual path to a stem (no `.tish`), list cascade keys to probe.
/// `normalized_stem` is e.g. `app/Button` (no extension).
pub fn platform_virtual_keys(normalized_stem: &str, ctx: ResolveContext) -> Vec<String> {
    let parent = Path::new(normalized_stem)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| !p.is_empty() && p != ".");
    let file_stem = Path::new(normalized_stem)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("index");
    let mut keys = Vec::new();
    for suffix in platform_suffixes(ctx) {
        keys.push(match &parent {
            Some(p) => format!("{p}/{file_stem}.{suffix}.tish"),
            None => format!("{file_stem}.{suffix}.tish"),
        });
    }
    keys.push(match &parent {
        Some(p) => format!("{p}/{file_stem}.tish"),
        None => format!("{file_stem}.tish"),
    });
    keys
}

/// Resolve a relative import specifier to an on-disk path, trying platform variants.
///
/// `spec` is e.g. `./Button` or `./Button.tish`. Never picks native-only suffixes for web surface.
pub fn resolve_with_platform(
    spec: &str,
    from_dir: &Path,
    ctx: ResolveContext,
) -> Option<PathBuf> {
    let joined = from_dir.join(spec);
    let (stem_path, had_tish_ext) = if joined.extension().and_then(|e| e.to_str()) == Some("tish")
    {
        (joined.with_extension(""), true)
    } else if joined.extension().is_none() {
        (joined, false)
    } else {
        // Other extensions (.js, .css): leave to caller / no platform remap
        return if joined.exists() {
            joined.canonicalize().ok()
        } else {
            None
        };
    };

    let parent = stem_path.parent().unwrap_or(from_dir);
    let file_stem = stem_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("index");

    for suffix in platform_suffixes(ctx) {
        let candidate = parent.join(format!("{file_stem}.{suffix}.tish"));
        if candidate.exists() {
            return candidate.canonicalize().ok();
        }
    }

    let base = parent.join(format!("{file_stem}.tish"));
    if base.exists() {
        return base.canonicalize().ok();
    }

    // Explicit path without remap that already exists (edge)
    if had_tish_ext {
        let explicit = from_dir.join(spec);
        if explicit.exists() {
            return explicit.canonicalize().ok();
        }
    }

    None
}

/// Public one-shot for CLI / Vite: resolve `importer` + `source` → absolute path string.
pub fn resolve_id_public(
    source: &str,
    importer: Option<&str>,
    platform: Option<&str>,
    surface: Option<&str>,
) -> Result<String, String> {
    apply_resolve_env(platform, surface)?;
    let ctx = resolve_context();
    let from_dir = if let Some(imp) = importer {
        Path::new(imp)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    if !source.starts_with("./") && !source.starts_with("../") {
        return Err(format!(
            "resolve-id only handles relative imports (got '{source}')"
        ));
    }

    resolve_with_platform(source, &from_dir, ctx)
        .map(|p| p.display().to_string())
        .ok_or_else(|| {
            let tried: Vec<_> = platform_suffixes(ctx)
                .into_iter()
                .map(|s| format!("*.{s}.tish"))
                .collect();
            format!(
                "Cannot resolve '{source}' from {} (tried {} then *.tish)",
                from_dir.display(),
                if tried.is_empty() {
                    "(no platform suffixes)".into()
                } else {
                    tried.join(", ")
                }
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn macos_native_prefers_macos_then_desktop() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Button.desktop.tish"), "").unwrap();
        fs::write(root.join("Button.macos.tish"), "").unwrap();
        fs::write(root.join("Button.tish"), "").unwrap();
        let ctx = ResolveContext {
            platform: Platform::Macos,
            surface: Surface::Native,
        };
        let p = resolve_with_platform("./Button", root, ctx).unwrap();
        assert!(p.ends_with("Button.macos.tish"));
    }

    #[test]
    fn webview_reuses_web_not_macos() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Button.macos.tish"), "").unwrap();
        fs::write(root.join("Button.web.tish"), "").unwrap();
        let ctx = ResolveContext {
            platform: Platform::Macos,
            surface: Surface::Webview,
        };
        let p = resolve_with_platform("./Button", root, ctx).unwrap();
        assert!(p.ends_with("Button.web.tish"));
    }

    #[test]
    fn web_surface_only_web_and_base() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Button.macos.tish"), "").unwrap();
        fs::write(root.join("Button.web.tish"), "").unwrap();
        let ctx = ResolveContext {
            platform: Platform::Web,
            surface: Surface::Web,
        };
        let p = resolve_with_platform("./Button.tish", root, ctx).unwrap();
        assert!(p.ends_with("Button.web.tish"));
    }

    #[test]
    fn android_native_prefers_android_then_mobile() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Button.mobile.tish"), "").unwrap();
        fs::write(root.join("Button.android.tish"), "").unwrap();
        fs::write(root.join("Button.tish"), "").unwrap();
        let ctx = ResolveContext {
            platform: Platform::Android,
            surface: Surface::Native,
        };
        let p = resolve_with_platform("./Button", root, ctx).unwrap();
        assert!(p.ends_with("Button.android.tish"));
    }

    #[test]
    fn ios_native_falls_back_to_mobile() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Button.mobile.tish"), "").unwrap();
        fs::write(root.join("Button.tish"), "").unwrap();
        let ctx = ResolveContext {
            platform: Platform::Ios,
            surface: Surface::Native,
        };
        let p = resolve_with_platform("./Button", root, ctx).unwrap();
        assert!(p.ends_with("Button.mobile.tish"));
    }
}
