# Tish build recipes
# Install just: cargo install just OR brew install just
#
# ═══════════════════════════════════════════════════════════════════════════════
# TWO WAYS TO EXECUTE TISH PROGRAMS
# ═══════════════════════════════════════════════════════════════════════════════
#
# 1. RUN (Interpreter) - Execute directly, no build step:
#      just run run hello.tish           # runs via interpreter
#      tish run hello.tish               # if tish is in PATH
#
# 2. BUILD (native binary) - Create standalone executable:
#      just run build hello.tish -o hello   # builds native binary
#      ./hello                                 # run the standalone binary
#
# ═══════════════════════════════════════════════════════════════════════════════
# FEATURE FLAGS (compile-time security)
# ═══════════════════════════════════════════════════════════════════════════════
#
#   - http    : Network access (fetch, fetchAll, serve)
#   - fs      : File system access (readFile, writeFile, fileExists, readDir, mkdir)
#   - process : Process control (process.exit, process.cwd, process.argv, process.env)
#   - regex   : Regular expression support (RegExp, String.match/replace/search/split)
#   - full    : All features enabled (http + fs + process + regex)
#
# Default: NO features enabled (secure mode)

# Default recipe - build secure (no dangerous features)
default: build-secure

# Build secure mode - no network, no fs, no process access
build-secure:
    cargo build --release --no-default-features

# Build with all features (for development/trusted environments)
build-full:
    cargo build --release --features full

# Build with specific features
build-http:
    cargo build --release --no-default-features --features http

build-fs:
    cargo build --release --no-default-features --features fs

build-process:
    cargo build --release --no-default-features --features process

build-regex:
    cargo build --release --no-default-features --features regex

# Build with multiple specific features
build-custom FEATURES:
    cargo build --release --no-default-features --features "{{FEATURES}}"

# Run in secure mode (no dangerous features)
run-secure *ARGS:
    cargo run --release --no-default-features -- {{ARGS}}

# Run with all features
run *ARGS:
    cargo run --release --features full -- {{ARGS}}

# Run with specific features
run-http *ARGS:
    cargo run --release --no-default-features --features http -- {{ARGS}}

run-fs *ARGS:
    cargo run --release --no-default-features --features fs -- {{ARGS}}

run-regex *ARGS:
    cargo run --release --no-default-features --features regex -- {{ARGS}}

# ═══════════════════════════════════════════════════════════════════════════════
# BUILD TISH PROGRAMS TO NATIVE BINARIES (just recipe name: compile → invokes `tish build`)
# ═══════════════════════════════════════════════════════════════════════════════

# Build a .tish file to native binary (with all features)
# Usage: just compile hello.tish hello
compile INPUT OUTPUT:
    cargo run --release --features full -- build {{INPUT}} -o {{OUTPUT}}

# Compile with secure mode (no dangerous features)
compile-secure INPUT OUTPUT:
    cargo run --release --no-default-features -- build {{INPUT}} -o {{OUTPUT}}

# Build compiler WASM (for playground, REPL, try-it). Output: tish_compiler.js, tish_compiler_bg.wasm
# Requires: rustup target add wasm32-unknown-unknown, cargo install wasm-bindgen-cli
build-compiler-wasm OUT_DIR:
    mkdir -p "{{OUT_DIR}}"
    cargo build -p tish_compiler_wasm --target wasm32-unknown-unknown --release
    wasm-bindgen target/wasm32-unknown-unknown/release/tish_compiler_wasm.wasm --out-dir "{{OUT_DIR}}" --out-name tish_compiler --target web

# Compile to WebAssembly (browser) - produces .wasm, .js, .html
# Requires: rustup target add wasm32-unknown-unknown, cargo install wasm-bindgen-cli
compile-wasm INPUT OUTPUT:
    cargo run --release -- build {{INPUT}} -o {{OUTPUT}} --target wasm

# Compile to WebAssembly (Wasmtime/WASI) - single .wasm, run with: wasmtime OUTPUT.wasm
# Requires: rustup target add wasm32-wasip1, wasmtime (curl -sSf https://wasmtime.dev/install.sh | bash)
compile-wasi INPUT OUTPUT:
    cargo run --release -- build {{INPUT}} -o {{OUTPUT}} --target wasi

# Compile with specific features
# Usage: just compile-with "http fs" hello.tish hello
compile-with FEATURES INPUT OUTPUT:
    cargo run --release --no-default-features --features "{{FEATURES}}" -- build {{INPUT}} -o {{OUTPUT}}

# ═══════════════════════════════════════════════════════════════════════════════
# TESTS
# ═══════════════════════════════════════════════════════════════════════════════
#
#   just test              # full workspace: unit tests (tish_compile, tish_opt, etc.) + tish integration
#   just test-tish         # tish package only (same as CI; no other crates' unit tests)
#   just test-quick        # tish only, skip slow backend tests (native/cranelift/wasi)
#   just test-coverage     # tish only + llvm-cov (writes lcov.info, coverage-html/)
#   just test-cargo        # plain cargo test (whole workspace, full features)
#   just test-secure       # cargo test, no features
#
# Regenerate static expected files: REGENERATE_EXPECTED=1 just test-tish test_mvp_programs_interpreter

