#!/usr/bin/env bash
# Load test: a real rollout to a simulated fleet against a running hawier.
#
#   make load                      # defaults: 200 devices, 4 MB artifact
#   COUNT=1000 SIZE_MB=16 make load
#
# Requires: a running hawier (make up or make run), curl, python3, and the
# hawier-device binary (built automatically by the make target).
set -euo pipefail

URL="${URL:-http://localhost:8080}"
COUNT="${COUNT:-200}"
SIZE_MB="${SIZE_MB:-4}"
USER_PASS="${USER_PASS:-admin:admin}"
GATEWAY_KEY="${GATEWAY_KEY:-load-gateway-key}"
PREFIX="load$(date +%s)"
# Where the fleet polls. In production devices talk to sub nodes while
# operators talk to control; on one node the script's own progress query
# queues behind the fleet's DDI traffic and ends up measuring its own
# starvation rather than the update. Defaults to URL so a single-node
# deployment still works.
DDI_URL="${DDI_URL:-$URL}"

# The simulator gives a single device the controller id exactly as passed,
# with no "-1" suffix (that is what a real single device wants), and this
# script looks the fleet up by the "$PREFIX-*" mask — so COUNT=1 registers a
# device the script then cannot find and fails as "registration incomplete".
[ "$COUNT" -ge 2 ] || { echo "FAIL: COUNT must be 2 or more (see the comment above)"; exit 1; }

DEVICE_BIN="${DEVICE_BIN:-target/release/hawier-device}"
# Build it here rather than in the make target. `bash scripts/load.sh` is how
# this gets run interactively, and that path silently used whatever binary
# happened to be in target/release: one 1000-device run measured a client
# from an hour earlier and looked like the fix under test did nothing.
# cargo is a no-op when it is already current.
if [ "$DEVICE_BIN" = "target/release/hawier-device" ]; then
    cargo build --release -p hawier-client >&2 || exit 1
fi
TIMEOUT_S="${TIMEOUT_S:-600}"

# Both nodes are ours, wherever they live: a host proxy in the environment
# would otherwise swallow the run (curl exits 22 on the first call). Split
# roles mean non-localhost addresses, so the hosts are taken from the URLs.
BENCH_HOSTS="localhost,127.0.0.1,$(printf '%s\n%s\n' "$URL" "$DDI_URL" \
    | sed -E 's#^[a-z]+://##; s#[:/].*##' | sort -u | paste -sd,)"
export no_proxy="$BENCH_HOSTS" NO_PROXY="$BENCH_HOSTS"
AUTH=(-u "$USER_PASS")
api() { curl -sf "${AUTH[@]}" -H "Content-Type: application/json" "$@"; }
jqpy() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

echo "== hawier load test: $COUNT devices, ${SIZE_MB}MB artifact =="
echo "   mgmt $URL | fleet $DDI_URL"

# Both nodes must be release builds, or the number is about rustc rather than
# about hawier. Images are built with the dev profile by default — they exist
# to be looked at — and a debug node is several times slower while behaving
# identically, which is exactly the kind of difference that would land in
# load-history.csv looking like a regression in the code.
#
# Asked of the node itself (`debug_assertions`, which the profile sets and
# nothing can override) rather than inferred from how this script was called.
check_profile() {
    local where=$1 url=$2 profile
    profile=$(api "$url/rest/v1/system/extensions" 2>/dev/null \
        | jqpy "d.get('node', {}).get('profile', 'unknown')" 2>/dev/null) || profile=unknown
    if [ "$profile" != "release" ]; then
        cat >&2 <<MSG
FAIL: the $where node reports a '$profile' build, and this measurement would
      be recorded in bench/load-history.csv beside release numbers.

      Rebuild the stack optimised first:

          CARGO_PROFILE=release make scale

      PROFILE_CHECK=0 skips this, for a run you do not intend to record.
MSG
        exit 1
    fi
}
if [ "${PROFILE_CHECK:-1}" = 1 ]; then
    check_profile control "$URL"
    [ "$DDI_URL" = "$URL" ] || check_profile fleet "$DDI_URL"
fi

