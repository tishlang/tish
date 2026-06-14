#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Downstream regression suite — build & test tish CONSUMERS against THIS tish
# checkout (HEAD). Catches API/semantic breaks in the ecosystem (the kind that
# silently broke tish-pg / tish-callbacks when `NativeFn`→`Callable` and
# `Value::String`→`ArcStr` landed on feature/perf).
#
# For each repo it: (1) sources it (git clone or a local copy), (2) rewrites any
# `path = ".../crates/tish_*"` dependency to point at the tish-HEAD crates being
# tested, (3) runs the repo's build/test command, (4) compares the result to the
# expected status in the manifest. A repo marked `pass` that fails — or `xfail`
# that unexpectedly passes — fails the suite (the regression signal).
#
# Usage:
#   regression/downstream/run.sh [all | NAME ...] [options]
#     --tish DIR     tish checkout to test against (default: this repo root)
#     --workdir DIR  scratch dir for clones/copies (default: a tmp dir)
#     --git-only     only run repos with a git: source (CI default — local-only
#                    repos can't be cloned on a runner)
#     --keep         keep the workdir for inspection
#     --list         print the manifest and exit
# Env: TISH_DIR, WORKDIR override the same options.
# ---------------------------------------------------------------------------
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
MANIFEST="$SCRIPT_DIR/repos.tsv"

TISH="${TISH_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
WORKDIR="${WORKDIR:-}"
GIT_ONLY=0
KEEP=0
SELECT=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tish) TISH=$(cd "$2" && pwd); shift 2 ;;
    --workdir) WORKDIR="$2"; shift 2 ;;
    --git-only) GIT_ONLY=1; shift ;;
    --keep) KEEP=1; shift ;;
    --list) sed -n 's/^#.*//; /^[^[:space:]]/p' "$MANIFEST" 2>/dev/null; rg -v '^\s*#|^\s*$' "$MANIFEST"; exit 0 ;;
    all) shift ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) SELECT+=("$1"); shift ;;
  esac
done

[[ -f "$TISH/crates/tish_core/Cargo.toml" ]] || { echo "ERROR: --tish '$TISH' is not a tish checkout (no crates/tish_core)"; exit 2; }
[[ -f "$MANIFEST" ]] || { echo "ERROR: manifest not found: $MANIFEST"; exit 2; }
if [[ -z "$WORKDIR" ]]; then WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/tish-downstream.XXXXXX"); fi
mkdir -p "$WORKDIR"
[[ $KEEP -eq 1 ]] || trap 'rm -rf "$WORKDIR"' EXIT

echo "tish HEAD under test: $TISH ($(git -C "$TISH" rev-parse --short HEAD 2>/dev/null || echo '?'))"
echo "workdir:              $WORKDIR"
echo ""

# Rewrite every `path = "..../crates/tish_NAME"` in the repo's Cargo.tomls to the
# HEAD checkout — robust for path-dep consumers (no version-match issues).
rewrite_tish_paths() {
  local root="$1" t="$2"
  find "$root" -name Cargo.toml -not -path '*/target/*' -not -path '*/node_modules/*' 2>/dev/null \
    | while IFS= read -r f; do
        TISH_ABS="$t" perl -i -pe 's{path\s*=\s*"[^"]*?/crates/(tish_[A-Za-z0-9_]+)"}{path = "$ENV{TISH_ABS}/crates/$1"}g' "$f" 2>/dev/null || true
      done
}

PASS=(); FAIL=(); XFAIL=(); UNXPASS=(); SKIP=()

