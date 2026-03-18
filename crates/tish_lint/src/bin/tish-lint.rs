//! Standalone linter — not part of the `tish` compiler CLI.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

#[derive(Parser)]
#[command(name = "tish-lint")]
#[command(about = "AST-based linter for Tish")]
struct Cli {
    #[arg(required = true)]
    paths: Vec<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli.paths) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn run(paths: &[String]) -> Result<(), String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            for e in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if e.path().extension().map(|x| x == "tish").unwrap_or(false) {
                    files.push(e.path().to_path_buf());
                }
            }
        } else if path.exists() {
            files.push(path.to_path_buf());
        } else {
            return Err(format!("Not found: {}", p));
        }
    }
    if files.is_empty() {
        return Err("No .tish files to lint".into());
    }
    let mut errors = 0;
    for f in files {
        let src = fs::read_to_string(&f).map_err(|e| format!("{}: {}", f.display(), e))?;
        match tish_lint::lint_source(&src) {
            Ok(diags) => {
                for d in diags {
                    let sev = match d.severity {
                        tish_lint::Severity::Error => {
                            errors += 1;
                            "error"
                        }
                        tish_lint::Severity::Warning => "warning",
                    };
                    println!(
                        "{}:{}:{}: {} [{}] {}",
                        f.display(),
                        d.line,
                        d.col,
                        sev,
                        d.code,
                        d.message
                    );
                }
            }
            Err(e) => {
                eprintln!("{}: parse error: {}", f.display(), e);
                errors += 1;
            }
        }
    }
    if errors > 0 {
        return Err(format!("{} issue(s)", errors));
    }
    Ok(())
}