# Clean up after ourselves — on ANY exit. Every run used to leave COUNT
# targets, a DS, a module, an artifact and a rollout behind for good; the
# database grew from run to run and two measurements with the same parameters
# stopped being comparable (that is what the 49.2 -> 5.2 updates/s
# "regression" in the history really was). The first version of this cleanup
# sat at the end of the script, so a timed-out or failed run still leaked
# a thousand targets — hence the trap. KEEP=1 skips it for a post-mortem.
cleanup() {
  local status=$?
  # `set +e` FIRST, before anything that can fail: killing a fleet that has
  # already exited returns non-zero, and under `set -e` that aborted the
  # whole cleanup silently — the leak this trap exists to prevent.
  set +e
  # Local scratch first, then the fleet process, then the server-side rows.
  [ -n "${FLEET_PID:-}" ] && kill "$FLEET_PID" 2>/dev/null
  [ -n "${ARTIFACT:-}" ] && rm -f "$ARTIFACT"
  [ "${KEEP:-0}" = "1" ] && return $status
  echo "== cleaning up $PREFIX =="
  [ -n "${ROLLOUT_ID:-}" ] && api -X DELETE "$URL/rest/v1/rollouts/$ROLLOUT_ID" >/dev/null 2>&1
  # In parallel and in pages: a thousand sequential DELETEs took longer than
  # the measurement itself.
  while :; do
    local ids
    ids=$(api "$URL/rest/v1/targets?q=controllerid==$PREFIX-*&limit=500" 2>/dev/null \
          | jqpy "'\n'.join(t['controllerId'] for t in d['content'])" 2>/dev/null)
    [ -z "$ids" ] && break
    echo "$ids" | xargs -P 16 -I{} curl -s "${AUTH[@]}" -X DELETE \
      "$URL/rest/v1/targets/{}" -o /dev/null
  done
  [ -n "${DS_ID:-}" ] && api -X DELETE "$URL/rest/v1/distributionsets/$DS_ID" >/dev/null 2>&1
  [ -n "${SM_ID:-}" ] && api -X DELETE "$URL/rest/v1/softwaremodules/$SM_ID" >/dev/null 2>&1
  local left
  left=$(api "$URL/rest/v1/targets?q=controllerid==$PREFIX-*&limit=1" 2>/dev/null | jqpy "d['total']" 2>/dev/null)
  echo "   left behind: ${left:-?} targets"
  return $status
}
# EXIT alone is not enough: bash skips an EXIT trap when it dies from an
# uncaught signal, so a Ctrl-C or a `timeout` left the fleet behind anyway.
trap cleanup EXIT INT TERM

# 1. Enable gateway-token auth for plug-and-play registration.
api -X PUT -d "{\"authentication.gatewaytoken.enabled\":{\"value\":true},
                \"authentication.gatewaytoken.key\":{\"value\":\"$GATEWAY_KEY\"}}" \
    "$URL/rest/v1/system/configs" >/dev/null

# 2. Software module + artifact of SIZE_MB random bytes.
SM_ID=$(api -X POST -d "[{\"name\":\"$PREFIX-fw\",\"version\":\"1.0\",\"type\":\"os\"}]" \
    "$URL/rest/v1/softwaremodules" | jqpy "d[0]['id']")
ARTIFACT="$(mktemp)"
head -c "$((SIZE_MB * 1024 * 1024))" /dev/urandom > "$ARTIFACT"
curl -sf "${AUTH[@]}" -F "file=@$ARTIFACT;filename=$PREFIX.bin" \
    "$URL/rest/v1/softwaremodules/$SM_ID/artifacts" >/dev/null
DS_ID=$(api -X POST -d "[{\"name\":\"$PREFIX-ds\",\"version\":\"1.0\",\"type\":\"os\",
                          \"modules\":[{\"id\":$SM_ID}]}]" \
    "$URL/rest/v1/distributionsets" | jqpy "d[0]['id']")
echo "software module $SM_ID, distribution set $DS_ID"