# Run all tests in the workspace (unit tests in every crate + tish integration tests)
test *ARGS:
    cargo nextest run --workspace --features full -- {{ARGS}}

# Run only tish package tests (same as CI: integration tests only)
test-tish *ARGS:
    cargo nextest run -p tishlang--features full -- {{ARGS}}

# Skip slow backend tests (native/cranelift/wasi) for fast local iteration
test-quick:
    cargo nextest run -p tishlang--features full -- --skip test_mvp_programs_native --skip test_mvp_programs_cranelift --skip test_mvp_programs_wasi

# Run tests with coverage (requires llvm-tools: rustup component add llvm-tools-preview)
test-coverage:
    cargo llvm-cov nextest -p tishlang--features full --lcov --output-path lcov.info --html coverage-html

# Plain cargo test (whole workspace)
test-cargo:
    cargo test --features full

# Test secure mode
test-secure:
    cargo test --no-default-features

# Install tish CLI (secure mode - no dangerous features)
install *ARGS:
    cargo install --path crates/tish --no-default-features {{ARGS}}

# Install tish CLI with all features
install-full *ARGS:
    cargo install --path crates/tish --features full {{ARGS}}

# Check compilation for all feature combinations
check-all:
    @echo "Checking secure mode (no features)..."
    cargo check --no-default-features
    @echo "Checking http only..."
    cargo check --no-default-features --features http
    @echo "Checking fs only..."
    cargo check --no-default-features --features fs
    @echo "Checking process only..."
    cargo check --no-default-features --features process
    @echo "Checking regex only..."
    cargo check --no-default-features --features regex
    @echo "Checking full mode..."
    cargo check --features full
    @echo "All feature combinations compile successfully!"

# Refresh tish_jsx_web vendor from lattish (sibling package in repo)
refresh-lattish:
    cp ../lattish/src/Lattish.tish crates/tish_jsx_web/vendor/Lattish.tish
    @echo "Vendor Lattish.tish refreshed from lattish"

# Clean build artifacts
clean:
    cargo clean

# Format code
fmt:
    cargo fmt --all

# Lint
lint:
    cargo clippy --features full -- -D warnings

# ═══════════════════════════════════════════════════════════════════════════════
# TEST262 - JavaScript Conformance Tests
# ═══════════════════════════════════════════════════════════════════════════════

# Run all test262 tests
test262:
    ./scripts/run_test262.sh

# Run test262 tests with verbose output
test262-verbose:
    ./scripts/run_test262.sh --verbose

# Run filtered test262 tests (e.g., just test262-filter expressions)
test262-filter PATTERN:
    ./scripts/run_test262.sh --filter "{{PATTERN}}"

# Run test262 verbose with filter
test262-filter-verbose PATTERN:
    ./scripts/run_test262.sh --verbose --filter "{{PATTERN}}"

# ═══════════════════════════════════════════════════════════════════════════════
# PARITY COMPARE - Compare runtime outputs (interp vs vm, rust, cranelift, wasi, node)
# ═══════════════════════════════════════════════════════════════════════════════
#
# Run tests/core .tish (and .js) across runtimes; fail if any output differs from reference.
# Use to find VM/Cranelift/WASI/Node parity gaps (e.g. optional_chaining returns nothing on VM).
#
#   just parity                    # all runtimes, all tests
#   just parity optional_chaining  # single test
#   just parity "optional"         # tests matching name

parity filter="":
    #!/usr/bin/env bash
    if [[ -n "{{filter}}" ]]; then
      ./scripts/run_parity_compare.sh --filter "{{filter}}"
    else
      ./scripts/run_parity_compare.sh
    fi

parity-verbose filter="":
    #!/usr/bin/env bash
    if [[ -n "{{filter}}" ]]; then
      ./scripts/run_parity_compare.sh --filter "{{filter}}" --verbose
    else
      ./scripts/run_parity_compare.sh --verbose
    fi

parity-limit N:
    ./scripts/run_parity_compare.sh --limit {{N}}

# ═══════════════════════════════════════════════════════════════════════════════
# UTILITIES
# ═══════════════════════════════════════════════════════════════════════════════

# Profile array_stress sections (run each isolated test with timing to find slow parts)
array-stress-profile:
    ./scripts/run_array_stress_profile.sh

# Profile optional_chaining sections (find which ?? or ?. operation freezes)
optional-chaining-profile:
    ./scripts/run_optional_chaining_profile.sh

# Performance benchmark (vm vs interp vs cranelift vs wasi vs Node)
perf *ARGS:
    ./scripts/run_performance_manual.sh {{ARGS}}

# Bundled perf suite (single tests/main.tish — one native link per backend; CI uses this)
perf-suite *ARGS:
    ./scripts/run_performance_suite.sh {{ARGS}}

# Regenerate tests/main.tish + tests/main.js after changing paired pure perf tests
perf-suite-gen:
    python3 scripts/generate_perf_ci_main.py

# Show binary sizes for different builds
sizes:
    @echo "Building secure binary..."
    cargo build --release --no-default-features
    @ls -lh target/release/tish | awk '{print "Secure (no features):", $5}'
    @echo "Building full binary..."
    cargo build --release --features full
    @ls -lh target/release/tish | awk '{print "Full (all features):", $5}'
