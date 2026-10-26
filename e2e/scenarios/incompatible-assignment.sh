# shellcheck shell=bash
# The server half of "distributed strictly": a set never reaches a target
# whose type does not accept it, nor — in the hardened profile — a target
# with no type at all. No device has to boot for that.

scenario_incompatible_assignment() {
	local typed=e2e-incompat-rev1 untyped=e2e-incompat-untyped ds body
	provision_target "$typed" "${TARGET_TYPE[rev1]}"
	provision_target "$untyped"
	ds=$(distribution_set e2e-incompat-rev2 os-rev2 "$E2E_DIR/out/update-rev2-label.swu")

	body=$(assign "$typed" "$ds")
	[ "$(api_status)" = 400 ] || die "assigning a rev2 set to a rev1 target answered $(api_status): $body"
	grep -q 'target.type.incompatible' <<<"$body" || die "unexpected refusal: $body"

	body=$(assign "$untyped" "$ds")
	[ "$(api_status)" = 400 ] || die "assigning to an untyped target answered $(api_status): $body"
	grep -q 'target.type.incompatible' <<<"$body" || die "unexpected refusal: $body"

	[ "$(action_count "$typed")" = 0 ] || die "$typed has an action"
	[ "$(action_count "$untyped")" = 0 ] || die "$untyped has an action"
}
