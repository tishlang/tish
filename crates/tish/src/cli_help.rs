//! Long help text, terminal styling, and ASCII banner for the `tish` CLI.

use std::io::{self, IsTerminal, Write};
use std::thread;
use std::time::Duration;

use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, Subcommand};

/// FIGlet-style block letters (UTF-8). On a TTY, a short expand + palette-color animation runs.
const TISH_BANNER_LINES: &[&str] = &[
    "████████╗██╗███████╗██╗  ██╗",
    "╚══██╔══╝██║██╔════╝██║  ██║",
    "   ██║   ██║███████╗███████║",
    "   ██║   ██║╚════██║██╔══██║",
    "   ██║   ██║███████║██║  ██║",
    "   ╚═╝   ╚═╝╚══════╝╚═╝  ╚═╝",
];

/// Frames used for the left-to-right expand reveal.
const BANNER_REVEAL_FRAMES: usize = 14;
/// Extra frames of rainbow cycling after the logo is fully visible.
const BANNER_CYCLE_FRAMES: usize = 36;
const BANNER_FRAME_MS: u64 = 28;

/// Orange → Yellow → Green → Teal → Blue → Purple → Pink (matching the brand palette).
const PALETTE: &[(u8, u8, u8)] = &[
    (255, 159,  64),  // Orange
    (255, 213,  64),  // Yellow
    ( 52, 199,  89),  // Green
    ( 48, 209, 188),  // Teal
    ( 10, 132, 255),  // Blue
    (175,  82, 222),  // Purple
    (255,  55, 148),  // Pink
];

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Linearly interpolate between two palette colors.
fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t).round() as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t).round() as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t).round() as u8,
    )
}

/// Smooth palette sample for a given (row, col) cell and scrolling color frame.
/// Uses column as the primary gradient axis so every row has a continuous sweep.
/// A small per-row offset adds a gentle diagonal tilt rather than flat stripes.
fn palette_color(row: usize, col: usize, color_frame: usize) -> (u8, u8, u8) {
    let n = PALETTE.len();
    // one full palette cycle every ~5 columns; row adds a slight diagonal
    let scroll = color_frame as f32 * 0.22;
    let pos = ((col as f32 / 5.0) + (row as f32 * 0.25) + scroll).rem_euclid(n as f32);
    let lo = pos.floor() as usize % n;
    let hi = (lo + 1) % n;
    lerp_color(PALETTE[lo], PALETTE[hi], pos.fract())
}

/// Render one frame. `reveal_t` is 0..=1 (how much of each line is visible).
/// `color_frame` is the ever-incrementing counter that drives the rainbow scroll.
fn write_tish_banner_frame(out: &mut impl Write, reveal_t: f32, color_frame: usize) {
    for (row, line) in TISH_BANNER_LINES.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let visible = ((len as f32) * reveal_t).round() as usize;
        let visible = visible.min(len);

        for col in 0..len {
            let ch = chars[col];
            if col >= visible || ch == ' ' {
                let _ = write!(out, " ");
            } else {
                let (r, g, b) = palette_color(row, col, color_frame);
                let _ = write!(out, "\x1b[1;38;2;{r};{g};{b}m{ch}\x1b[0m");
            }
        }
        let _ = writeln!(out);
    }
}

fn print_tish_banner_plain(out: &mut impl Write) {
    for line in TISH_BANNER_LINES {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out);
}

fn print_tish_banner_animated(out: &mut impl Write) {
    let n = TISH_BANNER_LINES.len();
    let total = BANNER_REVEAL_FRAMES + BANNER_CYCLE_FRAMES;

    for f in 0..total {
        if f > 0 {
            let _ = write!(out, "\x1b[{n}A");
        }
        // Phase 1: ease-out expand.  Phase 2: fully visible, rainbow keeps scrolling.
        let reveal_t = if f < BANNER_REVEAL_FRAMES {
            ease_out_cubic((f + 1) as f32 / BANNER_REVEAL_FRAMES as f32)
        } else {
            1.0
        };
        write_tish_banner_frame(out, reveal_t, f);
        let _ = out.flush();
        thread::sleep(Duration::from_millis(BANNER_FRAME_MS));
    }
    let _ = writeln!(out);
}

/// Print the `TISH` tile banner to stdout (animated palette on a TTY; plain text otherwise).
pub fn print_tish_banner() {
    let mut out = io::stdout().lock();
    if io::stdout().is_terminal() {
        print_tish_banner_animated(&mut out);
    } else {
        print_tish_banner_plain(&mut out);
    }
}

/// Whether argv will cause clap to print help (top-level or subcommand).
pub fn argv_requests_help(argv: &[String]) -> bool {
    argv.iter().any(|a| a == "--help" || a == "-h")
        || matches!(argv.get(1).map(String::as_str), Some("help"))
}

