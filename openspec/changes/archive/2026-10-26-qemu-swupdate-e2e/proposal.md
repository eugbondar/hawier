# Proposal

## Why

hawier's only DDI client today is its own simulator, `hawier-device`. It was
written against hawier, so it confirms what hawier already believes. Real
fleets talk to hawkBit through SWUpdate's suricatta. Nothing checks hawier
against suricatta, and nothing exercises a full update: download, install into
the inactive slot, reboot, report the result. The repository is about to move
to GitHub, so this check has to fit a hosted runner's time and disk budget, and
it has to be repeatable there.

The request also asks for a strict ("hardened") mode. An update is installed
only on the exact hardware revision it was built for, and the server only sends
it to targets of that revision. Half of this is missing on the server. hawier
stores target-type ↔ distribution-set-type compatibility, but it never checks it
when it assigns a set. hawkBit 1.0 does check it, so this gap is also a
compatibility divergence.

## What Changes

- **Reference device image.** A minimal Buildroot system for QEMU x86_64, built
  from an external tree in the repository with a pinned Buildroot release and
  an external toolchain. It has a SquashFS root filesystem, A/B root slots with
  bootloader-driven switching, and SWUpdate with the suricatta hawkBit client.
  The build produces a disk image and a signed `.swu`.
- **Hardened device policy.** The image accepts only signed `.swu` files. Each
  `.swu` names exactly one hardware revision, with no regex and no wildcard.
  The device's revision is supplied per boot from QEMU firmware config, which
  models a board EEPROM. One image can therefore boot as different boards. A
  device without a revision does not start the updater.
- **End-to-end harness.** `make e2e-image` builds the image. `make e2e` starts
  PostgreSQL and a hawier node with the hardened profile. It provisions typed
  targets, boots QEMU devices, and runs these scenarios:
  - a full A/B update that ends FINISHED, with the new slot booted
  - a signature rejection
  - a hardware-revision rejection on the device
  - a server-side refusal to assign an incompatible set
  - a rollout that reaches only the compatible targets
- **CI.** `.github/workflows/e2e.yml` calls the same make targets. It caches the
  Buildroot download directory and the compiler cache. A budget for build time,
  disk and image size is stated, and it is enforced by the harness.
- **Target type compatibility on the server (hawkBit parity).** Assigning a set
  to a target whose type does not list the set's type as compatible SHALL get
  400. A rollout SHALL include only compatible targets. Target-filter
  auto-assignment SHALL skip incompatible targets. This is a behaviour fix for
  clients that relied on hawier ignoring the relation. hawkBit clients see no
  difference.
- **Typed targets only (hardened).** A new hawier tenant key,
  `target.type.required` (default `false`). When it is on, a target without a
  type receives no assignment at all. It becomes part of the hardened profile:
  API_NOTES §6 and EXTENSIONS.md §5 are edited together.

## Capabilities

### New Capabilities
- `reference-device-image`: the minimal Buildroot system. Covers its layout
  (SquashFS, A/B), its SWUpdate configuration (suricatta, signature,
  hardware-compatibility), its per-boot identity and revision, and its size
  budget.
- `device-e2e`: the QEMU end-to-end harness. Covers its scenarios, what counts
  as a pass, the hardened server profile it runs against, and the CI workflow
  with its time and disk budget.

### Modified Capabilities
- `deployment-assignment`: two requirements are added. Assignment checks target
  type compatibility. Untyped targets are refused under `target.type.required`.
- `rollout-engine`: rollout materialization includes only compatible targets,
  and target-filter auto-assignment skips incompatible ones.
- `tenant-configuration`: the key table gains `target.type.required`.
- `ddi-api`: when the server asks a device for its attributes. The first real
  client found that a target provisioned through Mgmt was never asked, and
  that a finished update did not ask again; hawkBit does both.
- `console-objects`: a refused assignment from the target page is shown on
  the page, not only in the action log. The live check found that the console
  redirected back as if the assignment had succeeded.
- `incidents`: the error table gains 400 `target.type.incompatible`.
- `build-tooling`: the make target list gains `e2e-image` and `e2e`.

These capabilities come from `baseline-existing-capabilities`, which is now
archived.

## Impact

- **New tree:** `e2e/` holds the Buildroot external tree (board, defconfig,
  kernel fragment, SWUpdate config, `sw-description` template, genimage
  config) and the harness scripts (bash, curl and python3, as in
  `scripts/load.sh`). New file: `.github/workflows/e2e.yml`. Build output goes
  to `target/e2e/`, a directory that `make gc` never touches.
- **Server:** the assignment path, rollout materialization, target-filter
  auto-assignment, the tenant key table and one new error code. There is no
  schema change, because compatibility is already stored in hawkBit's tables.
- **Host dependencies of `make e2e`:** `qemu-system-x86_64` (KVM is used when
  `/dev/kvm` exists, otherwise TCG) and a container engine for the build
  container. Downloads go through the proxy, as for every other image build.
- **Hot path:** assignment runs on the control node and is not on the DDI path.
  The rollout engine changes only target selection at materialization. Even
  so, `make load` is compared with `bench/load-history.csv` before the commit.
  A degradation above 20% at the same parameters is a regression.
- **Documentation:** README (the make targets and a section on the e2e stand),
  API_NOTES §6 (the new hardened item) and §9 (the compatibility divergence,
  now fixed), and EXTENSIONS.md §5 (the hardened profile now includes the
  tenant key). The console manual gets a line for the new tenant key, if
  `/settings` lists keys.
- **Findings:** suricatta may show incompatibilities with hawier's DDI.
  Each one is either fixed in its own change or recorded in API_NOTES. The
  harness never works around one silently.
