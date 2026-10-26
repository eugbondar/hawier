# Tasks

Prerequisite: `baseline-existing-capabilities` is archived, so that the
MODIFIED deltas here have main specs to apply to. Each numbered group ends in
one commit on `master`. Before every commit, run `make lint`. Run `make
ui-audit` as well when `apps/ui` is touched, and that includes `apps/ui/docs`.
Stage only the group's own changes.

## 1. Server: target type compatibility on assignment (hawkBit parity)

- [x] 1.1 Confirm the hawkBit 1.0 contract for incompatible assignment: errorCode, HTTP status, exceptionClass, and whether a batch request is refused as a whole. Write the result into design.md D11 if it differs.
- [x] 1.2 Add an `AppError` variant for `target.type.incompatible` (400, hawkBit exceptionClass). Verify with a unit test that the ExceptionInfo body carries `hawkbit.server.error.target.type.incompatible`.
- [x] 1.3 Add the shared compatibility predicate (design D11) and the per-request precheck in `POST /rest/v1/targets/{id}/assignedDS` and `POST /rest/v1/distributionsets/{id}/assignedTargets`. The precheck runs inside the transaction, after the target row lock and before the first action is written. Verify with new `tests/it/mgmt.rs` cases:
  - an incompatible type gets 400 and creates no action
  - a mixed batch gets 400, and neither target has an action and no set is locked
  - an untyped target succeeds
  - a compatible target succeeds
- [x] 1.4 Replace the hard-coded enum numbers in any query touched in `mgmt/targets.rs` with `codes.rs`, as CLAUDE.md requires. Verify by grep that the touched queries contain no bare status codes.
- [x] 1.5 Add the divergence to API_NOTES §9 as found and fixed by this change, and add a manual line in `apps/ui/docs/deployment/assignment.md` saying what the 400 means for an operator. Commit after 1.6.
- [x] 1.6 Show a refused assignment on the target page (spec console-objects: A refused assignment is reported on the page). Assign incompatibly from the console and check on the live page that `Assign stopped: …` is shown as a danger message and that the action log has the same error. `make ui-audit` passes.

## 2. Server: rollouts and auto-assignment respect compatibility

- [x] 2.1 AND the predicate into the host-side rollout target query (total, strategy snapshot, group membership, dynamic groups). Verify with `tests/it/rollout.rs`: a mixed fleet gives `totalTargets` 1 and only the compatible target in a group, and a late incompatible device in a dynamic rollout gets no group.
- [x] 2.2 AND the predicate into the auto-assignment target query. Verify with `tests/it/rollout.rs`: an incompatible target is skipped, the compatible one gets one action, and a second tick creates none.
- [x] 2.3 Run EXPLAIN on the rollout target query over the `make load` fleet and check that the semi-join uses the relation's key. Run `make load` and compare with `bench/load-history.csv`. A degradation above 20% at the same parameters blocks the commit. Commit.

## 3. Server: `target.type.required` (hardened)

- [x] 3.1 Add the tenant key `target.type.required` (default false) to the key list and the configs API. Verify with `tests/it`: `GET /rest/v1/system/configs` shows it as `false`, `global: true`.
- [x] 3.2 Make the predicate refuse untyped targets when the key is on. Verify with `tests/it`, using the key on:
  - an untyped assignment gets 400, with a message that the target has no type
  - a rollout skips untyped targets
  - auto-assignment skips untyped targets
- [x] 3.3 Update the hardened profile in both places together: API_NOTES §6 gets a new item (an untyped target accepts any set → hardened refuses it), and EXTENSIONS.md §5 states that hardened means no classic rollout module plus `target.type.required=true`. Update the status block in EXTENSIONS.md: the profile is now run by `make e2e`.
- [x] 3.4 Document the key in `apps/ui/docs/admin/settings.md`. Check on the live `/settings` page that the key is listed and can be saved and reset. `make ui-audit` passes, including `every_declared_thing_is_documented`. Commit.

