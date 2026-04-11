//! CLI for pre-generating `cargo:` binding crates (bindgen-style).
//!
//! Resolves the dependency with `cargo metadata`, scans its `src/**/*.rs` with `syn`, classifies
//! each requested `pub fn` by signature, and emits a wrapper crate.

use std::path::PathBuf;

use clap::Parser;
use tishlang_cargo_bindgen::{
    generate_from_manifest, generate_from_registry_dependency, infer,
    resolve_runtime_path_for_output, runtime_path_relative_to_out_dir, BindgenConfig, TishlangRuntimeDep,
};

#[derive(Parser, Debug)]
#[command(name = "tishlang-cargo-bindgen")]
#[command(about = "Generate Rust glue for Tish `cargo:` imports from dependency source + metadata")]
struct Args {
    /// Upstream Cargo package to wrap (e.g. serde_json). Omit to read `tish.rustDependencies` + glue `Cargo.toml`.
    #[arg(long)]
    dependency: Option<String>,

    /// Semver for the upstream crate when using `--dependency` (ignored in full project auto mode).
    #[arg(long, default_value = "1.0")]
    dependency_version: String,

    /// Generated crate `[package].name` when using `--dependency` with `--out-dir` (default: tish_serde_json).
    #[arg(long)]
    crate_name: Option<String>,

    /// Output directory. Required with `--dependency` unless `--project-root` selects a path from `package.json`.
    #[arg(long)]
    out_dir: Option<PathBuf>,

    /// Comma-separated export names to bind (must match `pub fn` names in the dependency).
    #[arg(long, default_value = "to_string,from_str")]
    exports: String,

    /// If set, resolve `dependency` from this workspace instead of a temporary probe crate.
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Directory containing `package.json` (default: current directory). Drives automatic glue discovery.
    #[arg(long)]
    project_root: Option<PathBuf>,

    /// Crates.io semver for `tishlang_runtime` in the generated `Cargo.toml` (no path into the Tish repo).
    /// Mutually exclusive with `--tishlang-runtime-path`.
    #[arg(long, conflicts_with = "tishlang_runtime_path")]
    tishlang_runtime_version: Option<String>,

    /// Absolute path to the `tishlang_runtime` crate root for generated `path = ...` (overrides search).
    /// Mutually exclusive with `--tishlang-runtime-version`.
    #[arg(long, conflicts_with = "tishlang_runtime_version")]
    tishlang_runtime_path: Option<PathBuf>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("tishlang-cargo-bindgen: error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();

    let exports: Vec<String> = args
        .exports
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if exports.is_empty() {
        return Err("no exports given".into());
    }

    if args.dependency.is_none() && args.out_dir.is_some() {
        return Err(
            "--out-dir is only used with --dependency (without --dependency, output path comes from package.json tish.rustDependencies)"
                .into(),
        );
    }

    let current_dir = std::env::current_dir().map_err(|e| e.to_string())?;

    let (
        output_crate_name,
        out_dir,
        dependency_name,
        dependency_version_req,
        search_root,
    ) = if let Some(dep) = args.dependency.as_ref() {
        if let Some(out) = args.out_dir.as_ref() {
            let root = args
                .project_root
                .clone()
                .unwrap_or_else(|| out.parent().unwrap_or(out).to_path_buf());
            (
                args.crate_name
                    .clone()
                    .unwrap_or_else(|| "tish_serde_json".into()),
                out.clone(),
                dep.clone(),
                args.dependency_version.clone(),
                root,
            )
        } else {
            let root = args.project_root.clone().unwrap_or_else(|| current_dir.clone());
            let (crate_key, od) = infer::infer_glue_paths_only(&root, args.crate_name.as_deref())?;
            (
                crate_key,
                od,
                dep.clone(),
                args.dependency_version.clone(),
                root,
            )
        }
    } else {
        let root = args.project_root.clone().unwrap_or_else(|| current_dir.clone());
        let inf = infer::infer_from_project_root(&root, args.crate_name.as_deref())?;
        (
            inf.output_crate_name,
            inf.out_dir,
            inf.dependency_name,
            inf.dependency_version_req,
            root,
        )
    };

    let tishlang_runtime = if let Some(req) = args.tishlang_runtime_version.clone() {
        TishlangRuntimeDep::Version(req)
    } else if let Some(p) = args.tishlang_runtime_path.as_ref() {
        let abs = p
            .canonicalize()
            .map_err(|e| format!("tishlang_runtime path {}: {}", p.display(), e))?;
        TishlangRuntimeDep::Path(runtime_path_relative_to_out_dir(&out_dir, &abs)?)
    } else {
        let runtime_abs = resolve_runtime_path_for_output(&search_root)?;
        let rt_rel = runtime_path_relative_to_out_dir(&out_dir, &runtime_abs)?;
        TishlangRuntimeDep::Path(rt_rel)
    };

    let cfg = BindgenConfig {
        output_crate_name,
        tishlang_runtime,
        out_dir: out_dir.clone(),
        dependency_name,
        dependency_version_req,
        exports,
    };

    if let Some(manifest) = args.manifest_path {
        generate_from_manifest(&cfg, &manifest, &cfg.dependency_name)?;
    } else {
        generate_from_registry_dependency(&cfg)?;
    }

    println!(
        "Wrote {} (upstream `{}`, metadata + syn)",
        out_dir.display(),
        cfg.dependency_name
    );
    Ok(())
}
