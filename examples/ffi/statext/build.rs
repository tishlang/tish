//! Let the cdylib's undefined `tish_value_*` symbols resolve against the host at dlopen time.
//! macOS uses a two-level namespace, so a plugin that calls back into the loader needs
//! `-undefined dynamic_lookup`. Linux/BSD resolve flat against the (-rdynamic) host automatically.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "macos" | "ios") {
        println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    }
}