# 3. Register the fleet (one poll each).
T_REG0=$(date +%s.%N)
"$DEVICE_BIN" --url "$DDI_URL" --controller-id "$PREFIX" --gateway-token "$GATEWAY_KEY" \
    --count "$COUNT" --once >/dev/null 2>&1
T_REG1=$(date +%s.%N)
REGISTERED=$(api "$URL/rest/v1/targets?q=controllerid==$PREFIX-*&limit=1" | jqpy "d['total']")
echo "registered $REGISTERED devices in $(python3 -c "print(f'{$T_REG1-$T_REG0:.1f}')")s"
[ "$REGISTERED" -eq "$COUNT" ] || { echo "FAIL: registration incomplete"; exit 1; }

# 4. Rollout: one group, 100% success threshold, start it.
ROLLOUT_ID=$(api -X POST -d "{\"name\":\"$PREFIX-rollout\",\"distributionSetId\":$DS_ID,
    \"targetFilterQuery\":\"controllerid==$PREFIX-*\",\"amountGroups\":1,
    \"successCondition\":{\"condition\":\"THRESHOLD\",\"expression\":\"100\"}}" \
    "$URL/rest/v1/rollouts" | jqpy "d['id']")
for _ in $(seq 1 60); do
    STATUS=$(api "$URL/rest/v1/rollouts/$ROLLOUT_ID" | jqpy "d['status']")
    [ "$STATUS" = "ready" ] && break
    sleep 1
done
[ "$STATUS" = "ready" ] || { echo "FAIL: rollout stuck in $STATUS"; exit 1; }
api -X POST "$URL/rest/v1/rollouts/$ROLLOUT_ID/start" -d '{}' >/dev/null
echo "rollout $ROLLOUT_ID started"

# 5. Fleet polls fast until every action is FINISHED.
T0=$(date +%s.%N)
# The fleet's own log used to go to /dev/null, so a device that got stuck
# left no trace anywhere: the run just reported N of COUNT and the reason
# died with the process. DEVICE_LOG=/tmp/fleet.log keeps it for a post-mortem
# (with KEEP=1, which leaves the targets behind to match it against).
"$DEVICE_BIN" --url "$DDI_URL" --controller-id "$PREFIX" --gateway-token "$GATEWAY_KEY" \
    --count "$COUNT" --poll-override 500ms >"${DEVICE_LOG:-/dev/null}" 2>&1 &
FLEET_PID=$!

FINISHED=0
DEADLINE=$(( $(date +%s) + TIMEOUT_S ))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    FINISHED=$(api "$URL/rest/v1/rollouts/$ROLLOUT_ID" \
        | jqpy "d['totalTargetsPerStatus']['finished']")
    [ "$FINISHED" -ge "$COUNT" ] && break
    sleep 2
done
T1=$(date +%s.%N)
kill $FLEET_PID 2>/dev/null || true

[ "$FINISHED" -ge "$COUNT" ] || { echo "FAIL: only $FINISHED/$COUNT finished in ${TIMEOUT_S}s"; exit 1; }

GIT_REV=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
HISTORY="${HISTORY:-bench/load-history.csv}"
mkdir -p "$(dirname "$HISTORY")"
[ -f "$HISTORY" ] || echo "date,git_rev,count,size_mb,seconds,updates_per_s,mb_per_s" > "$HISTORY"

python3 - "$COUNT" "$SIZE_MB" "$T0" "$T1" "$GIT_REV" "$HISTORY" <<'PY'
import datetime
import sys

count, size_mb, t0, t1 = int(sys.argv[1]), float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
git_rev, history = sys.argv[5], sys.argv[6]
dt = t1 - t0
gbits = count * size_mb * 8 / 1000 / dt
print(f"== RESULT: {count} devices updated in {dt:.1f}s "
      f"({count/dt:.1f} updates/s, ~{count*size_mb/dt:.0f} MB/s ≈ {gbits:.2f} Gbit/s payload) ==")
with open(history, "a") as f:
    f.write(f"{datetime.date.today().isoformat()},{git_rev},{count},{size_mb:g},"
            f"{dt:.1f},{count/dt:.1f},{count*size_mb/dt:.0f}\n")
print(f"recorded in {history} — compare against previous rows for regressions")
PY
