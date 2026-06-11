#!/usr/bin/env bash
# Run a cargo build/test command with resilience to transient sccache / GitHub-Actions-cache
# failures. sccache is wired in as RUSTC_WRAPPER and validates its storage backend at server
# startup; when the GHA cache service (`ghac`) returns a transient error (e.g. a 502 Bad Gateway
# from its nginx proxy), the sccache server fails to start and — because every rustc invocation
# goes through it — the whole compile fails (cargo exit 101).
#
# Strategy: retry the command a few times (these 502s are momentary), then as a last resort rebuild
# with sccache disabled, so a sustained cache outage still produces a green build (just slower).
#
# Usage: scripts/ci_build_retry.sh <command> [args...]
set -uo pipefail

attempts="${CI_RETRY_ATTEMPTS:-3}"
for i in $(seq 1 "$attempts"); do
  if "$@"; then
    exit 0
  fi
  if [ "$i" -lt "$attempts" ]; then
    delay=$((i * 15))
    echo "::warning::'$*' failed (attempt ${i}/${attempts}) — likely a transient sccache/cache flake; retrying in ${delay}s"
    sleep "$delay"
  fi
done

echo "::warning::sccache-backed command failed ${attempts}× — retrying once with sccache disabled (RUSTC_WRAPPER unset)"
RUSTC_WRAPPER='' SCCACHE_GHA_ENABLED='false' "$@"