## 4. Reference device image (Buildroot external tree)

- [x] 4.1 Create `e2e/Containerfile`, starting from the fully qualified `debian:bookworm-slim` with the Buildroot host prerequisites and ccache, and a Makefile rule that builds it with `$(ENGINE)`. The rule checks for the image by exact name and passes `--user` only for docker. Verify that `ENGINE=podman` and `ENGINE=docker` both produce a usable builder and that files in `target/e2e/` are owned by the invoking user. (Done with docker; podman is not installed on the build machine, so the podman path is unverified here.)
- [x] 4.2 Add the pinned Buildroot tarball (version and sha256) and the pinned Bootlin x86-64 toolchain (URL and sha256). Download into `target/e2e/dl`. Verify that a tampered checksum fails the download with a named message.
- [x] 4.3 Create the external tree `e2e/br2-external/`:
  - `external.desc`, `Config.in`, `external.mk`
  - `configs/hawier_qemu_defconfig`
  - a kernel `tinyconfig` fragment
  - the BusyBox config
  - the SWUpdate `.config`: suricatta hawkBit, signed images RAWRSA, GRUB, `raw` handler, no web server, no Lua
  - Verify that `make e2e-image` reaches a booting kernel in `output/images`.
- [x] 4.4 Add a rootfs overlay with these init scripts:
  - the `fw_cfg` reader, writing `/run/hwrevision` and `/run/swupdate.cfg`, which refuses to start the updater when an entry is missing
  - the read-only root, with tmpfs mounts and the data partition
  - the start of SWUpdate in suricatta mode with `-e stable,to-<inactive>`
  - Verify by booting by hand with and without `fw_cfg` entries: the serial log names a missing entry, and with both entries suricatta polls.
- [x] 4.5 Add `grub.cfg` with the trial-boot logic, written against the pinned SWUpdate's GRUB and state code (design D2), and a genimage config for the MBR, ext2 boot, slots A and B, and ext4 data. Verify that the disk boots slot A by default and slot B after setting `rootslot=b` with `grub-editenv`. (Verified instead through SWUpdate itself writing `rootslot=b` in the `update` scenario, and the fallback in `fallback`.)
- [x] 4.6 Add a post-image script that builds `slot-v1`, `slot-v2` and `slot-noinit` from one target tree, generates the `signing` and `foreign` keys when absent, and packages the four `.swu` variants (design D8). Verify that `cpio -t` lists `sw-description` and `sw-description.sig` first, and that `openssl dgst -verify` accepts each `.swu` with its intended key.
- [x] 4.7 Add the size budget check (spec: Size and time budget). Verify that it passes on the real build and fails, naming the measure, when one limit is lowered for a test.
- [x] 4.8 Wire `e2e-image` into the root Makefile, keeping it out of `all` and `build`. Confirm that `scripts/gc.sh` never scans `target/e2e` and add a comment saying so. Verify that a warm `make e2e-image` with the network disabled succeeds and changes no output. Commit.

## 5. Harness and the server-side and update scenarios

- [x] 5.1 Create `e2e/lib.sh` with:
  - the API helper (curl and python3)
  - the deadline loop that also checks QEMU pids
  - QEMU start and stop on a qcow2 overlay, with `restrict=on` and `guestfwd`
  - log collection into `target/e2e/logs/`
  - the report writer
  - the EXIT/INT/TERM trap
  - Verify that killing a QEMU process mid-wait fails within one interval and keeps its serial log.
- [x] 5.2 Create `e2e/run.sh`. It:
  - checks for QEMU, and for KVM with the TCG fallback and its deadline factor
  - starts stand PostgreSQL and creates the `hawier_e2e` database
  - starts the dev node with ext-fs and ext-rollout-bucket and without classic (design D10)
  - sets the tenant keys
  - runs the hardened preflight against the node's extension inventory and configs
  - provisions the types (one DS type per revision; each target type lists exactly one)
  - Verify that loading the classic module by hand fails the run before any device boots.
