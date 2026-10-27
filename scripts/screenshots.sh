#!/usr/bin/env bash
# Console screenshots for the README (spec: build-tooling, "Console
# screenshots"; spec: repository-docs). Entry point: `make screenshots`.
#
# It brings up a stand of its own — a dev node and a dev console on loopback
# ports nobody else uses, over a database of its own in the compose
# PostgreSQL — seeds it through Mgmt and the device simulator, captures the
# pages in PAGES with a pinned headless browser in a container, and takes
# everything down again, pass or fail. The committed pictures in
# docs/assets/screenshots/ are replaced only when every page succeeded.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE=${ENGINE:-$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)}
COMPOSE="$ENGINE compose -f deploy/docker-compose.yml"
# Full registry name: podman with short-name-mode=enforcing and no TTY does
# not resolve a short one. Exact version, so a rerun captures with the same
# browser.
BROWSER_IMAGE=mcr.microsoft.com/playwright/python:v1.63.0-noble
# The image carries the browsers but not the Python package that drives them.
# It is installed once into PY_DIR, with every dependency pinned, so a rerun
# needs no network beyond the image itself.
PY_PACKAGES=(playwright==1.63.0 pyee==13.0.1 greenlet==3.5.6 typing_extensions==4.16.0)
PY_DIR=target/screenshots-py
DB=hawier_shots
# Distinct from the compose stand (8080/8081/8088) and from make e2e (18080).
NODE_PORT=18180
CONSOLE_PORT=18188
NODE="http://127.0.0.1:$NODE_PORT"
CONSOLE="http://127.0.0.1:$CONSOLE_PORT"
AUTH=admin:admin
GATEWAY_KEY=shots-gateway-key
INTERNAL_TOKEN=shots-internal-token
EXT=target/wasm32-wasip1/debug
RUN_DIR=target/screenshots
OUT_DIR=docs/assets/screenshots
MAX_BYTES=$((600 * 1024))

export no_proxy=127.0.0.1,localhost NO_PROXY=127.0.0.1,localhost

say() { echo "screenshots: $*"; }
die() { echo "screenshots: FAIL: $*" >&2; exit 1; }

# --- preconditions ------------------------------------------------------------

for tool in curl python3 "$ENGINE"; do
	command -v "$tool" >/dev/null 2>&1 || die "$tool is required and was not found"
done
for bin in hawier hawier-ui hawier-device; do
	[ -x "target/debug/$bin" ] || die "no binary at target/debug/$bin (make screenshots builds it)"
done
for module in hawier_ext_fs hawier_ext_rollout_classic; do
	[ -s "$EXT/$module.wasm" ] || die "no extension module $EXT/$module.wasm (make ext)"
done

rm -rf "$RUN_DIR"
mkdir -p "$RUN_DIR/artifacts" "$RUN_DIR/out" "$RUN_DIR/logs"

# --- teardown, whatever happens -------------------------------------------------

