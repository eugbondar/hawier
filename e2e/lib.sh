# shellcheck shell=bash
# Shared pieces of the end-to-end harness (spec: device-e2e). Sourced by
# e2e/run.sh; every scenario in e2e/scenarios/ uses only what is here.
#
# Two rules every wait follows (CLAUDE.md, "waits must be able to give up"):
# it has a deadline, and in the same loop it checks that every QEMU process
# it depends on is still alive — a device that died fails the scenario at
# once, not after the deadline.

E2E_DIR=${E2E_DIR:-target/e2e}
E2E_RUN_DIR="$E2E_DIR/run"
E2E_LOGS="$E2E_DIR/logs"
# shellcheck disable=SC2034 # used by run.sh
E2E_REPORT="$E2E_DIR/report.txt"
E2E_NODE_PORT=${E2E_NODE_PORT:-18080}
E2E_NODE="http://127.0.0.1:$E2E_NODE_PORT"
E2E_TENANT=DEFAULT
E2E_AUTH=${E2E_AUTH:-admin:admin}
# Where a device finds the node: QEMU forwards this one address, and nothing
# else, to the node (restrict=on).
E2E_DEVICE_URL="http://10.0.2.100"
# Deadlines are written for KVM and multiplied under software emulation.
E2E_FACTOR=1
E2E_POLL_S=2

declare -A E2E_QEMU_PID=()

say() { echo "e2e: $*"; }
die() { echo "e2e: FAIL: $*" >&2; exit 1; }

# --- management API -----------------------------------------------------------

# api METHOD PATH [JSON] — prints the body. The HTTP status goes to a file,
# not a variable: callers usually take the body with $(…), which runs this in
# a subshell, and a variable set there would be lost. Read it with api_status.
api() {
	local method=$1 path=$2 body=${3:-} out status
	out=$(mktemp)
	if [ -n "$body" ]; then
		status=$(curl -s --noproxy '*' -o "$out" -w '%{http_code}' -u "$E2E_AUTH" \
			-X "$method" -H 'Content-Type: application/json' -d "$body" "$E2E_NODE$path")
	else
		status=$(curl -s --noproxy '*' -o "$out" -w '%{http_code}' -u "$E2E_AUTH" \
			-X "$method" "$E2E_NODE$path")
	fi
	echo "$status" > "$E2E_RUN_DIR/api.status"
	cat "$out"
	rm -f "$out"
}

api_status() { cat "$E2E_RUN_DIR/api.status"; }

# api_ok METHOD PATH [JSON] — as api, but a non-2xx answer fails the run.
api_ok() {
	local body status
	body=$(api "$@")
	status=$(api_status)
	case "$status" in
		2??) printf '%s' "$body" ;;
		*) die "$1 $2 answered $status: $body" ;;
	esac
}

# json EXPR — evaluate a Python expression over the JSON on stdin (as `d`).
json() { python3 -I -c "import json,sys; d=json.load(sys.stdin); v=($1); print(v if not isinstance(v,(dict,list)) else json.dumps(v))"; }

upload_artifact() { # <module id> <file>
	local out status
	out=$(mktemp)
	status=$(curl -s --noproxy '*' -o "$out" -w '%{http_code}' -u "$E2E_AUTH" \
		-F "file=@$2" "$E2E_NODE/rest/v1/softwaremodules/$1/artifacts")
	[ "${status:0:1}" = 2 ] || die "upload of $(basename "$2") answered $status: $(cat "$out")"
	rm -f "$out"
}

# --- waiting ------------------------------------------------------------------

# wait_for <seconds> <what> <command…> — run the command until it succeeds.
# Every running device is checked on each pass, as is the node.
wait_for() {
	local seconds=$1 what=$2
	shift 2
	local deadline=$((SECONDS + seconds * E2E_FACTOR)) name
	while true; do
		if "$@"; then
			return 0
		fi
		for name in "${!E2E_QEMU_PID[@]}"; do
			kill -0 "${E2E_QEMU_PID[$name]}" 2>/dev/null \
				|| die "device $name exited while waiting for $what (serial log: $E2E_LOGS/$name.serial)"
		done
		if [ -n "${E2E_NODE_PID:-}" ] && ! kill -0 "$E2E_NODE_PID" 2>/dev/null; then
			die "the node exited while waiting for $what (log: $E2E_LOGS/node.log)"
		fi
		[ "$SECONDS" -lt "$deadline" ] \
			|| die "gave up after $((seconds * E2E_FACTOR))s waiting for $what"
		sleep "$E2E_POLL_S"
	done
}

# --- devices --------------------------------------------------------------------