/// Help colors aligned with `cargo` (green section labels, cyan flags / placeholders).
pub fn cargo_help_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Cyan.on_default() | Effects::BOLD)
}

pub const CLI_AFTER_HELP: &str = r#"Environment variables:
  TISH_NO_OPTIMIZE=1
          Disable AST and bytecode optimizations for run/build

Run / REPL backends (--backend):
  vm
          Bytecode VM (default)
  interp
          Tree-walking interpreter

Capabilities (--feature, repeatable; comma-separated values are split):
  http
          Network: fetch, serve, Promise / timers (native async)
  fs
          Filesystem: readFile, writeFile, fileExists, isDir, readDir, mkdir
  process
          process.exit, cwd, exec, argv, env
  regex
          RegExp
  ws
          WebSocket client / server
  full
          All of the above (http, fs, process, regex, ws)

Omit --feature on run/repl (VM) or native build to use every capability linked into this binary.
Build `tish` with matching Cargo features (e.g. cargo build -p tishlang --features full).

For --target (native, js, wasm, wasi) and --native-backend (rust, cranelift, llvm), see:
  tish build --help"#;

pub const BUILD_COMMAND_AFTER_LONG_HELP: &str = r#"Build targets (--target, default: native):
  native
          Native executable (see --native-backend)
  js
          JavaScript bundle
  wasm
          WebAssembly (.tish project; .js source supported on some paths)
  wasi
          WASI WebAssembly

Native backends (--native-backend, only with --target native, default: rust):
  rust
          Emit Rust + link tishlang_runtime via cargo
  cranelift
          Embedded bytecode + Cranelift/VM runtime binary
  llvm
          Embedded bytecode + LLVM/clang link path

Capabilities (--feature, repeatable; comma-separated values are split):
  http
          Network: fetch, serve, Promise / timers (native async)
  fs
          Filesystem: readFile, writeFile, fileExists, isDir, readDir, mkdir
  process
          process.exit, cwd, exec, argv, env
  regex
          RegExp
  ws
          WebSocket client / server
  full
          All of the above (http, fs, process, regex, ws)

Omit --feature on native build to use every capability linked into this binary.
Build `tish` with matching Cargo features (e.g. cargo build -p tishlang --features full)."#;

#[derive(Parser)]
#[command(name = "tish")]
#[command(about = "Tish - minimal TS/JS-compatible language")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(styles = cargo_help_styles())]
#[command(after_help = CLI_AFTER_HELP)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Parser)]
pub(crate) struct RunArgs {
    /// Path to a `.tish` file, or `-` to read the program from stdin (like `node -`).
    #[arg(required = true, allow_hyphen_values = true, value_name = "FILE", help_heading = "Arguments")]
    pub file: String,
    /// `vm` or `interp` (see `tish --help` for capabilities / `--feature`).
    #[arg(long, default_value = "vm", value_name = "NAME", help_heading = "Options")]
    pub backend: String,
    /// Subset of capabilities (see `tish --help` for the full list).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    /// Disable AST and bytecode optimizations (for debugging).
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
}

#[derive(Parser)]
pub(crate) struct ReplArgs {
    /// `vm` or `interp` (see `tish --help`).
    #[arg(long, default_value = "vm", value_name = "NAME", help_heading = "Options")]
    pub backend: String,
    /// Subset of capabilities (see `tish --help` for the full list).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
}

#[derive(Parser)]
pub(crate) struct BuildArgs {
    #[arg(
        short,
        long,
        default_value = "tish_out",
        value_name = "PATH",
        help_heading = "Options"
    )]
    pub output: String,
    /// `native`, `js`, `wasm`, or `wasi` (see long help below).
    #[arg(long, default_value = "native", value_name = "TARGET", help_heading = "Options")]
    pub target: String,
    /// `rust`, `cranelift`, or `llvm` (only for `--target native`).
    #[arg(long, default_value = "rust", value_name = "BACKEND", help_heading = "Options")]
    pub native_backend: String,
    /// Capability subset for native output (see long help below).
    #[arg(
        long = "feature",
        value_name = "NAME",
        action = clap::ArgAction::Append,
        help_heading = "Options"
    )]
    pub features: Vec<String>,
    #[arg(long, help_heading = "Options")]
    pub no_optimize: bool,
    /// Entry `.tish` file (or `.js` where supported).
    #[arg(required = true, value_name = "FILE", help_heading = "Arguments")]
    pub file: String,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run a Tish file (interpret)
    Run(RunArgs),
    /// Interactive REPL
    Repl(ReplArgs),
    /// Build native binary, wasm, wasi, or JavaScript output
    #[command(after_long_help = BUILD_COMMAND_AFTER_LONG_HELP)]
    Build(BuildArgs),
    /// Parse and dump AST
    #[command(name = "dump-ast")]
    DumpAst {
        #[arg(required = true, value_name = "FILE", help_heading = "Arguments")]
        file: String,
    },
}