NODE_PID=
CONSOLE_PID=
FLEET_PIDS=()
STARTED_POSTGRES=0
cleanup() {
	local status=$?
	trap - EXIT INT TERM
	set +e
	for pid in "${FLEET_PIDS[@]}" "$CONSOLE_PID" "$NODE_PID"; do
		[ -n "$pid" ] && kill "$pid" 2>/dev/null
	done
	wait 2>/dev/null
	$COMPOSE exec -T postgres psql -q -U hawkbit -d hawkbit \
		-c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" >/dev/null 2>&1
	# Only a PostgreSQL this run started is stopped: the compose stand may be
	# running beside it.
	[ "$STARTED_POSTGRES" = 1 ] && $COMPOSE stop postgres >/dev/null 2>&1
	[ "$status" -ne 0 ] && say "logs are in $RUN_DIR/logs; $OUT_DIR is unchanged"
	exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# --- waiting --------------------------------------------------------------------

# wait_for <seconds> <what> <command…> — run the command until it succeeds.
# The node and the console, once started, are checked on every pass: one that
# died fails the run at once, with its log, instead of after the deadline.
wait_for() {
	local seconds=$1 what=$2 deadline
	shift 2
	deadline=$((SECONDS + seconds))
	until "$@"; do
		check_alive
		[ "$SECONDS" -lt "$deadline" ] || die "gave up after ${seconds}s waiting for $what"
		sleep 1
	done
}

check_alive() {
	if [ -n "$NODE_PID" ] && ! kill -0 "$NODE_PID" 2>/dev/null; then
		tail -n 30 "$RUN_DIR/logs/node.log" >&2
		die "the node exited (log tail above)"
	fi
	if [ -n "$CONSOLE_PID" ] && ! kill -0 "$CONSOLE_PID" 2>/dev/null; then
		tail -n 30 "$RUN_DIR/logs/console.log" >&2
		die "the console exited (log tail above)"
	fi
}

# --- management API -------------------------------------------------------------

# api METHOD PATH [JSON] — prints the body; a non-2xx answer fails the run.
api() {
	local method=$1 path=$2 body=${3:-} out status
	out=$(mktemp)
	status=$(curl -s -o "$out" -w '%{http_code}' -u "$AUTH" -X "$method" \
		-H 'Content-Type: application/json' ${body:+-d "$body"} "$NODE$path")
	case "$status" in
		2??) cat "$out"; rm -f "$out" ;;
		*) die "$method $path answered $status: $(cat "$out")" ;;
	esac
}

