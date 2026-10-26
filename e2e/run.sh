#!/usr/bin/env bash
# End-to-end run of hawier against SWUpdate's suricatta in QEMU
# (spec: device-e2e). Entry point: `make e2e` (one scenario: E2E_ONLY=<name>).
#
# It brings up the stand's PostgreSQL, a dev hawier node in the hardened
# profile on a database of its own, and QEMU devices from target/e2e/out;
# runs the scenarios; and takes everything down again, pass or fail.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=e2e/lib.sh
. e2e/lib.sh

ENGINE=${ENGINE:-$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)}
COMPOSE="$ENGINE compose -f deploy/docker-compose.yml"
DB=hawier_e2e
EXT=target/wasm32-wasip1/debug
SCENARIOS=(incompatible-assignment update bad-signature wrong-revision rollout fallback)

# --- preconditions ------------------------------------------------------------

for tool in qemu-system-x86_64 qemu-img curl python3; do
	command -v "$tool" >/dev/null 2>&1 || die "$tool is required and was not found"
done
[ -s "$E2E_DIR/out/disk.img" ] || die "no reference image in $E2E_DIR/out (make e2e-image)"
[ -x target/debug/hawier ] || die "no node binary at target/debug/hawier (cargo build -p hawier-server)"
for module in hawier_ext_fs hawier_ext_rollout_bucket; do
	[ -s "$EXT/$module.wasm" ] || die "no extension module $EXT/$module.wasm (make ext)"
done
# E2E_ACCEL=tcg forces software emulation on a KVM host: the way to check the
# fallback without taking /dev/kvm away.
if [ "${E2E_ACCEL:-}" != tcg ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
	E2E_ACCEL=kvm
else
	E2E_ACCEL=tcg
	E2E_FACTOR=6
	say "software emulation (no usable /dev/kvm, or E2E_ACCEL=tcg): deadlines x$E2E_FACTOR"
fi
say "QEMU $(qemu-system-x86_64 --version | head -n 1 | awk '{ print $4 }'), accel $E2E_ACCEL"

if [ -n "${E2E_ONLY:-}" ]; then
	[[ " ${SCENARIOS[*]} " == *" $E2E_ONLY "* ]] || die "unknown scenario '$E2E_ONLY' (have: ${SCENARIOS[*]})"
	SCENARIOS=("$E2E_ONLY")
fi

rm -rf "$E2E_RUN_DIR" "$E2E_LOGS"
mkdir -p "$E2E_RUN_DIR/artifacts" "$E2E_LOGS"
: > "$E2E_REPORT"

# --- teardown, whatever happens -------------------------------------------------

E2E_NODE_PID=
cleanup() {
	local status=$?
	trap - EXIT INT TERM
	devices_stop_all
	if [ -n "$E2E_NODE_PID" ]; then
		kill "$E2E_NODE_PID" 2>/dev/null || true
		wait "$E2E_NODE_PID" 2>/dev/null || true
	fi
	$COMPOSE exec -T postgres psql -q -U hawkbit -d hawkbit \
		-c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" >/dev/null 2>&1 || true
	$COMPOSE stop postgres >/dev/null 2>&1 || true
	if [ "$status" -ne 0 ]; then
		say "logs are in $E2E_LOGS, the report in $E2E_REPORT"
	fi
	exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# --- database and node -------------------------------------------------------------

$COMPOSE up -d postgres >/dev/null
pg_ready() { $COMPOSE exec -T postgres pg_isready -q -U hawkbit; }
wait_for 60 "PostgreSQL" pg_ready
$COMPOSE exec -T postgres psql -q -U hawkbit -d hawkbit \
	-c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" -c "CREATE DATABASE $DB" >/dev/null

# The hardened profile (EXTENSIONS.md §5): no classic rollout module, a
# non-classic strategy, and — set below — typed targets only.
artifacts=$(realpath "$E2E_RUN_DIR/artifacts")
HAWIER_ROLE=control \
HAWIER_BIND_ADDR="127.0.0.1:$E2E_NODE_PORT" \
HAWIER_DATABASE_URL="postgres://hawkbit:hawkbit@localhost:5432/$DB" \
HAWIER_ARTIFACT_DIR="$artifacts" \
HAWIER_EXT_FS_ROOT="$artifacts" \
HAWIER_EXT_STORAGE="$(realpath "$EXT/hawier_ext_fs.wasm")" \
HAWIER_EXT_ROLLOUT="$(realpath "$EXT/hawier_ext_rollout_bucket.wasm")" \
	target/debug/hawier > "$E2E_LOGS/node.log" 2>&1 &
E2E_NODE_PID=$!
node_up() { curl -sf --noproxy '*' -o /dev/null -u "$E2E_AUTH" "$E2E_NODE/rest/v1/system/configs"; }
wait_for 60 "the node to answer" node_up
say "node up on $E2E_NODE (pid $E2E_NODE_PID)"

api_ok PUT /rest/v1/system/configs '{"pollingTime":"00:00:05","target.type.required":true}' >/dev/null

# --- hardened preflight: ask the node itself ---------------------------------------

inventory=$(api_ok GET /rest/v1/system/extensions)
if grep -q rollout_classic <<<"$inventory"; then
	die "the node under test has the classic rollout extension loaded; the hardened profile runs without it"
fi
grep -q rollout_bucket <<<"$inventory" || die "the node under test has no non-classic rollout strategy loaded"
required=$(api_ok GET /rest/v1/system/configs | json "d['target.type.required']['value']")
[ "$required" = True ] || die "target.type.required is not on for the tenant under test (got $required)"
say "hardened profile confirmed: no classic rollout, bucket strategy, typed targets only"

# --- one type pair per hardware revision -------------------------------------------

# Read by the scenarios, which are sourced at run time.
declare -A TARGET_TYPE=()
for rev in rev1 rev2; do
	ds_type=$(api_ok POST /rest/v1/distributionsettypes \
		"[{\"name\":\"os-$rev\",\"key\":\"os-$rev\",\"optionalmodules\":[{\"id\":$(api_ok GET '/rest/v1/softwaremoduletypes?q=key==os' | json "d['content'][0]['id']")}]}]" \
		| json 'd[0]["id"]')
	# shellcheck disable=SC2034 # read by the scenarios
	TARGET_TYPE[$rev]=$(api_ok POST /rest/v1/targettypes \
		"[{\"name\":\"board-$rev\",\"key\":\"board-$rev\",\"compatibledistributionsettypes\":[{\"id\":$ds_type}]}]" \
		| json 'd[0]["id"]')
done
export E2E_ACCEL E2E_FACTOR E2E_NODE_PID

# --- scenarios ------------------------------------------------------------------------

failed=0
for scenario in "${SCENARIOS[@]}"; do
	say "--- $scenario"
	start=$SECONDS
	# A subshell per scenario: a failing wait ends the scenario, not the run,
	# and the scenario's own devices go down with it.
	if (
		trap scenario_teardown EXIT
		# shellcheck source=/dev/null
		. "e2e/scenarios/$scenario.sh"
		"scenario_${scenario//-/_}"
	); then
		result=PASS
	else
		result=FAIL
		failed=1
	fi
	printf '%-26s %s %4ds\n' "$scenario" "$result" $((SECONDS - start)) | tee -a "$E2E_REPORT"
done

leftover=$(pgrep -f "qemu-system-x86_64.*$E2E_RUN_DIR" || true)
[ -z "$leftover" ] || die "QEMU processes left behind: $leftover"
exit "$failed"
