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
echo "Building tish for $TARGET (features: full — npm CLI must match VM capabilities: http, fs, process, regex, ws)..."
cargo build --release -p tishlang --target "$TARGET" --features full
mkdir -p "$PLATFORM_DIR/$PLATFORM"
TRIPLE_BIN="target/$TARGET/release/$BIN"
HOST_BIN="target/release/$BIN"
if [[ -f "$TRIPLE_BIN" ]]; then
  cp "$TRIPLE_BIN" "$OUT"
elif [[ -f "$HOST_BIN" ]]; then
  cp "$HOST_BIN" "$OUT"
else
  echo "error: expected $TRIPLE_BIN or $HOST_BIN after build" >&2
  exit 1
fi
echo "Copied to $OUT"

BINDGEN_PLATFORM_DIR="npm/cargo-bindgen/platform"
BINDGEN_BIN="tish-bindgen"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  BINDGEN_BIN="tish-bindgen.exe"
fi
BINDGEN_SRC="target/$TARGET/release/tishlang-cargo-bindgen"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  BINDGEN_SRC="target/$TARGET/release/tishlang-cargo-bindgen.exe"
fi
BINDGEN_HOST_SRC="target/release/tishlang-cargo-bindgen"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  BINDGEN_HOST_SRC="target/release/tishlang-cargo-bindgen.exe"
fi
BINDGEN_OUT="$BINDGEN_PLATFORM_DIR/$PLATFORM/$BINDGEN_BIN"

echo "Building tishlang_cargo_bindgen for $TARGET..."
cargo build --release -p tishlang_cargo_bindgen --target "$TARGET"
mkdir -p "$BINDGEN_PLATFORM_DIR/$PLATFORM"
if [[ -f "$BINDGEN_SRC" ]]; then
  cp "$BINDGEN_SRC" "$BINDGEN_OUT"
elif [[ -f "$BINDGEN_HOST_SRC" ]]; then
  cp "$BINDGEN_HOST_SRC" "$BINDGEN_OUT"
else
  echo "error: expected $BINDGEN_SRC or $BINDGEN_HOST_SRC after bindgen build" >&2
  exit 1
fi
echo "Copied bindgen to $BINDGEN_OUT"

FMT_PLATFORM_DIR="npm/tish-format/platform"
FMT_BIN="tish-fmt"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  FMT_BIN="tish-fmt.exe"
fi
FMT_SRC="target/$TARGET/release/tish-fmt"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  FMT_SRC="target/$TARGET/release/tish-fmt.exe"
fi
FMT_HOST_SRC="target/release/tish-fmt"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  FMT_HOST_SRC="target/release/tish-fmt.exe"
fi
FMT_OUT="$FMT_PLATFORM_DIR/$PLATFORM/$FMT_BIN"

echo "Building tishlang_fmt for $TARGET..."
cargo build --release -p tishlang_fmt --target "$TARGET"
mkdir -p "$FMT_PLATFORM_DIR/$PLATFORM"
if [[ -f "$FMT_SRC" ]]; then
  cp "$FMT_SRC" "$FMT_OUT"
elif [[ -f "$FMT_HOST_SRC" ]]; then
  cp "$FMT_HOST_SRC" "$FMT_OUT"
else
  echo "error: expected $FMT_SRC or $FMT_HOST_SRC after tishlang_fmt build" >&2
  exit 1
fi
echo "Copied tish_fmt to $FMT_OUT"

LINT_PLATFORM_DIR="npm/tish-lint/platform"
LINT_BIN="tish-lint"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  LINT_BIN="tish-lint.exe"
fi
LINT_SRC="target/$TARGET/release/tish-lint"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  LINT_SRC="target/$TARGET/release/tish-lint.exe"
fi
LINT_HOST_SRC="target/release/tish-lint"
if [[ "$PLATFORM" == "win32-x64" ]]; then
  LINT_HOST_SRC="target/release/tish-lint.exe"
fi
LINT_OUT="$LINT_PLATFORM_DIR/$PLATFORM/$LINT_BIN"

echo "Building tishlang_lint for $TARGET..."
cargo build --release -p tishlang_lint --target "$TARGET"
mkdir -p "$LINT_PLATFORM_DIR/$PLATFORM"
if [[ -f "$LINT_SRC" ]]; then
  cp "$LINT_SRC" "$LINT_OUT"
elif [[ -f "$LINT_HOST_SRC" ]]; then
  cp "$LINT_HOST_SRC" "$LINT_OUT"
else
  echo "error: expected $LINT_SRC or $LINT_HOST_SRC after tishlang_lint build" >&2
  exit 1
fi
echo "Copied tish_lint to $LINT_OUT"
