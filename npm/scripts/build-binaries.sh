#!/usr/bin/env bash
# Build Tish binary for the current platform and copy into npm/tish/platform/<platform>/.
# Run from the tish repo root (parent of npm/).
# For release: run on each platform (or use CI) so npm/tish/platform/ has all binaries before publishing.

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"
PLATFORM_DIR="npm/tish/platform"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    TARGET=aarch64-apple-darwin
    PLATFORM=darwin-arm64
    BIN=tish
    ;;
  Darwin-x86_64)
    TARGET=x86_64-apple-darwin
    PLATFORM=darwin-x64
    BIN=tish
    ;;
  Linux-x86_64)
    TARGET=x86_64-unknown-linux-gnu
    PLATFORM=linux-x64
    BIN=tish
    ;;
  Linux-aarch64)
    TARGET=aarch64-unknown-linux-gnu
    PLATFORM=linux-arm64
    BIN=tish
    ;;
  MINGW*|MSYS*|CYGWIN*)
    TARGET=x86_64-pc-windows-msvc
    PLATFORM=win32-x64
    BIN=tish.exe
    ;;
  *)
    echo "Unsupported platform: $(uname -s)-$(uname -m)"
    echo "Supported: Darwin-arm64, Darwin-x86_64, Linux-x86_64, Linux-aarch64, Windows"
    exit 1
    ;;
esac

OUT="$PLATFORM_DIR/$PLATFORM/$BIN"
echo "Building tish for $TARGET..."
cargo build --release -p tishlang--target "$TARGET"
mkdir -p "$PLATFORM_DIR/$PLATFORM"
cp "target/$TARGET/release/$BIN" "$OUT"
echo "Copied to $OUT"
