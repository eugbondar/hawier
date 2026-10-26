# shellcheck shell=bash
# An update signed with a key the device does not trust is refused before a
# byte reaches a slot.

scenario_bad_signature() {
	local id=e2e-badsig ds
	provision_target "$id" "${TARGET_TYPE[rev1]}"
	device_start badsig rev1 "$id" "$TOKEN"
	wait_for 120 "$id to report its attributes" attribute_is "$id" slot a

	ds=$(distribution_set e2e-badsig-v2 os-rev1 "$E2E_DIR/out/update-rev1-foreign.swu")
	assign "$id" "$ds" >/dev/null
	[ "$(api_status)" = 200 ] || die "assignment answered $(api_status)"

	wait_for 180 "the update of $id to fail" action_detail_is "$id" error
	request_attributes "$id"
	wait_for 60 "$id to report again" has_requested_attributes_cleared "$id"
	[ "$(attribute "$id" slot)" = a ] || die "$id moved to slot $(attribute "$id" slot)"
	[ "$(attribute "$id" version)" = 1 ] || die "$id runs version $(attribute "$id" version)"
	serial_has badsig "rsa_verify.c : verify_final" || die "the serial log shows no signature refusal"
	serial_has badsig "slot b" && die "the device booted slot b"
	return 0
}
