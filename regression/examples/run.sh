#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Example-app regression suite — RUN (not just build) every in-repo example
# against THIS tish checkout (HEAD), and assert observable behavior.
#
# Companion to regression/downstream (external consumers). Where a transpile
# smoke only proves "it parses", this suite proves "it WORKS": run-and-exit
# programs are executed and their stdout asserted; HTTP servers are started,
# probed, and shut down; lattish apps are built to JS, mounted in jsdom, and
# their rendered DOM asserted.
#
# Why this exists: the downstream suite caught a JSX-lexer regression
# (tishlang/tish#108, fixed in #111) where a reserved keyword as bare text after
# a nested child element failed to parse. A keyword like that hiding in example
# markup would have shipped silently. Running the examples is the standing guard
# against the next one.
#
# Usage:
#   regression/examples/run.sh [all | NAME ...] [options]
#     --tish DIR   tish checkout to test against (default: this repo root)
#     --full       also run env-gated rows (net/db/gpu/wasm/npm/prebuild/slow)
#     --keep       keep the scratch workdir
#     --list       print the manifest and exit
# Env: TISH_DIR overrides --tish.
# ---------------------------------------------------------------------------
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
MANIFEST="$SCRIPT_DIR/examples.tsv"
RENDER_HARNESS="$SCRIPT_DIR/lattish-render.mjs"

TISH="${TISH_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
LATTISH_WS=""        # resolved after TISH is known
FULL=0
KEEP=0
SELECT=()
# tags that require an environment we don't assume in a default/CI run
SKIP_TAGS="net db gpu wasm npm prebuild native slow vite"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tish) TISH=$(cd "$2" && pwd); shift 2 ;;
    --full) FULL=1; shift ;;
    --keep) KEEP=1; shift ;;
    --list) grep -vE '^\s*#|^\s*$' "$MANIFEST"; exit 0 ;;
    all) shift ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) SELECT+=("$1"); shift ;;
  esac
done

[[ -f "$TISH/crates/tish_core/Cargo.toml" ]] || { echo "ERROR: --tish '$TISH' is not a tish checkout"; exit 2; }
[[ -f "$MANIFEST" ]] || { echo "ERROR: manifest not found: $MANIFEST"; exit 2; }
LATTISH_WS="$(cd "$TISH/.." && pwd)/lattish"

WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/tish-examples.XXXXXX")
mkdir -p "$WORKDIR"
[[ $KEEP -eq 1 ]] || trap 'rm -rf "$WORKDIR"' EXIT

echo "tish HEAD under test: $TISH ($(git -C "$TISH" rev-parse --short HEAD 2>/dev/null || echo '?'))"
echo "workdir:              $WORKDIR"

# 1. build the HEAD tish binary once, put it on PATH
echo "building HEAD tish binary…"
( cd "$TISH" && cargo build --release -p tishlang >/dev/null 2>&1 ) \
  || { echo "ERROR: failed to build tish (cargo build --release -p tishlang)"; exit 2; }
export PATH="$TISH/target/release:$PATH"
# the lattish render harness resolves jsdom (CJS) from this package's node_modules
export LATTISH_PKG="$LATTISH_WS"
# deterministic env for the server examples that read env vars
export TEST="tishreg" DEPLOYMENT_ID="reg1" TISH_HTTP_WORKERS=1

HAVE_JSDOM=0; [[ -d "$LATTISH_WS/node_modules/jsdom" ]] && HAVE_JSDOM=1

# a TCP port with a LISTEN socket?
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
# first free port in a high range (for PORT-respecting servers)
free_port() { local p; for p in $(seq 8123 8999); do port_busy "$p" || { echo "$p"; return; }; done; echo 0; }
# kill only OUR tish listeners still holding a port (workers the parent didn't reap) —
# never touches a foreign process on that port.
free_tish_port() {
  local lp; for lp in $(lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null); do
    [[ "$(ps -p "$lp" -o comm= 2>/dev/null)" == *tish* ]] && kill "$lp" 2>/dev/null
  done
}

echo ""

PASS=(); FAIL=(); XFAIL=(); UNXPASS=(); SKIP=()

