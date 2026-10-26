# shellcheck shell=bash
# A rollout whose filter matches boards of both revisions reaches only the
# ones its set fits; the other board is never offered anything.
#
# The hardened node has no classic module, so the rollout names its strategy:
# without one it asks for 'default', which the bucket module refuses, and the
# rollout stays CREATING. The bucket module's own defaults are enough for a
# two-board fleet: its final wave sweeps up whatever the windows missed.

scenario_rollout() {
	local one=e2e-ro-rev1 two=e2e-ro-rev2 ds rollout total
	provision_target "$one" "${TARGET_TYPE[rev1]}"
	local token_one=$TOKEN
	provision_target "$two" "${TARGET_TYPE[rev2]}"
	device_start ro-rev1 rev1 "$one" "$token_one"
	device_start ro-rev2 rev2 "$two" "$TOKEN"
	wait_for 120 "$one to report" attribute_is "$one" slot a
	wait_for 120 "$two to report" attribute_is "$two" slot a

	ds=$(distribution_set e2e-ro-v2 os-rev1 "$E2E_DIR/out/update-rev1.swu")
	rollout=$(api_ok POST /rest/v1/rollouts \
		"{\"name\":\"e2e-mixed\",\"distributionSetId\":$ds,\"targetFilterQuery\":\"controllerid==e2e-ro-*\",\"amountGroups\":1,\"strategy\":\"bucket\"}" \
		| json 'd["id"]')
	rollout_is() { [ "$(api GET "/rest/v1/rollouts/$rollout" | json "d['status']")" = "$1" ]; }
	wait_for 60 "the rollout to be ready" rollout_is ready
	total=$(api GET "/rest/v1/rollouts/$rollout" | json "d['totalTargets']")
	[ "$total" = 1 ] || die "the rollout counts $total targets, expected 1"

	api_ok POST "/rest/v1/rollouts/$rollout/start" >/dev/null
	wait_for 300 "the update of $one to finish" action_detail_is "$one" finished
	[ "$(action_count "$two")" = 0 ] || die "$two was offered the rev1 set"
	serial_has ro-rev2 "Update available" && die "$two saw a deployment"
	return 0
}