# device_start <name> <revision> <controller id> <target token>
# Boots a fresh copy-on-write overlay of the reference disk: every scenario
# starts from slot A at no copy cost.
device_start() {
	local name=$1 revision=$2 controller=$3 token=$4
	local disk="$E2E_RUN_DIR/$name.qcow2" identity="$E2E_RUN_DIR/$name.identity"
	rm -f "$disk"
	qemu-img create -q -f qcow2 -F raw -b "$(realpath "$E2E_DIR/out/disk.img")" "$disk"
	printf 'url=%s\ntenant=%s\ncontroller_id=%s\ntarget_token=%s\n' \
		"$E2E_DEVICE_URL" "$E2E_TENANT" "$controller" "$token" > "$identity"
	qemu-system-x86_64 \
		-accel "$E2E_ACCEL" -m 192 -smp 1 \
		-display none -monitor none \
		-serial "file:$E2E_LOGS/$name.serial" \
		-drive "file=$disk,if=virtio,format=qcow2" \
		-netdev "user,id=net0,restrict=on,guestfwd=tcp:10.0.2.100:80-cmd:python3 -I $(realpath e2e/relay.py) 127.0.0.1 $E2E_NODE_PORT" \
		-device virtio-net-pci,netdev=net0 \
		-fw_cfg "name=opt/org.hawier/hwrevision,string=$revision" \
		-fw_cfg "name=opt/org.hawier/identity,file=$identity" \
		> "$E2E_LOGS/$name.qemu" 2>&1 &
	E2E_QEMU_PID[$name]=$!
	say "device $name ($revision) booting, controller $controller"
}

device_stop() { # <name>
	local pid=${E2E_QEMU_PID[$1]:-}
	unset "E2E_QEMU_PID[$1]"
	[ -n "$pid" ] || return 0
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
}

devices_stop_all() {
	local name
	for name in "${!E2E_QEMU_PID[@]}"; do
		device_stop "$name"
	done
}

serial_has() { # <name> <fixed string>
	grep -qF -- "$2" "$E2E_LOGS/$1.serial" 2>/dev/null
}

# --- fleet objects ----------------------------------------------------------------

# provision_target <controller id> [target type id] — creates the target the
# device will authenticate as, before it boots. Sets $TOKEN.
TOKEN=
E2E_TARGETS=()
provision_target() {
	local controller=$1 type=${2:-} typed=
	TOKEN=$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')
	[ -z "$type" ] || typed=",\"targetType\":$type"
	api_ok POST /rest/v1/targets \
		"[{\"controllerId\":\"$controller\",\"name\":\"$controller\",\"securityToken\":\"$TOKEN\"$typed}]" >/dev/null
	E2E_TARGETS+=("$controller")
}

# Ask a device to report its attributes again on its next poll — what an
# operator does after an update to see what the device now says it runs.
request_attributes() { # <controller id>
	api_ok PUT "/rest/v1/targets/$1" '{"requestAttributes":true}' >/dev/null
}

# What a failed scenario leaves behind: every device's serial log is already
# in $E2E_LOGS; add each target's action history.
scenario_teardown() {
	local controller
	for controller in "${E2E_TARGETS[@]}"; do
		collect_action_history "$controller"
	done
	devices_stop_all
}

# distribution_set <name> <ds type key> <swu file> — prints the set id.
distribution_set() {
	local name=$1 type=$2 swu=$3 module
	module=$(api_ok POST /rest/v1/softwaremodules \
		"[{\"name\":\"$name\",\"version\":\"2\",\"type\":\"os\"}]" | json 'd[0]["id"]')
	upload_artifact "$module" "$swu"
	api_ok POST /rest/v1/distributionsets \
		"[{\"name\":\"$name\",\"version\":\"2\",\"type\":\"$type\",\"modules\":[{\"id\":$module}]}]" \
		| json 'd[0]["id"]'
}

assign() { # <controller id> <set id> — prints the body; status via api_status
	api POST "/rest/v1/targets/$1/assignedDS" "[{\"id\":$2,\"type\":\"forced\"}]"
}

attribute() { # <controller id> <key>
	api GET "/rest/v1/targets/$1/attributes" | json "d.get('$2', '')"
}

has_polled() { # <controller id>
	[ "$(api GET "/rest/v1/targets/$1" | json "d.get('lastControllerRequestAt') is not None")" = True ]
}

attribute_is() { [ "$(attribute "$1" "$2")" = "$3" ]; }

# True once the device has answered a request_attributes with fresh ones.
has_requested_attributes_cleared() { # <controller id>
	[ "$(api GET "/rest/v1/targets/$1" | json "d.get('requestAttributes')")" = False ]
}

# Latest action of the target: "<id> <status> <detailStatus>".
latest_action() {
	api GET "/rest/v1/targets/$1/actions?sort=id:DESC&limit=1" \
		| json "' '.join(str(a.get(k)) for a in d['content'][:1] for k in ('id','status','detailStatus')) or 'none'"
}

action_detail_is() { # <controller id> <detailStatus>
	[ "$(latest_action "$1" | cut -d' ' -f3)" = "$2" ]
}

action_count() { # <controller id>
	api GET "/rest/v1/targets/$1/actions" | json "d['total']"
}

# Keep what a failure needs to be read without rerunning it.
collect_action_history() { # <controller id>
	api GET "/rest/v1/targets/$1/actions?sort=id:DESC" > "$E2E_LOGS/$1.actions.json" 2>/dev/null || true
	local id
	for id in $(json "' '.join(str(a['id']) for a in d['content'])" < "$E2E_LOGS/$1.actions.json" 2>/dev/null); do
		api GET "/rest/v1/targets/$1/actions/$id/status?limit=100" \
			> "$E2E_LOGS/$1.action-$id.status.json" 2>/dev/null || true
	done
}
