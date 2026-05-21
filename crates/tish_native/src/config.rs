//! Native build configuration (desktop binary vs iOS staticlib, cross-target).

use tishlang_compile::NativeEmitMode;

/// Output artifact kind for `tish build --target native`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeArtifact {
    #[default]
    Bin,
    StaticLib,
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

    pub fn is_cross_compile(&self) -> bool {
        self.cargo_target.is_some()
    }
}

/// Filter runtime features for iOS sandbox builds.
pub fn ios_runtime_features(features: &[String]) -> Vec<String> {
    const ALLOW: &[&str] = &["http", "http-hyper", "regex"];
    features
        .iter()
        .filter(|f| ALLOW.contains(&f.as_str()))
        .cloned()
        .collect()
}
