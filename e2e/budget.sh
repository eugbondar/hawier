#!/usr/bin/env bash
# Size budget of the reference device image (spec: reference-device-image,
# "Size and time budget"). Fails naming the measure, its value and its limit.
# The limits can be lowered from the environment to check that this check
# fails (E2E_BUDGET_SLOT_MIB=1 make e2e-image).
set -euo pipefail

dir=${1:?usage: budget.sh <target/e2e>}
out="$dir/out"
slot_mib=${E2E_BUDGET_SLOT_MIB:-8}
swu_mib=${E2E_BUDGET_SWU_MIB:-8}
disk_mib=${E2E_BUDGET_DISK_MIB:-64}
tree_gib=${E2E_BUDGET_TREE_GIB:-4}

failed=0
mib() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1048576 }'; }
check() { # <measure> <bytes> <limit bytes> <limit text>
	if [ "$2" -gt "$3" ]; then
		echo "e2e-image: over budget: $1 is $(mib "$2") MiB, limit $4" >&2
		failed=1
	else
		echo "e2e-image: $1 $(mib "$2") MiB (limit $4)"
	fi
}

check "root filesystem image of one slot (slot-v1.squashfs)" \
	"$(stat -c %s "$out/slot-v1.squashfs")" $((slot_mib << 20)) "$slot_mib MiB"
for swu in "$out"/*.swu; do
	check "$(basename "$swu")" "$(stat -c %s "$swu")" $((swu_mib << 20)) "$swu_mib MiB"
done
check "raw disk image (disk.img, apparent size)" \
	"$(stat -c %s "$out/disk.img")" $((disk_mib << 20)) "$disk_mib MiB"
tree=$(du -sb "$dir/br-output" "$dir/dl" "$dir/ccache" 2>/dev/null | awk '{ s += $1 } END { print s + 0 }')
check "build tree (br-output, dl, ccache)" "$tree" $((tree_gib << 30)) "$tree_gib GiB"

exit "$failed"
