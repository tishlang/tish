#!/usr/bin/env bash
# Ephemeral DigitalOcean test runner for tish's MULTI-WORKER / MULTITHREAD HTTP path.
#
# macOS cannot validate multi-worker scaling: BSD SO_REUSEPORT funnels every accept to
# one worker, so `tish w=N` reads ~ `tish w=1`. On Linux the kernel load-balances across
# the prefork workers, so scaling is a real, observable property there. This spins up a
# short-lived Linux droplet, runs the tests, prints/collects the results, and ALWAYS
# destroys the droplet (trap on EXIT/INT/TERM) so it never keeps billing.
#
# Uses the already-authenticated `doctl`; ships the CURRENT COMMIT via `git archive HEAD`
# (no repo credentials, no target/ blowup). Requires: doctl, ssh, scp, git.
#
# Overrides (env): DO_SIZE DO_REGION DO_IMAGE DO_SSH_KEY_ID DO_SSH_KEY DO_KEEP=1
set -uo pipefail

SIZE="${DO_SIZE:-s-8vcpu-16gb}"          # 8 vCPU — clearest scaling curve; ~$0.14/hr
REGION="${DO_REGION:-nyc3}"
IMAGE="${DO_IMAGE:-ubuntu-24-04-x64}"
SSH_KEY_ID="${DO_SSH_KEY_ID:-54561881}"  # the 'a_' key registered with this DO account
SSH_KEY="${DO_SSH_KEY:-$HOME/.ssh/id_ed25519_digitalocean}"
KEEP="${DO_KEEP:-0}"

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
NAME="tish-http-test-$$"
OUTDIR="$REPO_DIR/target/do-http-results"
mkdir -p "$OUTDIR"
LOG="$OUTDIR/run-$$.log"
ARCHIVE="$OUTDIR/src-$$.tgz"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o ConnectTimeout=10 -o ServerAliveInterval=15 -i "$SSH_KEY")

DROPLET_ID=""
cleanup() {
  rm -f "$ARCHIVE" 2>/dev/null
  if [[ -n "$DROPLET_ID" ]]; then
    if [[ "$KEEP" == "1" ]]; then
      echo ">> DO_KEEP=1 — leaving droplet $DROPLET_ID up. Destroy with: doctl compute droplet delete $DROPLET_ID -f" | tee -a "$LOG"
      return
    fi
    echo ">> destroying droplet $DROPLET_ID ..." | tee -a "$LOG"
    if doctl compute droplet delete "$DROPLET_ID" -f >/dev/null 2>&1; then
      echo ">> destroyed $DROPLET_ID" | tee -a "$LOG"
    else
      echo "!! FAILED to destroy $DROPLET_ID — RUN THIS NOW: doctl compute droplet delete $DROPLET_ID -f" | tee -a "$LOG"
    fi
  fi
}
trap cleanup EXIT INT TERM

need(){ command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1"; exit 1; }; }
need doctl; need ssh; need scp; need git
[[ -f "$SSH_KEY" ]] || { echo "no ssh private key at $SSH_KEY (set DO_SSH_KEY)"; exit 1; }
doctl account get >/dev/null 2>&1 || { echo "doctl not authenticated (doctl auth init)"; exit 1; }

echo ">> creating droplet $NAME  ($SIZE / $REGION / $IMAGE)" | tee -a "$LOG"
DROPLET_ID="$(doctl compute droplet create "$NAME" \
  --size "$SIZE" --region "$REGION" --image "$IMAGE" \
  --ssh-keys "$SSH_KEY_ID" --wait --no-header --format ID 2>>"$LOG")"
[[ -n "$DROPLET_ID" ]] || { echo "droplet create failed (see $LOG)"; exit 1; }
echo ">> droplet id=$DROPLET_ID" | tee -a "$LOG"

IP="$(doctl compute droplet get "$DROPLET_ID" --no-header --format PublicIPv4)"
[[ -n "$IP" ]] || { echo "no public IP for $DROPLET_ID"; exit 1; }
echo ">> ip=$IP" | tee -a "$LOG"

echo ">> waiting for ssh ..." | tee -a "$LOG"
up=0
for i in $(seq 1 60); do
  if ssh "${SSH_OPTS[@]}" "root@$IP" true >/dev/null 2>&1; then up=1; break; fi
  sleep 5
done
[[ "$up" == 1 ]] || { echo "ssh never came up on $IP"; exit 1; }
echo ">> ssh up" | tee -a "$LOG"

echo ">> shipping repo (git archive HEAD: $(git -C "$REPO_DIR" rev-parse --short HEAD))" | tee -a "$LOG"
git -C "$REPO_DIR" archive --format=tar.gz -o "$ARCHIVE" HEAD
ssh "${SSH_OPTS[@]}" "root@$IP" 'mkdir -p ~/tish'
scp "${SSH_OPTS[@]}" "$ARCHIVE" "root@$IP:/root/src.tgz" >/dev/null
scp "${SSH_OPTS[@]}" "$REPO_DIR/scripts/remote_http_test.sh" "root@$IP:/root/remote_http_test.sh" >/dev/null
ssh "${SSH_OPTS[@]}" "root@$IP" 'tar xzf /root/src.tgz -C ~/tish'

echo ">> running tests (streaming below; full log at $LOG)" | tee -a "$LOG"
echo "=====================================================================" | tee -a "$LOG"
ssh "${SSH_OPTS[@]}" "root@$IP" 'bash /root/remote_http_test.sh' 2>&1 | tee -a "$LOG"
echo "=====================================================================" | tee -a "$LOG"
echo ">> results saved to $LOG" | tee -a "$LOG"
# droplet destroyed by trap