# resolve a manifest `dir` to an absolute source path (handles the @lattish: prefix)
resolve_dir() {
  case "$1" in
    @lattish:*) echo "$LATTISH_WS/${1#@lattish:}" ;;
    *) echo "$TISH/$1" ;;
  esac
}

# is any of the row's tags in the skip-set?
row_is_env_gated() {
  local tags="$1" t s
  [[ "$tags" == "-" || -z "$tags" ]] && return 1
  IFS=',' read -ra _ts <<< "$tags"
  for t in "${_ts[@]}"; do
    for s in $SKIP_TAGS; do [[ "$t" == "$s" ]] && return 0; done
  done
  return 1
}

classify() { # rc expected name
  local rc="$1" expected="$2" name="$3"
  if [[ $rc -eq 0 ]]; then
    if [[ "$expected" == "xfail" ]]; then echo "   ⚠ UNEXPECTED PASS (flip to pass)"; UNXPASS+=("$name")
    else echo "   ✓ PASS"; PASS+=("$name"); fi
  else
    if [[ "$expected" == "xfail" ]]; then echo "   ✓ xfail (as expected)"; XFAIL+=("$name")
    else echo "   ✗ FAIL (regression!)"; FAIL+=("$name"); fi
  fi
}

run_program() { # dir entry features expect logfile  -> sets RC
  local dir="$1" entry="$2" features="$3" expect="$4" log="$5"
  local featflag=""
  [[ "$features" != "-" && -n "$features" ]] && featflag="--feature $features"
  ( cd "$dir" && tish run "$entry" $featflag ) >"$log" 2>&1
  local rc=$?
  if [[ $rc -ne 0 ]]; then RC=1; return; fi
  grep -qF "$expect" "$log" && RC=0 || RC=1
}

run_serve() { # dir entry features check expect logfile -> sets RC (0 pass, 1 fail, 2 skip)
  local dir="$1" entry="$2" features="$3" check="$4" expect="$5" log="$6"
  # NOTE: split the parses across statements — `local a=.. b=${a}` reads the OUTER a, not the
  # one being declared, so port/path would come out empty (same footgun as downstream/run.sh).
  local method="${check%%:*}"
  local rest="${check#*:}"
  local port="${rest%%:*}"
  local path="${rest#*:}"
  local featflag=""
  [[ "$features" != "-" && -n "$features" ]] && featflag="--feature $features"
  # `auto` = the example honors $PORT, so relocate it to a free port. A FIXED port that is
  # already busy belongs to a foreign process we won't disturb → skip (can't test through it).
  local port_pass=""
  if [[ "$port" == "auto" ]]; then
    port=$(free_port); [[ "$port" == "0" ]] && { echo "   no free port available"; RC=2; return; }
    port_pass="$port"
  elif port_busy "$port"; then
    echo "   port $port busy (foreign process) — skip"; RC=2; return
  fi
  if [[ -n "$port_pass" ]]; then
    ( cd "$dir" && PORT="$port_pass" tish run "$entry" $featflag ) >"$log" 2>&1 &
  else
    ( cd "$dir" && tish run "$entry" $featflag ) >"$log" 2>&1 &
  fi
  local pid=$!
  local body="" i
  for ((i=0; i<80; i++)); do
    if ! kill -0 "$pid" 2>/dev/null; then break; fi   # server died early
    body=$(curl -s -m 1 -X "$method" "http://127.0.0.1:$port$path" 2>/dev/null)
    [[ -n "$body" ]] && break
    sleep 0.25
  done
  # tear down the server + any workers, and make sure the port is released for the next row
  pkill -P "$pid" 2>/dev/null; kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  free_tish_port "$port"
  # a port we thought was free but lost a race to → skip rather than cry regression
  if [[ -z "$body" ]] && grep -qiE "address already in use|failed to bind" "$log"; then
    echo "   port $port lost a bind race (foreign process) — skip"; RC=2; return
  fi
  echo "--- response body ---" >>"$log"; echo "$body" >>"$log"
  [[ -n "$body" ]] && grep -qF "$expect" <<< "$body" && RC=0 || RC=1
}

