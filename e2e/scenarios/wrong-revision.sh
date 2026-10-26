# shellcheck shell=bash
# A validly signed update that declares another hardware revision — a
# mislabelled set, which the server cannot see — is refused by the device.

scenario_wrong_revision() {
	local id=e2e-wrongrev ds
	provision_target "$id" "${TARGET_TYPE[rev1]}"
	device_start wrongrev rev1 "$id" "$TOKEN"
	wait_for 120 "$id to report its attributes" attribute_is "$id" slot a

	ds=$(distribution_set e2e-wrongrev-v2 os-rev1 "$E2E_DIR/out/update-rev2-label.swu")
	assign "$id" "$ds" >/dev/null
	[ "$(api_status)" = 200 ] || die "assignment answered $(api_status)"

	wait_for 180 "the update of $id to fail" action_detail_is "$id" error
	request_attributes "$id"
	wait_for 60 "$id to report again" has_requested_attributes_cleared "$id"
	[ "$(attribute "$id" slot)" = a ] || die "$id moved to slot $(attribute "$id" slot)"
	serial_has wrongrev "rev2" || die "the serial log shows no hardware-compatibility refusal"
	serial_has wrongrev "slot b" && die "the device booted slot b"
	return 0
}
