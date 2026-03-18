//! Standalone formatter — not part of the `tish` compiler CLI.

use std::fs;

use clap::Parser;

#[derive(Parser)]
#[command(name = "tish-fmt")]
#[command(about = "Format Tish source (pretty-print via AST)")]
struct Cli {
    #[arg(required = true)]
    file: String,
    #[arg(long)]
    check: bool,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli.file, cli.check) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn run(path: &str, check: bool) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    let formatted = tish_fmt::format_source(&source)?;
    if check {
        if formatted != source {
            return Err(format!(
                "Format check failed: {} needs formatting (run `tish-fmt {}`)",
                path, path
            ));
        }
        println!("{}: OK", path);
        return Ok(());
    }
    fs::write(path, formatted).map_err(|e| format!("Cannot write {}: {}", path, e))?;
    println!("Formatted: {}", path);
    Ok(())
}
