#!/usr/bin/env bash
# Build the example native extension and stage it as `mathext.lib` (an extension-neutral name
# that dlopen accepts on macOS/Linux/Windows alike, so the demo path is portable).
set -euo pipefail
cd "$(dirname "$0")"

# Stage a built cdylib at $2 (an extension-neutral name dlopen accepts everywhere).
stage() {
  local dir="$1" out="$2" lib=""
  for cand in "lib$out.dylib" "lib$out.so" "$out.dll"; do
    if [[ -f "$dir/target/release/$cand" ]]; then lib="$dir/target/release/$cand"; break; fi
  done
  if [[ -z "$lib" ]]; then
    echo "error: could not find the built cdylib under $dir/target/release/" >&2
    exit 1
  fi
  cp "$lib" "$out.lib"
  echo "Built $(basename "$lib") -> examples/ffi/$out.lib"
}

# mathext: LINKED model (links tishlang_ffi; built as a workspace member from the repo's target/).
( cd "$(git rev-parse --show-toplevel 2>/dev/null || echo ../..)" && cargo build --release -p mathext ) || \
  ( cargo build --release --manifest-path mathext/Cargo.toml )
root_target="$(git rev-parse --show-toplevel 2>/dev/null || echo ../..)/target/release"
for cand in libmathext.dylib libmathext.so mathext.dll; do
  [[ -f "$root_target/$cand" ]] && cp "$root_target/$cand" mathext.lib && echo "Built $cand -> examples/ffi/mathext.lib" && break
done

# statext: DECOUPLED model (links nothing tish-related; standalone, resolves accessors from the host).
( cd statext && cargo build --release )
stage statext statext

echo "Run:  tish run examples/ffi/demo.tish"
