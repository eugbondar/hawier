# shellcheck shell=bash
# A signed, revision-correct update whose slot cannot start: the trial boot
# fails, the bootloader goes back to the old slot, and the device reports
# the failure. The device is never left without a slot it boots.

scenario_fallback() {
	local id=e2e-fallback ds
	provision_target "$id" "${TARGET_TYPE[rev1]}"
	device_start fallback rev1 "$id" "$TOKEN"
	wait_for 120 "$id to report its attributes" attribute_is "$id" slot a

	ds=$(distribution_set e2e-fallback-v2 os-rev1 "$E2E_DIR/out/fallback-rev1.swu")
	assign "$id" "$ds" >/dev/null
	[ "$(api_status)" = 200 ] || die "assignment answered $(api_status)"

	wait_for 300 "the update of $id to fail" action_detail_is "$id" error
	serial_has fallback "Kernel panic" || die "the serial log shows no failed trial boot"
	request_attributes "$id"
	wait_for 60 "$id to report again" has_requested_attributes_cleared "$id"
	[ "$(attribute "$id" slot)" = a ] || die "$id is on slot $(attribute "$id" slot), expected a"
	[ "$(attribute "$id" version)" = 1 ] || die "$id runs version $(attribute "$id" version)"
}
