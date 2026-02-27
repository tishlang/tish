# Tish build recipes with feature flag support
# Install just: cargo install just OR brew install just
#
# Feature Flags (compile-time security):
#   - http    : Network access (fetch, fetchAll, serve)
#   - fs      : File system access (readFile, writeFile, fileExists, readDir, mkdir)
#   - process : Process control (process.exit, process.cwd, process.argv, process.env)
#   - full    : All features enabled (http + fs + process)
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

# Test secure mode
test-secure:
    cargo test --no-default-features

# Test all features
test:
    cargo test --features full

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
    @echo "Checking full mode..."
    cargo check --features full
    @echo "All feature combinations compile successfully!"

# Clean build artifacts
clean:
    cargo clean

# Format code
fmt:
    cargo fmt --all

# Lint
lint:
    cargo clippy --features full -- -D warnings

# Show binary sizes for different builds
sizes:
    @echo "Building secure binary..."
    cargo build --release --no-default-features
    @ls -lh target/release/tish | awk '{print "Secure (no features):", $5}'
    @echo "Building full binary..."
    cargo build --release --features full
    @ls -lh target/release/tish | awk '{print "Full (all features):", $5}'