run_one() {
  local name="$1" source="$2" subdir="$3" kind="$4" cmd="$5" expected="$6"
  local dir="$WORKDIR/$name"

  # 1. source
  case "$source" in
    git:*)
      # NOTE: split across statements — a single `local a=.. b=$a` reads the OUTER `a`, not the one
      # being declared, so url/ref would come out empty.
      local spec="${source#git:}"
      local url="${spec%@*}" ref="${spec##*@}"
      [[ "$ref" == "$url" ]] && ref=""
      echo "── $name ── cloning $url${ref:+ @$ref}"
      # Build args as an array — `${ref:+--branch "$ref"}` inline mis-passes `--branch main` as one arg.
      local clone_args=(clone --depth 1)
      [[ -n "$ref" ]] && clone_args+=(--branch "$ref")
      clone_args+=("$url" "$dir")
      if ! git "${clone_args[@]}" >/dev/null 2>&1; then
        echo "   SKIP (clone failed — private/unreachable/bad-ref)"; SKIP+=("$name"); return
      fi ;;
    self:*)
      # a subdir of the tish checkout under test (e.g. the in-repo ffi examples) — always available.
      local src="$TISH/${source#self:}"
      [[ -d "$src" ]] || { echo "── $name ── SKIP (self path missing: $src)"; SKIP+=("$name"); return; }
      echo "── $name ── copying $src (in-repo)"
      rsync -a --exclude target --exclude node_modules "$src/" "$dir/" >/dev/null 2>&1 ;;
    local:*)
      local src="${source#local:}"; src="${src/#\~/$HOME}"
      if [[ $GIT_ONLY -eq 1 ]]; then echo "── $name ── SKIP (local-only, --git-only)"; SKIP+=("$name"); return; fi
      [[ -d "$src" ]] || { echo "── $name ── SKIP (local path missing: $src)"; SKIP+=("$name"); return; }
      echo "── $name ── copying $src"
      rsync -a --exclude target --exclude node_modules --exclude .git "$src/" "$dir/" >/dev/null 2>&1 ;;
    *) echo "── $name ── SKIP (bad source: $source)"; SKIP+=("$name"); return ;;
  esac

  # 2. wire to tish HEAD
  rewrite_tish_paths "$dir" "$TISH"
  if [[ "$kind" == "tish" ]]; then
    ( cd "$TISH" && cargo build --release -p tishlang >/dev/null 2>&1 ) || true
    export PATH="$TISH/target/release:$PATH"
    export TISH_BIN="$TISH/target/release/tish" TISH_BINARY="$TISH/target/release/tish"
    # tish-program harnesses invoke the PINNED npm @tishlang/tish (npx / scripts/tish.mjs), so PATH
    # alone won't exercise HEAD. Install deps, then overwrite the bundled native binary with HEAD so
    # the repo's own `npm test`/`npm run build` actually runs against this tish.
    if [[ -f "$dir/$subdir/package.json" && "$cmd" == *npm* ]]; then
      ( cd "$dir/$subdir" && npm install --silent --no-audit --no-fund --no-progress >/dev/null 2>&1 ) || true
      find "$dir/$subdir/node_modules" -path '*@tishlang*' -name 'tish' -type f 2>/dev/null | while IFS= read -r bin; do
        if file "$bin" 2>/dev/null | rg -qi 'mach-o|elf|executable'; then cp -f "$TISH/target/release/tish" "$bin" 2>/dev/null || true; fi
      done
    fi
    # Wire any lattish dependency to the LOCAL WORKSPACE lattish — the npm analog of rewrite_tish_paths
    # for the Rust crates. We test consumers against the lattish being DEVELOPED next to this tish
    # checkout, not the published package (else we'd only re-test already-released code). Consumers pin
    # lattish to a local monorepo path (file:../tish/lattish) absent in an isolated clone, so npm leaves a
    # dangling symlink; replace it with the workspace copy. Wire BOTH the bare `lattish` dir (older
    # consumers, e.g. tish-audio) and the scoped `@tishlang/lattish` dir — the resolver matches a scoped
    # import (`from "@tishlang/lattish"`, e.g. tish-midi/deckard) ONLY at node_modules/@tishlang/lattish,
    # so the bare dir alone leaves it unresolved. Falls back to the cloned lattish in CI.
    if [[ -f "$dir/$subdir/package.json" ]] && rg -q '"(@tishlang/)?lattish"[[:space:]]*:' "$dir/$subdir/package.json" 2>/dev/null; then
      local lat_src=""
      if [[ -d "$TISH/../lattish" ]]; then lat_src="$(cd "$TISH/.." && pwd)/lattish"
      elif [[ -d "$WORKDIR/lattish" ]]; then lat_src="$WORKDIR/lattish"; fi
      if [[ -n "$lat_src" ]]; then
        rm -rf "$dir/$subdir/node_modules/lattish" "$dir/$subdir/node_modules/@tishlang/lattish"
        mkdir -p "$dir/$subdir/node_modules/@tishlang"
        rsync -a --exclude node_modules --exclude .git "$lat_src/" "$dir/$subdir/node_modules/lattish/" >/dev/null 2>&1
        rsync -a --exclude node_modules --exclude .git "$lat_src/" "$dir/$subdir/node_modules/@tishlang/lattish/" >/dev/null 2>&1
        echo "   wired LOCAL lattish ($lat_src) -> node_modules/{lattish,@tishlang/lattish}"
      else
        echo "   ⚠ depends on lattish but no local workspace ($TISH/../lattish) or clone ($WORKDIR/lattish) found"
      fi
    fi
  fi

  # 3. build + test
  echo "   running: $cmd  (in $subdir, expected=$expected)"
  local log="$WORKDIR/$name.log" rc
  ( cd "$dir/$subdir" && eval "$cmd" ) >"$log" 2>&1; rc=$?

  # 4. classify vs expected
  if [[ $rc -eq 0 ]]; then
    if [[ "$expected" == "xfail" ]]; then echo "   ⚠ UNEXPECTED PASS (manifest says xfail — flip to pass)"; UNXPASS+=("$name");
    else echo "   ✓ PASS"; PASS+=("$name"); fi
  else
    if [[ "$expected" == "xfail" ]]; then echo "   ✓ xfail (known-broken, as expected)"; XFAIL+=("$name");
    else echo "   ✗ FAIL (regression!) — last error:"; rg "^error|error\[|could not compile|FAILED|panicked" "$log" | head -3 | sed 's/^/      /'; FAIL+=("$name"); fi
  fi
}

