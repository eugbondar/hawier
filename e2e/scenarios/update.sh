# shellcheck shell=bash
# A full A/B cycle on a real client: download from hawier, install into the
# inactive slot, reboot, trial boot, confirm, report.

scenario_update() {
	local id=e2e-update ds installed
	provision_target "$id" "${TARGET_TYPE[rev1]}"
	device_start update rev1 "$id" "$TOKEN"
	wait_for 120 "$id to report its attributes" attribute_is "$id" slot a
	[ "$(attribute "$id" version)" = 1 ] || die "$id runs version $(attribute "$id" version), expected 1"
	[ "$(attribute "$id" hw_revision)" = rev1 ] || die "$id reports hw_revision $(attribute "$id" hw_revision)"

	ds=$(distribution_set e2e-update-v2 os-rev1 "$E2E_DIR/out/update-rev1.swu")
	assign "$id" "$ds" >/dev/null
	[ "$(api_status)" = 200 ] || die "assignment answered $(api_status)"

	wait_for 300 "the update of $id to finish" action_detail_is "$id" finished
	installed=$(api GET "/rest/v1/targets/$id/installedDS" | json "d.get('id')")
	[ "$installed" = "$ds" ] || die "installed set is $installed, expected $ds"

	request_attributes "$id"
	wait_for 60 "$id to report slot b" attribute_is "$id" slot b
	[ "$(attribute "$id" version)" = 2 ] || die "$id runs version $(attribute "$id" version), expected 2"
	[ "$(attribute "$id" hw_revision)" = rev1 ] || die "$id lost its hardware revision"
	serial_has update "hawier: hawier-qemu rev1, slot b, version 2" \
		|| die "the serial log does not show a boot into slot b"
}
