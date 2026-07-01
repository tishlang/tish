#!/usr/bin/env bash
# Runs ON an ephemeral Linux droplet (see scripts/do_http_test_runner.sh).
# Provisions toolchain, builds tish, and runs the multi-worker HTTP + multithread
# tests that macOS cannot validate (BSD SO_REUSEPORT funnels accepts to one worker;
# Linux kernel-load-balances → real per-core scaling).
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
NCPU=$(nproc)
log(){ printf '\n========== %s ==========\n' "$*"; }

log "PROVISION (nproc=$NCPU)"
apt-get update -y >/tmp/apt.log 2>&1
apt-get install -y build-essential git curl jq pkg-config libssl-dev ca-certificates taskset >>/tmp/apt.log 2>&1 \
  || apt-get install -y build-essential git curl jq pkg-config libssl-dev ca-certificates util-linux >>/tmp/apt.log 2>&1
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/tmp/rustup.log 2>&1
fi
# shellcheck source=/dev/null
. "$HOME/.cargo/env"
if ! node --version 2>/dev/null | grep -q '^v24'; then
  # Prefer NodeSource v24; verify the major actually took (Ubuntu's apt otherwise
  # installs its old default nodejs and we'd silently benchmark against node 18).
  curl -fsSL https://deb.nodesource.com/setup_24.x | bash - >/tmp/node.log 2>&1 \
    && apt-get install -y nodejs >>/tmp/node.log 2>&1
  if ! node --version 2>/dev/null | grep -q '^v24'; then
    # Fallback: official prebuilt tarball into /usr/local (no apt).
    f=$(curl -fsSL https://nodejs.org/dist/latest-v24.x/ 2>/dev/null | grep -oE 'node-v24[0-9.]*-linux-x64\.tar\.xz' | head -1)
    [ -n "$f" ] && curl -fsSL "https://nodejs.org/dist/latest-v24.x/$f" -o /tmp/node.tar.xz 2>/tmp/node.log \
      && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 2>>/tmp/node.log
  fi
fi
if ! command -v oha >/dev/null 2>&1; then
  curl -fsSL -o /usr/local/bin/oha https://github.com/hatoo/oha/releases/latest/download/oha-linux-amd64 2>/tmp/oha.log \
    && chmod +x /usr/local/bin/oha
  command -v oha >/dev/null 2>&1 || { echo "oha download failed; cargo install (slow)"; cargo install oha >/tmp/oha_build.log 2>&1; }
fi
if ! command -v bun >/dev/null 2>&1; then
  curl -fsSL https://bun.sh/install | bash >/tmp/bun.log 2>&1 || true
fi
export BUN_INSTALL="$HOME/.bun"; export PATH="$BUN_INSTALL/bin:$PATH"
echo "toolchain: $(cargo --version) | node $(node --version 2>&1) | bun $(bun --version 2>&1 | head -1) | oha $(oha --version 2>&1 | head -1)"

cd "$HOME/tish" || { echo "no repo at ~/tish"; exit 1; }
echo "commit under test: $(git rev-parse --short HEAD 2>/dev/null || echo '(archive, no .git)')"

log "BUILD tish (release)"
t0=$(date +%s)
cargo build --release --bin tish >/tmp/build.log 2>&1 || { echo "BUILD FAILED"; tail -50 /tmp/build.log; exit 1; }
echo "built in $(( $(date +%s) - t0 ))s"

log "TEST 1  correctness under threads — concurrent_shared_state (12 threads x 100 calls, send-values)"
cargo test -p tishlang_vm --features send-values --test concurrent_shared_state -- --nocapture 2>&1 | tail -25 \
  || echo "(concurrent_shared_state exit $?)"

log "TEST 2  shared-counter regression — test_http_concurrency.sh -n 8"
bash scripts/test_http_concurrency.sh -n 8 2>&1 | tail -25 || echo "(test_http_concurrency exit $?)"

log "TEST 3  multi-worker throughput (tiny_http) — tish w=1 vs w=$NCPU vs node  [THE Linux scaling proof]"
bash scripts/run_http_perf.sh --duration 5s --connections 128 --workers "$NCPU" 2>&1 || echo "(run_http_perf exit $?)"

log "TEST 3b  pinned scaling curve — server pinned off the load-gen cores (isolates server scaling)"
if command -v taskset >/dev/null 2>&1 && [[ "$NCPU" -ge 4 && -x /tmp/tish_http_perf_server ]]; then
  nsrv=$((NCPU-2)); srvcores="2-$((NCPU-1))"; loadcores="0,1"
  echo "load-gen pinned to cores $loadcores ; server pinned to cores $srvcores"
  for w in 1 2 "$nsrv"; do
    PORT=8092 TISH_HTTP_WORKERS="$w" taskset -c "$srvcores" /tmp/tish_http_perf_server >/tmp/sw.log 2>&1 &
    sp=$!
    for _ in $(seq 1 60); do curl -s localhost:8092/plaintext >/dev/null 2>&1 && break; sleep 0.1; done
    taskset -c "$loadcores" oha --no-tui --output-format json -z 5s -c 128 http://127.0.0.1:8092/json 2>/dev/null \
      | jq -r --arg w "$w" '"  workers="+$w+"  req/s(/json)="+(.summary.requestsPerSec|floor|tostring)+"  p50ms="+((.latencyPercentiles.p50*1000*100|round)/100|tostring)+"  success="+(.summary.successRate|tostring)'
    kill "$sp" 2>/dev/null; wait "$sp" 2>/dev/null
  done
else
  echo "(skipped: need taskset, >=4 cores, and the tiny_http server from TEST 3)"
fi

log "TEST 4  hyper backend (TFB tish-rust variant) — build + bench w=1 vs w=$NCPU"
if target/release/tish build tests/http/server.tish -o /tmp/srvh --target native --native-backend rust \
     --feature http --feature http-hyper --feature process >/tmp/hyper_build.log 2>&1; then
  for w in 1 "$NCPU"; do
    PORT=8091 TISH_HTTP_BACKEND=hyper TISH_HTTP_WORKERS="$w" /tmp/srvh >/tmp/srvh.log 2>&1 &
    sp=$!
    for _ in $(seq 1 60); do curl -s localhost:8091/plaintext >/dev/null 2>&1 && break; sleep 0.1; done
    ok=$(curl -s -o /dev/null -w '%{http_code}' localhost:8091/json 2>/dev/null)
    oha --no-tui --output-format json -z 5s -c 128 http://127.0.0.1:8091/json 2>/dev/null \
      | jq -r --arg w "$w" --arg ok "$ok" '"  hyper workers="+$w+"  http="+$ok+"  req/s(/json)="+(.summary.requestsPerSec|floor|tostring)+"  success="+(.summary.successRate|tostring)'
    kill "$sp" 2>/dev/null; wait "$sp" 2>/dev/null
  done
else
  echo "hyper build FAILED:"; tail -40 /tmp/hyper_build.log
fi

log "DONE — $(uname -srm)  kernel $(uname -r)  $(nproc) cores"
