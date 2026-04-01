fn main() {
    let version = read_mlx_rs_version().unwrap_or_else(|| "0.25".to_string());
    println!("cargo:rustc-env=TISH_MLX_RS_VERSION={}", version);
    println!("cargo:rerun-if-changed=../../Cargo.lock");
}

fn read_mlx_rs_version() -> Option<String> {
    let lock = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Cargo.lock"),
    ).ok()?;
    let mut in_mlx = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == r#"name = "mlx-rs""# { in_mlx = true; continue; }
        if in_mlx {
            if let Some(v) = line.strip_prefix("version = \"").and_then(|s| s.strip_suffix('"')) {
                return Some(v.to_string());
            }
            if line.starts_with("name =") { in_mlx = false; }
        }
    }
    None
}
