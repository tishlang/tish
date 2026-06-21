//! Dump perf fixtures to target/dump_*.rs for inspection.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    for k in [
        "TISH_PARAM_NATIVE",
        "TISH_PARAM_INFER",
        "TISH_NATIVE_FN",
        "TISH_STRUCT_INFER",
        "TISH_FUSED_HOF",
        "TISH_NATIVE_HOF",
        "TISH_AGGREGATE_INFER",
    ] {
        std::env::set_var(k, "1");
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn dump_perf_fixtures() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::write(out.join("dump_mandelbrot.rs"), compile("tests/perf/mandelbrot.tish")).unwrap();
    std::fs::write(out.join("dump_fasta.rs"), compile("tests/perf/fasta.tish")).unwrap();
    std::fs::write(out.join("dump_fannkuch.rs"), compile("tests/perf/fannkuch.tish")).unwrap();
    std::fs::write(out.join("dump_fnv_hash.rs"), compile("tests/perf/fnv_hash.tish")).unwrap();
    std::fs::write(out.join("dump_spectral_norm.rs"), compile("tests/perf/spectral_norm.tish")).unwrap();
    std::fs::write(out.join("dump_queens.rs"), compile("tests/perf/queens.tish")).unwrap();
}