# read manifest: name<TAB>source<TAB>subdir<TAB>kind<TAB>cmd<TAB>expected
while IFS=$'\t' read -r name source subdir kind cmd expected; do
  [[ -z "${name:-}" || "$name" =~ ^[[:space:]]*# ]] && continue
  if [[ ${#SELECT[@]} -gt 0 ]]; then
    local_match=0; for s in "${SELECT[@]}"; do [[ "$s" == "$name" ]] && local_match=1; done
    [[ $local_match -eq 1 ]] || continue
  fi
  run_one "$name" "$source" "${subdir:-.}" "$kind" "$cmd" "$expected"
done < <(rg -v '^\s*#|^\s*$' "$MANIFEST")

echo ""
echo "═════════════════════ DOWNSTREAM REGRESSION SUMMARY ═════════════════════"
printf "  PASS:           %s\n" "${PASS[*]:-(none)}"
printf "  xfail (known):  %s\n" "${XFAIL[*]:-(none)}"
printf "  SKIP:           %s\n" "${SKIP[*]:-(none)}"
[[ ${#UNXPASS[@]} -gt 0 ]] && printf "  ⚠ UNEXPECTED PASS (flip manifest to pass): %s\n" "${UNXPASS[*]}"
[[ ${#FAIL[@]} -gt 0 ]]    && printf "  ✗ REGRESSIONS (was expected to pass): %s\n" "${FAIL[*]}"
echo "═════════════════════════════════════════════════════════════════════════"

# Fail the suite on a real regression (expected-pass repo that failed). Unexpected
# passes are a warning (non-fatal) — they mean a known-broken repo got fixed.
[[ ${#FAIL[@]} -eq 0 ]] || exit 1
exit 0
