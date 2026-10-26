# Tasks

## 1. Cross-check specs against tests

Each spec's scenarios were written from code and tests. Check the test names
below against the spec. Then confirm by `make it` (server areas) and
`make ui-audit` (console) that the cited behavior still passes. A scenario that
contradicts its test is fixed in the spec, not in the code.

- [x] 1.1 Check `ddi-api` against `it/ddi.rs`, `it/correctness.rs`, `it/maintenance.rs` and `it/errors.rs`:
  - `ddi_wrong_target_token_returns_401`, `ddi_gateway_token_toggle_and_autoregistration`, `ddi_poll_no_action_returns_polling_sleep_and_no_links`
  - `ddi_assign_ds_makes_deployment_base_link_appear_and_retrieved_written_once`, `ddi_full_update_cycle_to_finished`, `ddi_closed_failure_sets_action_and_target_error`, `ddi_cancel_flow_and_rejected_cancel`
  - `ddi_confirmation_flow_confirm_deny_and_auto_confirm`, `ddi_config_data_merge_replace_remove`, `ddi_installed_base_get_and_offline_put`, `action_status_quota_blocks_device_spam_not_operator`
  - `repeated_proceeding_is_counted_not_appended`, `multi_assignments_*`, `equal_weight_serves_oldest_action_first`
  - `maintenance::*`, `ddi_auth_failures_are_not_journaled`

  Verify that each scenario matches an assertion, or is listed in design.md item 31 as untested.
- [x] 1.2 Check `artifact-delivery` against `ddi_artifact_etag_if_match_if_none_match_and_range`, `ext::without_a_storage_extension_artifacts_answer_503`, `ext::with_a_storage_extension_a_download_works`, `ext::the_fs_extension_answers_on_its_own` and `retention::shared_content_survives_until_the_last_reference_goes`. Verify that the 302 and path-containment requirements are marked untested in item 31.
- [x] 1.3 Check `deployment-assignment` and `rollout-engine`:
  - `correctness::default_supersede_uses_canceling_handshake`, `autoclose_closes_superseded_action_without_cancel_handshake`, `implicit_lock_*`, `force_quit_under_device_feet`, `polite_device_defers_soft_update_until_forced`, `mgmt_repeat_assignment_only_allowed_with_multi_assignments`
  - `rollout::rollout_full_lifecycle_two_groups`, `rollout_error_threshold_pauses`, `rollout_approval_workflow`, `rollout_trigger_next_group_forces_advance`, `target_filter_auto_assign_picks_up_matching_target`
  - `rollout_strategy::*`, `scenarios::mixed_fleet_rollout_partial_failures`

  Verify that stop, delete, retry and dynamic groups are listed as untested.
- [x] 1.4 Check the background capabilities:
  - `action-housekeeping` against `correctness::housekeeping_deletes_expired_actions_and_sessions`
  - `artifact-retention` against `retention::*`
  - `event-delivery` against `events::*`

  Verify that the retention requirement does not claim API-created rules fire (design item 7).
- [x] 1.5 Check `extension-host` and `first-party-extensions` against `it/ext.rs`, `it/extensions_api.rs` (all 11 tests), `it/authz.rs` and the unit tests in each `apps/ext-*/src/lib.rs`. Verify by `make test` and `make it`.
- [x] 1.6 Check `mgmt-api`, `rsql-query`, `access-control`, `incidents`, `tenant-configuration`, `database-schema` and `ui-state-api` against:
  - `it/mgmt.rs`: `mgmt_target_crud_array_create_rsql_pagination`, `mgmt_console_contract`, `sort_parity_with_ui_columns`, `poll_state_*`, `mgmt_sm_multipart_upload_hash_verify_and_download`, `mgmt_system_configs_single_and_batch`
  - `rsql.rs` unit tests
  - `it/authz.rs`
  - `it/errors.rs`
  - `it/phase1.rs`
  - `it/ui_state.rs`
- [x] 1.7 Check `node-roles` against `rollout::failover_control_node_dies_sub_keeps_devices_updating` and `extensions_api::a_sub_node_reports_but_refuses_to_write`.
- [x] 1.8 Check `device-simulator` against the client unit tests (`feedback_href_strips_query_and_appends_feedback`, `backoff_tests::*`, `parses_polling_sleep`), `ddi::a_device_obeys_retry_after_instead_of_hammering` and `ddi::a_silent_server_does_not_hang_the_device_forever`.
- [x] 1.9 Check the eight `console-*` specs against `apps/ui/tests/it/ui_audit.rs`, `apps/ui/tests/it/ui.rs`, `apps/ui/src/help_audit.rs` and the `help.rs`, `resources.rs` and `ui_api.rs` unit tests. Verify by `make ui-audit`, and verify that every UI rule classified as product behavior in design.md D6 maps to at least one requirement.
- [x] 1.10 Check `build-tooling`, `load-benchmark`, `remote-deployment`, `tls-proxy` and `build-identity` against the Makefiles, `scripts/*.sh`, `scripts/build_version.rs`, `deploy/ansible/**`, `ui_audit::settings_shows_the_build_of_each_component` and `extensions_api::*`. These have no automated tests, so verify by `make gc-dry` and `make deploy-list`, and by reading the role.
- [x] 1.11 Run `openspec validate baseline-existing-capabilities --strict` and confirm it reports the change valid.

## 2. OpenSpec project context

- [x] 2.1 Fill `context` in `openspec/config.yaml` with:
  - the stack: Rust 2021, axum 0.8, sqlx without ORM, askama, wasm32-wasip1 extensions, PostgreSQL only
  - the monorepo layout and binaries
  - the rule that specs are written in English while project prose stays Russian
  - "code wins; a divergence goes to the catalogue or backlog, not into a SHALL"
  - where the backlog lives (`API_NOTES.md`, `SCHEMA_NOTES.md`, the roadmap parts of `EXTENSIONS.md`)
  - the NARROW duty

  Keep the context under 51,200 bytes. Verify by `openspec context --json`.