# json EXPR — evaluate a Python expression over the JSON on stdin (as `d`).
json() { python3 -I -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

upload_artifact() { # <module id> <file name> <size KiB>
	local file="$RUN_DIR/$2" status
	head -c "$(($3 * 1024))" /dev/urandom > "$file"
	status=$(curl -s -o /dev/null -w '%{http_code}' -u "$AUTH" \
		-F "file=@$file" "$NODE/rest/v1/softwaremodules/$1/artifacts")
	[ "${status:0:1}" = 2 ] || die "upload of $2 answered $status"
}

# fleet <prefix> <count> [simulator flags…] — a simulated fleet in the background.
fleet() {
	local prefix=$1 count=$2
	shift 2
	target/debug/hawier-device --url "$NODE" --controller-id "$prefix" --gateway-token "$GATEWAY_KEY" \
		--count "$count" --poll-override 1s "$@" >> "$RUN_DIR/logs/fleet.log" 2>&1 &
	FLEET_PIDS+=("$!")
}

# --- database, node, console ------------------------------------------------------

if ! $COMPOSE exec -T postgres pg_isready -q -U hawkbit >/dev/null 2>&1; then
	$COMPOSE up -d postgres >/dev/null
	STARTED_POSTGRES=1
fi
pg_ready() { $COMPOSE exec -T postgres pg_isready -q -U hawkbit >/dev/null 2>&1; }
wait_for 60 "PostgreSQL" pg_ready
$COMPOSE exec -T postgres psql -q -U hawkbit -d hawkbit \
	-c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" -c "CREATE DATABASE $DB" >/dev/null

artifacts=$(realpath "$RUN_DIR/artifacts")
HAWIER_ROLE=control \
HAWIER_BIND_ADDR="127.0.0.1:$NODE_PORT" \
HAWIER_DATABASE_URL="postgres://hawkbit:hawkbit@localhost:5432/$DB" \
HAWIER_INTERNAL_TOKEN="$INTERNAL_TOKEN" \
HAWIER_ARTIFACT_DIR="$artifacts" \
HAWIER_EXT_FS_ROOT="$artifacts" \
HAWIER_EXT_STORAGE="$(realpath "$EXT/hawier_ext_fs.wasm")" \
HAWIER_EXT_ROLLOUT="$(realpath "$EXT/hawier_ext_rollout_classic.wasm")" \
	target/debug/hawier > "$RUN_DIR/logs/node.log" 2>&1 &
NODE_PID=$!
node_up() { curl -sf -o /dev/null -u "$AUTH" "$NODE/rest/v1/system/configs"; }
wait_for 60 "the node to answer" node_up

HAWKBIT_MGMT_URL="$NODE" \
HAWIER_API_URL="$NODE" \
HAWIER_INTERNAL_TOKEN="$INTERNAL_TOKEN" \
HAWIER_BIND_ADDR="127.0.0.1:$CONSOLE_PORT" \
HAWIER_STATIC_DIR=static \
	target/debug/hawier-ui > "$RUN_DIR/logs/console.log" 2>&1 &
CONSOLE_PID=$!
console_up() { curl -sf -o /dev/null "$CONSOLE/api/health"; }
wait_for 60 "the console to answer" console_up
say "node on $NODE, console on $CONSOLE"

# --- demo data --------------------------------------------------------------------

api PUT /rest/v1/system/configs "{\"pollingTime\":\"00:00:05\",
	\"authentication.gatewaytoken.enabled\":true,
	\"authentication.gatewaytoken.key\":\"$GATEWAY_KEY\"}" >/dev/null

tags=$(api POST /rest/v1/targettags '[{"name":"field","description":"Units installed on site"},
	{"name":"lab","description":"Bench units"}]')
field_tag=$(json "d[0]['id']" <<<"$tags")
lab_tag=$(json "d[1]['id']" <<<"$tags")
os_type=$(api GET '/rest/v1/distributionsettypes?q=key==os' | json "d['content'][0]['id']")
types=$(api POST /rest/v1/targettypes "[{\"name\":\"edge-gateway\",\"key\":\"edge-gateway\",\"description\":\"x86 edge gateway\",
	\"compatibledistributionsettypes\":[{\"id\":$os_type}]},
	{\"name\":\"kiosk\",\"key\":\"kiosk\",\"description\":\"Self-service kiosk\",
	\"compatibledistributionsettypes\":[{\"id\":$os_type}]}]")
edge_type=$(json "d[0]['id']" <<<"$types")
kiosk_type=$(json "d[1]['id']" <<<"$types")

declare -A DS=()
for version in 2.4.0 2.5.0; do
	module=$(api POST /rest/v1/softwaremodules \
		"[{\"name\":\"edge-os\",\"version\":\"$version\",\"type\":\"os\",\"vendor\":\"Example Devices\"}]" \
		| json "d[0]['id']")
	upload_artifact "$module" "edge-os-$version.swu" 256
	DS[$version]=$(api POST /rest/v1/distributionsets \
		"[{\"name\":\"edge-os\",\"version\":\"$version\",\"type\":\"os\",\"modules\":[{\"id\":$module}]}]" \
		| json "d[0]['id']")
done

# Devices that answer, devices whose installs fail, and targets created
# through Mgmt that never poll: the last ones keep the second rollout in
# progress however fast the fleet is.
fleet edge 36 --attributes hw=rev2,site=north
fleet kiosk 4 --fail-download --attributes hw=rev1,site=south
depots=""
for n in 1 2 3 4 5 6; do
	depots+="${depots:+,}{\"controllerId\":\"depot-$n\",\"name\":\"depot-$n\",\"description\":\"Warehouse gateway\"}"
done
api POST /rest/v1/targets "[$depots]" >/dev/null

registered() {
	[ "$(api GET '/rest/v1/targets?limit=1' | json "d['total']")" -ge 46 ]
}
wait_for 60 "the fleet to register" registered

# typed_and_tagged <prefix> <count> <type id> <tag id>
typed_and_tagged() {
	local ids=""
	for ((n = 0; n < $2; n++)); do
		api POST "/rest/v1/targets/$1-$n/targettype" "{\"id\":$3}" >/dev/null
		ids+="${ids:+,}\"$1-$n\""
	done
	api POST "/rest/v1/targettags/$4/assigned" "[$ids]" >/dev/null
}
typed_and_tagged edge 36 "$edge_type" "$field_tag"
typed_and_tagged kiosk 4 "$kiosk_type" "$lab_tag"

rollout_is() { [ "$(api GET "/rest/v1/rollouts/$1" | json "d['status']")" = "$2" ]; }

# rollout <name> <ds id> <filter> <groups> <success %> — create, wait for ready,
# start; prints the id.
rollout() {
	local id
	id=$(api POST /rest/v1/rollouts "{\"name\":\"$1\",\"distributionSetId\":$2,
		\"targetFilterQuery\":\"$3\",\"amountGroups\":$4,
		\"successCondition\":{\"condition\":\"THRESHOLD\",\"expression\":\"$5\"},
		\"errorCondition\":{\"condition\":\"THRESHOLD\",\"expression\":\"80\"}}" | json "d['id']")
	wait_for 60 "rollout '$1' to become ready" rollout_is "$id" ready
	api POST "/rest/v1/rollouts/$id/start" '{}' >/dev/null
	echo "$id"
}

first=$(rollout "edge-os 2.4.0 fleet-wide" "${DS[2.4.0]}" "controllerid==edge-*,controllerid==kiosk-*" 2 50)
wait_for 180 "the first rollout to finish" rollout_is "$first" finished

# A group is judged finished at its success threshold; at 100% the group
# holding the depots cannot get there, so the rollout stays RUNNING.
second=$(rollout "edge-os 2.5.0 staged" "${DS[2.5.0]}" "controllerid==edge-*,controllerid==depot-*" 3 100)
second_moving() {
	[ "$(api GET "/rest/v1/rollouts/$second" | json "d['totalTargetsPerStatus']['finished']")" -ge 10 ]
}
wait_for 180 "the second rollout to make progress" second_moving
say "seeded: rollouts $first (finished) and $second (in progress)"

# --- capture ------------------------------------------------------------------------

# The one list of what the README shows: name, then console path.
PAGES=(
	overview /
	rollout "/rollouts/$second"
	targets /targets
	target /targets/edge-0
	distribution-set "/distribution-sets/${DS[2.5.0]}"
)

if [ -z "$($ENGINE images -q "$BROWSER_IMAGE")" ]; then
	say "pulling $BROWSER_IMAGE"
	$ENGINE pull "$BROWSER_IMAGE" >/dev/null
fi
# Files written in the container are handed to whoever owns the mounted
# directory as the container sees it: the caller under docker, root (which
# is the caller) under rootless podman. A plain --user would be wrong for
# one of the two.
mkdir -p "$PY_DIR"
if [ ! -d "$PY_DIR/${PY_PACKAGES[0]/==/-}.dist-info" ]; then
	say "installing ${PY_PACKAGES[*]} into $PY_DIR"
	# shellcheck disable=SC2016 # expanded by the shell in the container
	$ENGINE run --rm --network=host -v "$(realpath "$PY_DIR"):/py" "$BROWSER_IMAGE" \
		sh -c 'pip install -q --disable-pip-version-check --root-user-action=ignore --no-cache-dir --no-deps --target /py "$@" &&
			chown -R --reference=/py /py' sh "${PY_PACKAGES[@]}"
fi
# shellcheck disable=SC2016 # expanded by the shell in the container
$ENGINE run --rm --network=host -e PYTHONPATH=/py \
	-v "$(realpath "$PY_DIR"):/py:ro" \
	-v "$PWD/scripts/screenshots.py:/work/screenshots.py:ro" \
	-v "$(realpath "$RUN_DIR/out"):/out" \
	"$BROWSER_IMAGE" sh -c 'python3 /work/screenshots.py "$@"; status=$?; chown -R --reference=/out /out; exit $status' \
	sh "$CONSOLE" /out "${PAGES[@]}"

for ((i = 0; i < ${#PAGES[@]}; i += 2)); do
	file="$RUN_DIR/out/${PAGES[i]}.png"
	[ -s "$file" ] || die "page '${PAGES[i]}' produced no picture"
	size=$(wc -c < "$file")
	[ "$size" -le "$MAX_BYTES" ] || die "page '${PAGES[i]}' is $size bytes, over the $MAX_BYTES-byte cap"
done

mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR"/*.png
for ((i = 0; i < ${#PAGES[@]}; i += 2)); do
	cp "$RUN_DIR/out/${PAGES[i]}.png" "$OUT_DIR/"
done
say "wrote $((${#PAGES[@]} / 2)) pictures to $OUT_DIR"
