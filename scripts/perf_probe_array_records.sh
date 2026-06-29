#!/usr/bin/env bash
# De-risk the array_records lever: does a TYPE-ANNOTATED array-of-records already lower to Vec<struct>
# and beat node? If yes, the only missing piece for the untyped fixture is inference. One run.
# Output: target/ar_probe.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/ar_probe.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
mkdir -p target/probe

log "== ensure current release tish =="
cargo build --release --bin tish > target/probe/tish_build.log 2>&1 || { log "tish build FAIL"; tail -20 target/probe/tish_build.log | tee -a "$OUT"; exit 1; }
TISH=target/release/tish

# Typed variant: same computation as tests/perf/array_records.tish, but with a Row type so the
# existing object->struct / Vec<Named> codegen can fire.
cat > target/probe/ar_typed.tish <<'TISH'
type Row = { id: number, x: number, y: number, w: number }
let n = 2000000
let rows: Row[] = []
for (let i = 0; i < n; i++) { rows.push({ id: i, x: i * 2, y: i + 1, w: (i % 7) + 1 }) }
let t0 = Date.now()
let acc = 0
for (let pass = 0; pass < 10; pass++) {
  for (let i = 0; i < rows.length; i++) {
    acc = (acc + rows[i].x * rows[i].w + rows[i].y) % 2147483647
  }
}
let CHECK = acc % 2147483647
console.log("typed_array_records " + (Date.now() - t0) + " " + CHECK)
TISH

log "== build TYPED variant (keep generated Rust to confirm struct lowering) =="
TISH_KEEP_BUILD=1 TISH_KEEP=1 "$TISH" build target/probe/ar_typed.tish -o target/probe/ar_typed_bin > target/probe/ar_build.log 2>&1 || { log "typed build FAIL"; tail -25 target/probe/ar_build.log | tee -a "$OUT"; }
log "== run TYPED variant (tish native) =="
[ -x target/probe/ar_typed_bin ] && target/probe/ar_typed_bin 2>&1 | tee -a "$OUT"

log "== boxed (TISH_NATIVE_OPT=0) typed variant, for A/B =="
TISH_NATIVE_OPT=0 "$TISH" build target/probe/ar_typed.tish -o target/probe/ar_boxed_bin > target/probe/ar_boxed_build.log 2>&1 && target/probe/ar_boxed_bin 2>&1 | tee -a "$OUT" || log "(boxed build skipped/failed)"

log "== node on the ORIGINAL untyped fixture =="
node tests/perf/array_records.tish 2>&1 | tee -a "$OUT"

log "== did codegen emit a native struct + Vec<struct>? =="
find /var/folders /private/var/folders "${TMPDIR:-/tmp}" -path '*ar_typed*' -name '*.rs' 2>/dev/null | head -1 | while read -r f; do
  echo "rust: $f"; grep -nE "struct Tish|Vec<Tish|: f64|\.id\b|\.x\b" "$f" | head -20
done
grep -nE "struct Tish|Vec<Tish" target/probe/ar_build.log 2>/dev/null | head
log DONE