run_lattish() { # dir entry expect logfile -> sets RC  (build to JS + jsdom render)
  local dir="$1" entry="$2" expect="$3" log="$4"
  local out="$WORKDIR/$(basename "$dir")-$(basename "$entry" .tish).js"
  if ! ( cd "$dir" && tish build "$entry" -o "$out" --target js ) >"$log" 2>&1; then RC=1; return; fi
  node "$RENDER_HARNESS" "$out" "$expect" >>"$log" 2>&1 && RC=0 || RC=1
}

while IFS=$'\t' read -r name dir entry kind features check expect tags expected; do
  [[ -z "${name:-}" || "$name" =~ ^[[:space:]]*# ]] && continue
  if [[ ${#SELECT[@]} -gt 0 ]]; then
    m=0; for s in "${SELECT[@]}"; do [[ "$s" == "$name" ]] && m=1; done
    [[ $m -eq 1 ]] || continue
  elif [[ $FULL -eq 0 ]] && row_is_env_gated "$tags"; then
    echo "── $name ── SKIP (env-gated: $tags; use --full)"; SKIP+=("$name"); continue
  fi

  src=$(resolve_dir "$dir")
  if [[ ! -d "$src" ]]; then echo "── $name ── SKIP (missing: $src)"; SKIP+=("$name"); continue; fi

  # copy into scratch so run-time mutation (e.g. json-file-edit) never touches the repo
  scratch="$WORKDIR/$name"
  rsync -a --exclude node_modules --exclude dist --exclude target --exclude .git "$src/" "$scratch/" >/dev/null 2>&1

  # lattish apps: wire the LOCAL workspace lattish under node_modules/lattish (bare `import "lattish"`)
  if [[ "$kind" == "lattish" ]]; then
    if [[ ! -d "$LATTISH_WS" ]]; then echo "── $name ── SKIP (no workspace lattish at $LATTISH_WS)"; SKIP+=("$name"); continue; fi
    if [[ $HAVE_JSDOM -eq 0 ]]; then echo "── $name ── SKIP (jsdom not installed in $LATTISH_WS; run npm install there)"; SKIP+=("$name"); continue; fi
    mkdir -p "$scratch/node_modules"
    rsync -a --exclude node_modules --exclude .git "$LATTISH_WS/" "$scratch/node_modules/lattish/" >/dev/null 2>&1
  fi

  echo "── $name ── $kind  ($entry, expect \"$expect\", expected=$expected)"
  log="$WORKDIR/$name.log"; RC=1
  case "$kind" in
    run)     run_program "$scratch" "$entry" "$features" "$expect" "$log" ;;
    serve)   run_serve   "$scratch" "$entry" "$features" "$check" "$expect" "$log" ;;
    lattish) run_lattish "$scratch" "$entry" "$expect" "$log" ;;
    *) echo "   ✗ unknown kind: $kind"; FAIL+=("$name"); continue ;;
  esac
  if [[ $RC -eq 2 ]]; then SKIP+=("$name"); continue; fi
  [[ $RC -ne 0 ]] && { echo "   — last log lines —"; tail -3 "$log" | sed 's/^/      /'; }
  classify "$RC" "$expected" "$name"
done < <(grep -vE '^\s*#|^\s*$' "$MANIFEST")

echo ""
echo "═════════════════════ EXAMPLE REGRESSION SUMMARY ═════════════════════"
printf "  PASS (%d):   %s\n" "${#PASS[@]}" "${PASS[*]:-(none)}"
printf "  xfail (%d):  %s\n" "${#XFAIL[@]}" "${XFAIL[*]:-(none)}"
printf "  SKIP (%d):   %s\n" "${#SKIP[@]}" "${SKIP[*]:-(none)}"
[[ ${#UNXPASS[@]} -gt 0 ]] && printf "  ⚠ UNEXPECTED PASS (flip to pass): %s\n" "${UNXPASS[*]}"
[[ ${#FAIL[@]} -gt 0 ]]    && printf "  ✗ REGRESSIONS: %s\n" "${FAIL[*]}"
echo "══════════════════════════════════════════════════════════════════════"

[[ ${#FAIL[@]} -eq 0 ]] || exit 1
exit 0
