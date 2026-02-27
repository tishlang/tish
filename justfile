# Tish build recipes with feature flag support
# Install just: cargo install just OR brew install just

# Default recipe - build with all features
default: build

# Build with HTTP support (default)
build:
    cargo build --release

# Build without HTTP (minimal binary)
build-minimal:
    cargo build --release --no-default-features

# Build with explicit HTTP feature
build-http:
    cargo build --release --features http

# Run interpreter with HTTP
run *ARGS:
    cargo run --release -- {{ARGS}}

# Run interpreter without HTTP
run-minimal *ARGS:
    cargo run --release --no-default-features -- {{ARGS}}

# Test all features
test:
    cargo test --all-features

# Test without HTTP
test-minimal:
    cargo test --no-default-features

# Check compilation for all feature combinations
check-all:
    cargo check --no-default-features
    cargo check --features http
    cargo check --all-features

# Clean build artifacts
clean:
    cargo clean

# Format code
fmt:
    cargo fmt --all

# Lint
lint:
    cargo clippy --all-features -- -D warnings