- [x] 2.2 Add `rules` to `openspec/config.yaml`:
  - specs: no internal Rust names; hawkBit compatibility stated per surface; a new console list, column, view or bulk operation needs a manual section
  - tasks: one slice per commit, with `make lint`, plus `make ui-audit` when `apps/ui` or `static/assets` is touched
  - tasks: a hot-path change needs a `make load` comparison

  Move the "File maintenance rule" sections of EXTENSIONS.md and API_NOTES.md here. Verify that `openspec instructions specs --change baseline-existing-capabilities --json` shows the rules.

## 3. Documentation migration (per design.md D6)

- [x] 3.1 README.md:
  - keep the quick start, the make table and the environment tables
  - complete the make table: `gc`, `gc-dry`, `load`, `load-poll`, `rollback-<host>`, `ps`, `ext-release`, `clean`, `fmt`, `check-tools`
  - complete the load parameters: `DDI_URL`, `PROFILE_CHECK`, `HISTORY`, `DEVICE_BIN`, `DEVICE_LOG`, `USER_PASS`, `GATEWAY_KEY`
  - replace the deploy-internals, TLS and extension paragraphs with links to `remote-deployment`, `tls-proxy` and `extension-host`
  - fix the image-ID comparison claim, the `.env` mention, the stale `IMAGE_REPO` advice, the "compose ps -a" claim and the hardcoded `docker compose`

  Verify by `grep -n 'image ID\|\.env\|compose ps -a\|docker compose' README.md`: each remaining hit must be correct.
- [x] 3.2 ARCHITECTURE.md:
  - keep the topology, failure model, 10 Gbit path, trade-offs and measurements
  - fix the monorepo list (all 8 `apps/ext-*`), the endpoint count, the hawier table count (12), the `ui_state` description and the stale storage enum paragraph
  - add a "Specs" section linking every capability

  Verify that every relative link resolves.
- [x] 3.3 EXTENSIONS.md:
  - replace the §4 point contracts and the §5 parameter rules with links to `extension-host`, `first-party-extensions`, `event-delivery`, `artifact-retention` and `rollout-engine`
  - keep the rationale, NARROW notes, status block, remote-shell, AuthProvider, the hardened profile and §6
  - fix the stale line references, the guest-capability sentence, the failure-semantics sentence, the manifest-limits sentence and the event-sink absence

  Verify that no statement left in the file contradicts `extension-host`.
- [x] 3.4 API_NOTES.md:
  - replace §5 "IMPLEMENTED" with a short list of links into `mgmt-api` and `ddi-api`
  - shorten §2 to the doctrine and a link to `device-simulator`
  - keep §1, §3, §4, §6, the open part of §7 and §8 as backlog

  Verify that the file contains only problems, plans and doctrine.
- [x] 3.5 SCHEMA_NOTES.md: renumber #19, #19a and #20 into order, updating every `SCHEMA_NOTES #N` reference in code comments and docs. Verify by `grep -rn 'SCHEMA_NOTES #' apps libs *.md`.
- [x] 3.6 CLAUDE.md:
  - add at the top: "behavior lives in `openspec/specs/`; change it through an OpenSpec change (`/opsx:propose`)"
  - condense the ops paragraphs (engine, target downloads nothing, waits, diagnostics, preflight, TLS, profiles, debug info, gc) into one-line invariants with pointers to `build-tooling`, `remote-deployment` and `tls-proxy`
  - replace the product UI rules with a pointer to the `console-*` specs, and keep the engineering rules 4, 9, 13, 19, 26, 27 and the halves of 1 and 28 verbatim
  - link the architectural invariants to `node-roles`, `artifact-delivery` and `extension-host`
  - keep commit discipline, make-only, NARROW, `sp_*`, style and test layout unchanged
  - fix "the image ID does not change" and the verify list (also role, paramsFileWritable, console build, TLS)

  Verify that the file shrinks substantially, and that every removed rule is stated by a spec requirement.
- [x] 3.7 Verify that `apps/ui/docs/**` is untouched (`git diff --stat apps/ui/docs` is empty) and that `make ui-audit` passes, so `every_declared_thing_is_documented` and `docs_are_plain_markdown` are still green.

## 4. Route the divergence catalogue

- [x] 4.1 Add the API-surface divergences from design.md (items 16–19, 22, 23, 25, 26) to API_NOTES.md as "SHORTCOMING" entries, each with problem → plan → compatibility. Verify that each item number appears once.
- [x] 4.2 Record item 20 (the relative `HAWIER_ARTIFACT_DIR` default) and item 30 (hardcoded codes) as notes next to the README environment table and in CLAUDE.md. Verify by grep.
- [x] 4.3 List the remaining items (security 1–6, behavior 7–15, 21, 24, 27–29, test gaps 31) in the final apply report as proposed follow-up changes. Verify that the report names each one.

## 5. Commit and archive

- [x] 5.1 Commit the OpenSpec scaffold: `openspec/` with this change and the `.claude/commands/opsx` and `.claude/skills/openspec-*` integration. Run `make lint` first. The message states why: the switch to OpenSpec. Verify by `git status` showing them tracked.
- [x] 5.2 Commit the config (group 2) and the documentation migration (groups 3–4) as their own slices, staging only those files and running `make lint` and `make ui-audit` before each. Verify that `git log --stat` shows no code files in them.
- [x] 5.3 Archive the change with `/opsx:archive`. Verify that `openspec list --specs` lists all 31 capabilities, with no `TBD` Purpose.
