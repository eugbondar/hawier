#!/usr/bin/env bash
# Poll benchmark: a fleet hammers the DDI poll endpoint (the real production
# load) against a running hawier; results land in bench/poll-history.csv.
#
#   make load-poll
#   COUNT=5000 INTERVAL=500ms DURATION_S=60 make load-poll
set -euo pipefail

URL="${URL:-http://localhost:8080}"
COUNT="${COUNT:-2000}"
INTERVAL="${INTERVAL:-1s}"
DURATION_S="${DURATION_S:-60}"
USER_PASS="${USER_PASS:-admin:admin}"
GATEWAY_KEY="${GATEWAY_KEY:-load-gateway-key}"
PREFIX="poll$(date +%s)"
# Where the fleet polls. In production devices talk to sub nodes while
# operators talk to control; on one node the script's own progress query
# queues behind the fleet's DDI traffic and ends up measuring its own
# starvation rather than the update. Defaults to URL so a single-node
# deployment still works.
DDI_URL="${DDI_URL:-$URL}"
DEVICE_BIN="${DEVICE_BIN:-target/release/hawier-device}"
# Build it here rather than in the make target. `bash scripts/load.sh` is how
# this gets run interactively, and that path silently used whatever binary
# happened to be in target/release: one 1000-device run measured a client
# from an hour earlier and looked like the fix under test did nothing.
# cargo is a no-op when it is already current.
if [ "$DEVICE_BIN" = "target/release/hawier-device" ]; then
    cargo build --release -p hawier-client >&2 || exit 1
fi
HISTORY="${HISTORY:-bench/poll-history.csv}"

# Both nodes are ours, wherever they live: a host proxy in the environment
# would otherwise swallow the run (curl exits 22 on the first call). Split
# roles mean non-localhost addresses, so the hosts are taken from the URLs.
BENCH_HOSTS="localhost,127.0.0.1,$(printf '%s\n%s\n' "$URL" "$DDI_URL" \
    | sed -E 's#^[a-z]+://##; s#[:/].*##' | sort -u | paste -sd,)"
export no_proxy="$BENCH_HOSTS" NO_PROXY="$BENCH_HOSTS"
AUTH=(-u "$USER_PASS")

api() { curl -sf "${AUTH[@]}" -H "Content-Type: application/json" "$@"; }
jqpy() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

# Same rule as scripts/load.sh: a benchmark that leaves its fleet behind makes
# the next measurement incomparable. On ANY exit — including a timeout or a
# Ctrl-C — the devices this run registered go away again. KEEP=1 keeps them.
OUT="$(mktemp)"
cleanup() {
  local status=$?
  set +e
  rm -f "$OUT"
  [ "${KEEP:-0}" = "1" ] && return $status
  echo "== cleaning up $PREFIX =="
  while :; do
    local ids
    ids=$(api "$URL/rest/v1/targets?q=controllerid==$PREFIX-*&limit=500" 2>/dev/null \
          | jqpy "'\n'.join(t['controllerId'] for t in d['content'])" 2>/dev/null)
    [ -z "$ids" ] && break
    echo "$ids" | xargs -P 16 -I{} curl -s "${AUTH[@]}" -X DELETE \
      "$URL/rest/v1/targets/{}" -o /dev/null
  done
  echo "   left behind: $(api "$URL/rest/v1/targets?q=controllerid==$PREFIX-*&limit=1" 2>/dev/null | jqpy "d['total']" 2>/dev/null) targets"
  return $status
}
# EXIT alone is not enough: bash skips an EXIT trap when it dies from an
# uncaught signal, so a Ctrl-C or a `timeout` left the fleet behind anyway.
trap cleanup EXIT INT TERM

echo "== hawier poll benchmark: $COUNT devices @ $INTERVAL for ${DURATION_S}s =="
echo "   mgmt $URL | fleet $DDI_URL"

# Gateway-token auth for plug-and-play registration.
curl -sf "${AUTH[@]}" -H "Content-Type: application/json" -X PUT \
    -d "{\"authentication.gatewaytoken.enabled\":{\"value\":true},
         \"authentication.gatewaytoken.key\":{\"value\":\"$GATEWAY_KEY\"}}" \
    "$URL/rest/v1/system/configs" >/dev/null

# Register the fleet (one cycle each), then hammer polls.
"$DEVICE_BIN" --url "$DDI_URL" --controller-id "$PREFIX" --gateway-token "$GATEWAY_KEY" \
    --count "$COUNT" --once >/dev/null 2>&1

timeout "${DURATION_S}s" "$DEVICE_BIN" --url "$DDI_URL" --controller-id "$PREFIX" \
    --gateway-token "$GATEWAY_KEY" --count "$COUNT" \
    --poll-only --poll-override "$INTERVAL" > "$OUT" || true

LAST=$(grep "^POLLSTAT" "$OUT" | tail -1)
[ -n "$LAST" ] || { echo "FAIL: no POLLSTAT output"; exit 1; }
echo "$LAST"

GIT_REV=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
mkdir -p "$(dirname "$HISTORY")"
[ -f "$HISTORY" ] || echo "date,git_rev,count,interval,duration_s,total,errors,avg_rps,avg_ms,max_ms" > "$HISTORY"

python3 - "$LAST" "$COUNT" "$INTERVAL" "$DURATION_S" "$GIT_REV" "$HISTORY" <<'PY'
import datetime
import sys

line, count, interval, duration, rev, history = sys.argv[1:7]
kv = dict(part.split("=") for part in line.split()[1:])
elapsed = float(kv["elapsed"]) or 1.0
total, errors = int(kv["total"]), int(kv["errors"])
avg_rps = total / elapsed
print(f"== RESULT: {total} polls in {elapsed:.0f}s ({avg_rps:.0f} avg polls/s, "
      f"errors {errors}, avg {kv['avg_ms']} ms, max {kv['max_ms']} ms) ==")
with open(history, "a") as f:
    f.write(f"{datetime.date.today().isoformat()},{rev},{count},{interval},"
            f"{duration},{total},{errors},{avg_rps:.0f},{kv['avg_ms']},{kv['max_ms']}\n")
print(f"recorded in {history}")
PY
