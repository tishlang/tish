//! Native build configuration (desktop binary vs iOS staticlib, cross-target).

use tishlang_compile::NativeEmitMode;

/// Output artifact kind for `tish build --target native`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeArtifact {
    #[default]
    Bin,
    StaticLib,
    /// A GBA ROM: cargo builds an ELF for `thumbv4t-none-eabi`, then `agb-gbafix`
    /// converts it to a `.gba`.
    GbaRom,
}

/// Options passed from the CLI into nested `cargo build`.
#[derive(Debug, Clone, Default)]
pub struct NativeBuildConfig {
    pub artifact: NativeArtifact,
    /// When set, run `cargo build --target <triple>` and skip `-C target-cpu=native`.
    pub cargo_target: Option<String>,
    pub emit_mode: NativeEmitMode,
}

impl NativeBuildConfig {
    pub fn desktop() -> Self {
        Self::default()
    }

    pub fn ios_staticlib(triple: &str) -> Self {
        Self {
            artifact: NativeArtifact::StaticLib,
            cargo_target: Some(triple.to_string()),
            emit_mode: NativeEmitMode::EmbeddedLib,
        }
    }

    /// Game Boy Advance ROM build (`tish build --target gba`): cross-compile to
    /// `thumbv4t-none-eabi` with the `Gba` emit mode.
    pub fn gba() -> Self {
        Self {
            artifact: NativeArtifact::GbaRom,
            cargo_target: Some("thumbv4t-none-eabi".to_string()),
            emit_mode: NativeEmitMode::Gba,
        }
    }

    pub fn is_cross_compile(&self) -> bool {
        self.cargo_target.is_some()
    }
}

/// Feature cap for GBA builds: no host-only runtime capabilities (http/fs/process/
/// timers/ws/tty/pty). The facade `compile_error!`-stubs them, but capping keeps
/// the emitted prelude from importing names that don't exist on GBA.
pub fn gba_runtime_features(_features: &[String]) -> Vec<String> {
    Vec::new()
}

/// Filter runtime features for iOS sandbox builds.
pub fn ios_runtime_features(features: &[String]) -> Vec<String> {
    const ALLOW: &[&str] = &["http", "http-hyper", "regex", "timers"];
    features
        .iter()
        .filter(|f| ALLOW.contains(&f.as_str()))
        .cloned()
        .collect()
}