- [x] 5.3 Add `make e2e`, with `E2E_ONLY=`, depending on `e2e-image` and `ext` and building the dev node itself (design D10). Add `shellcheck` over `e2e/` to `make lint` and to `check-tools`. Verify that `make lint` fails on a shellcheck warning in `e2e/`. (shellcheck is not installed on the build machine; verified through a wrapper running Debian's shellcheck 0.9.0 in a container.)
- [x] 5.4 Add the `update` scenario. Verify it passes:
  - the action is FINISHED
  - the installed set is v2
  - attributes `slot=b`, `version=2`, `hw_revision=rev1`
  - the serial log shows the reboot into slot B
- [x] 5.5 Each suricatta incompatibility found while getting 5.4 green gets one of two treatments. A small, clear fix goes into its own commit with its `tests/it/ddi.rs` case, ahead of the scenario commit. A larger one gets an API_NOTES entry and a separate change. The harness never works around it (design D13). Verify that the commit log contains no harness-side URL rewrite or relaxed SWUpdate option.
- [x] 5.6 Add the `incompatible-assignment` and `rollout` scenarios. Verify:
  - both assignment calls get 400 and neither target has an action
  - the rollout has `totalTargets` 1
  - the `rev2` device's poll never carries `deploymentBase` during the run
  - the `rev1` device ends FINISHED
  - record the bucket parameters used (design open question): the bucket defaults, with `strategy: bucket` named explicitly
- [x] 5.7 Run `make e2e` locally from a clean `target/e2e/run`. Verify that everything passes, `report.txt` lists each scenario with its duration, and no qemu, hawier or postgres process is left running. Commit.

## 6. Device-side refusal and fallback scenarios

- [x] 6.1 Add the `bad-signature` scenario. Verify that the action is ERROR, the attributes stay `slot=a`, `version=1`, and the serial log shows SWUpdate's signature error.
- [x] 6.2 Add the `wrong-revision` scenario. Verify that the action is ERROR, the slot is unchanged, and the serial log shows the hardware-compatibility refusal.
- [x] 6.3 Add the `fallback` scenario. Verify that the device reboots, panics in slot B, comes back on slot A, and reports failure: the action is ERROR and the attribute is `slot=a`.
- [x] 6.4 Run `make e2e` with all six scenarios. Run it again with KVM hidden (TCG; `E2E_ACCEL=tcg`, since hiding /dev/kvm needs root). Verify that both pass and that the TCG run says which mode it used. Commit.

## 7. CI workflow

- [x] 7.1 Add `.github/workflows/e2e.yml` (design D12) with:
  - `workflow_dispatch` and path-filtered push/pull_request triggers
  - the KVM udev step
  - installation of `qemu-system-x86` and `shellcheck`
  - SHA-pinned actions
  - the dl, ccache and cargo caches
  - `make e2e-image` and `make e2e` as the only build steps
  - a 60-minute cap
  - report and log artifacts
  - Verify with `actionlint`, if available, and by checking that every `run:` step calls only make or set-up commands. (actionlint is not available here; the YAML was parsed and every `run:` checked by hand.)
- [x] 7.2 Start the README section "End-to-end on a real client" (completed in 8.1) with the budget: cold and warm duration, disk use, image sizes. Mark the cold-runner time as "to be measured on the first hosted run". Commit.

## 8. Documentation and wrap-up

- [x] 8.1 Update the README:
  - the make target table gains `e2e-image` and `e2e`
  - a section "End-to-end on a real client" covers the host requirements, `E2E_ONLY`, where logs and the report go, how to boot a device by hand for debugging, and the release note about the compatibility check
  - Verify that every command in the section runs as written.
- [x] 8.2 Run `openspec validate qemu-swupdate-e2e --strict` and confirm it is clean, with no INFO lines about missing target specs once the baseline is archived. Run the full `make lint`, `make test`, `make it`, `make ui-audit` and `make e2e` and confirm all are green.
