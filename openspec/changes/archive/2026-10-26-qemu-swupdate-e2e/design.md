# Design

## Context

For the motivation, see proposal.md (Why). The requirements are in
`specs/reference-device-image`, `specs/device-e2e` and the five server and
tooling deltas. This document covers how.

What exists today and shapes the approach:

- **Type relations are stored but never checked.** The table
  `sp_target_type_ds_type` is filled by
  `/rest/v1/targettypes/{id}/compatibledistributionsettypes`. The assignment
  path (`deploy_logic::assign_in_tx`) never reads it. Rollout materialization
  (`rollout::materialize_rollout`, `count_matching_targets`) and auto-assignment
  (`rollout::auto_assign_tick`, `matching_target_ids`) do not read it either.
  RSQL already knows `targettype.key`.
- **Link base.** The DDI link base comes from the request's `Host` header. A
  device that reaches the node at `10.0.2.100` gets links to `10.0.2.100`, so
  QEMU user-mode forwarding needs no URL rewriting.
- **Existing tooling.** `make it` already brings up PostgreSQL from the stand
  compose file on `localhost:5432`. `scripts/load.sh` already sets the harness
  style: bash, curl, and python3 in place of jq.
- **The hardened profile today** is "a node without `ext-rollout-classic`"
  (EXTENSIONS.md §5). It has never been run (EXTENSIONS.md status block). This
  change runs it for the first time.
- **Attribute keys.** API_NOTES §9.5: RSQL lowercases `attribute.<key>`, so a
  key with capitals can never be found. The device's attribute keys are
  lowercase for that reason (`hw_revision`, not `hwRevision`).

### Divergence found

hawkBit 1.0 refuses to assign a set to an incompatible target type
(`IncompatibleTargetTypeException`, 400). Its rollouts and auto-assignment
consider only compatible targets. hawier does none of this, and no spec in
`baseline-existing-capabilities` mentions it either way. This change fixes it.
API_NOTES §9 gets an entry marked as fixed by this change, so the history of
the divergence can be followed.

A second divergence surfaced while testing group 2. The rollout-engine spec
requires dynamic rollouts to grow, but the engine's evaluation query did not
select `target_filter`. The read in `grow_dynamic_group` therefore panicked on
the first tick after the static groups finished, and no dynamic rollout ever
grew. No test covered it. The fix is one column, in its own commit with a
regression test for the spec's "Late-joining device" scenario. It is a code
defect against an existing SHALL, so it needs no delta.

## Goals / Non-Goals

**Goals:**
- A real-client check that fits a free hosted runner. The cold build must stay
  well under the job cap, and the warm build is cached.
- One disk image serves every scenario and both hardware revisions.
- The server fix is a small shared predicate, not three copies of the same
  rule.

**Non-Goals:**
- TLS between the device and the node. The `tls-proxy` capability covers TLS,
  and suricatta over HTTPS is a later extension of the harness.
- Real hardware boards, other architectures, and delta updates.
- Running the harness against a deployed host. It runs only against the node
  it starts itself.
- Automatic assignment of a target type from reported attributes. Devices are
  provisioned with their type, and that is the hardened position: a device
  does not choose its own type.
- The other hardened items in API_NOTES §6 (CancelRejected, deny, offline
  installedBase, attempt expiry, force-quit). They stay in the backlog.

## Decisions

### D1. x86_64 under QEMU, KVM when available

Hosted Linux runners expose `/dev/kvm`, and with KVM the system boots in a few
seconds. aarch64 `virt` would match more real boards, but on an x86 runner it
runs under TCG, roughly ten times slower, and it would need U-Boot built from
source. Without KVM the harness falls back to TCG and multiplies its deadlines
(by ×6 at first, to be tuned on the first runs).

### D2. GRUB2 (BIOS) with grubenv, MBR with four primary partitions

SWUpdate supports the GRUB environment block natively, and Buildroot builds
GRUB for `i386-pc`. SeaBIOS ships with QEMU, so no firmware has to be built.
An MBR with exactly four primaries (boot, slot A, slot B, data) avoids the
BIOS-boot partition GPT would need. `core.img` goes in the post-MBR gap.

The boot partition is ext2, because GRUB's `save_env` writes reliably only to
plain filesystems. GRUB reads the kernel out of the slot's SquashFS (module
`squash4`), so kernel and root filesystem change together.

The trial-boot logic in `grub.cfg`, as implemented against SWUpdate 2025.05.
Reading `server_handle_initial_state` showed that suricatta reports success
only from `ustate=2` (testing). From `ustate=1` it sends just "proceeding". So
the bootloader, not the device, moves a fresh install into testing, and no
separate trial counter is needed:

```
ustate=1 (installed)  →  ustate=2; save_env; boot rootslot (the new slot)
ustate=2 (testing)    →  the trial never got the updater up:
                         rootslot=other; ustate=3 (failed); save_env; boot it
otherwise             →  boot rootslot
```

After a healthy trial boot, suricatta starts, finds `ustate=2`, reports
success, and resets the state to 0. If it finds `ustate=3`, it reports
failure. The kernel runs with `panic=5` and `init=/sbin/init`. An explicit
init= that fails panics instead of falling back to /bin/sh, so a slot without
init reboots by itself and lands in the second row of the table.

Alternatives considered:
- **U-Boot on x86.** Unusual, and one more component to build.
- **EFI Boot Guard.** Needs OVMF on the runner and an EFI system partition.
- **A custom initramfs that switches slots.** It would test hawier against
  something no real device runs.

### D3. Prebuilt musl toolchain and a tiny kernel

Building the toolchain is most of a cold Buildroot build. A Bootlin
`x86-64` musl stable toolchain, pinned by URL and checksum, removes it. musl
also keeps the slot image small.

The kernel is built from `tinyconfig` plus a fragment in the external tree:
- virtio-pci, virtio-blk, virtio-net
- SquashFS (xz), ext2/ext4, tmpfs, devtmpfs
- 8250 console
- `FW_CFG_SYSFS`
- IPv4 and unix sockets
- ELF and `#!` scripts
- `panic=5` support

This takes a couple of minutes to build, where `x86_64_defconfig` takes about
ten. The kernel is compressed with xz: it sits in a SquashFS slot that cannot
compress it again. The kernel version is Buildroot's "latest" (6.19.14),
pinned together with Buildroot. Buildroot 2026.02.3 keeps that version's hash
in `linux/from-6.17/`, where its own lookup never finds it, so the tarball
would download unchecked. The external tree adds the hash through
`BR2_GLOBAL_PATCH_DIR` (`patches/linux/linux.hash`).

`linux.config` is generated, not written: allnoconfig with the fragment, then
savedefconfig. The fragment is the source. objtool needs host libelf, so
`BR2_LINUX_KERNEL_NEEDS_HOST_LIBELF` is on.

Fallback: if SWUpdate or one of its libraries fails to build on musl at the
pinned versions, switch to the Bootlin glibc toolchain. Nothing else in the
design changes.

### D4. Package set

The image contains only:
- BusyBox, providing init, shell and the `udhcpc` client
- SWUpdate, with:
  - suricatta hawkBit
  - signed images
  - the GRUB bootloader interface, and the bootloader handler. The handler
    is what registers `bootenv`; without it SWUpdate refuses any
    sw-description that sets a variable.
  - the `raw` handler
  - no web server, no Lua, no compression handlers (SquashFS is already
    compressed)
- libcurl, json-c, libconfig and OpenSSL
- the init scripts from the external tree's overlay

The Bootlin toolchain installs the C++ runtime (libstdc++, libgomp) into the
target because it can. Nothing on the device is C++. The only user is
libconfig++, which nothing links. That runtime was a third of the slot
(7.3 MiB of an 8 MiB budget before), so post-build removes it and fails the
build if any ELF still needs it.

OpenSSL is chosen over mbedTLS for signature verification. It is the signature
path SWUpdate documents and tests most widely, and the cost is about 2 MiB
compressed, well inside the 8 MiB budget. mbedTLS is a later size
optimization, if the budget gets tight.

### D5. Signature: RSA over `sw-description`, keys generated by the build

The build generates two RSA key pairs into `target/e2e/keys/` on first run:
- `signing`: its public half is baked into the image
- `foreign`: used for the `bad-signature` update

`sw-description` is signed with RSA PKCS#1 v1.5 / SHA-256, SWUpdate's
`RAWRSA`. Image hashes are inside `sw-description`, so the signature covers
the slot image too. CMS was rejected because it needs a certificate chain and
buys nothing for a test key. CI generates fresh keys on every cold cache. No
key is ever committed.

### D6. Identity and revision through `fw_cfg`

The harness passes two entries:
- `-fw_cfg name=opt/org.hawier/hwrevision,string=rev1`
- `-fw_cfg name=opt/org.hawier/identity,file=<dir>/identity`, a file of
  `KEY=value` lines: `url`, `tenant`, `controller_id`, `target_token`

An early init script reads `/sys/firmware/qemu_fw_cfg/by_name/opt/org.hawier/*/raw`
and writes `/run/hwrevision` (`hawier-qemu rev1`), to which `/etc/hwrevision`
links. It also renders `/run/swupdate.cfg` from a template in `/etc`, with
suricatta `url`, `tenantid`, `id`, `targettoken`, and an `identify` block with
`hw_revision`, `slot` and `version`. If either entry is missing, the script
prints the reason and does not start SWUpdate.

Alternatives considered:
- **Kernel command line.** It is baked into `grub.cfg` inside the image, so it
  cannot vary per device.
- **Writing into the data partition per device.** That needs mtools or
  debugfs on the host and an image edit for every boot. `fw_cfg` is also what a
  board EEPROM looks like from the device's side: read-only, per unit, and
  outside the image.

### D7. Slot selection by software collection

`sw-description` declares one board, `hawier-qemu`, with one revision in
`hardware-compatibility`. It has two collections:
- `stable.to-b` writes `/dev/vda3` and sets `rootslot=b`
- `stable.to-a` writes `/dev/vda2` and sets `rootslot=a`

The init script starts SWUpdate with `-e stable,to-<inactive>`, derived from
the slot it booted from, which comes from `/proc/cmdline`. One `.swu` therefore
updates either slot. The device never writes the slot it runs from.

### D8. One Buildroot build, many slot images

The image and every `.swu` variant come from one `output/target` tree. The
post-image script copies the tree, writes the version file and the variant
change, and runs the host `mksquashfs` for each variant:

| Output | Version | Variant |
|---|---|---|
| `slot-v1.squashfs` | 1 | base, for both slots of the disk image |
| `slot-v2.squashfs` | 2 | normal update |
| `slot-noinit.squashfs` | 2 | `/sbin/init` removed |

Then a packaging script in the external tree (host `cpio` and `openssl`)
builds:
- `update-rev1.swu`
- `update-rev1-foreign.swu`
- `update-rev2-label.swu` (declares `rev2`)
- `fallback-rev1.swu`

genimage builds `disk.img` with both slots set to v1 and `rootslot=a`. The
budget check runs last and is written in shell. A second Buildroot build per
version would double the build time for a one-line difference.

### D9. The build runs in a builder container, outputs in `target/e2e/`

`e2e/Containerfile` starts from `docker.io/library/debian:bookworm-slim`, with
the Buildroot host prerequisites and ccache. The image is built with
`$(ENGINE)`. Its presence is checked by exact name (`hawier-e2e-builder`
under podman's `localhost/` and docker's bare name), as CLAUDE.md requires.

The repository is mounted, and Buildroot runs out-of-tree:
`O=target/e2e/br-output`, `BR2_DL_DIR=target/e2e/dl`,
`BR2_CCACHE_DIR=target/e2e/ccache`. Proxy variables from the caller's
environment are passed through, and the build uses `--network=host` (see the
build-network note). Ownership differs between the engines:
- rootless podman maps container root to the user
- docker gets `--user $(id -u):$(id -g)`

That is the one engine-specific branch, asked about explicitly, as the
engine rule allows. `scripts/gc.sh` only scans `target/debug`, so
`target/e2e/` is outside its reach. A task verifies this, and a comment there
says so.

The Buildroot tarball is fetched by the Makefile with a pinned sha256, not
vendored and not a submodule. The latest LTS release (`2026.02.x`) is pinned.

### D10. The harness: bash on the host, node from `cargo build`

The harness is `e2e/run.sh`, with `e2e/lib.sh` (waits, API calls, QEMU
lifecycle) and one file per scenario in `e2e/scenarios/`. It is not a Rust
test, for these reasons:
- the work is process orchestration
- a test under `tests/it` would be swept into `make it`, which has no image and
  no QEMU
- a new crate is not wanted

`make lint` gains `shellcheck` over `e2e/`, and `check-tools` names it. The
false positives of indirect calls (functions handed to `wait_for` and `trap`
by name) are switched off once in `e2e/.shellcheckrc`.

`make e2e` depends on `e2e-image` and `ext` and builds the dev node itself.
It does not depend on `build`, which is the release build of the whole
workspace. The node under test is the dev binary with the debug extension
modules.

The node is the dev `hawier` binary run on the host, not a container image.
Building the image in CI would rebuild the workspace without a cache. It is
started with:
- `HAWIER_ROLE=control` (the full node)
- `HAWIER_BIND_ADDR=127.0.0.1:18080`, away from `make run`'s port
- a dedicated database `hawier_e2e`, created and dropped in the stand's
  PostgreSQL
- `HAWIER_ARTIFACT_DIR` set equal to `HAWIER_EXT_FS_ROOT`, an absolute path
  under `target/e2e/run/`
- `HAWIER_EXT_STORAGE` pointing at `ext-fs`
- `HAWIER_EXT_ROLLOUT` pointing at `ext-rollout-bucket` with
  `finalWave=true`, so a two-device rollout reaches everything compatible
- no classic module

The tenant gets `pollingTime=00:00:05` and `target.type.required=true`.

Each device runs from a qcow2 overlay over the read-only `disk.img`, with:
- `-netdev user,restrict=on,guestfwd=tcp:10.0.2.100:80-cmd:python3 -I e2e/relay.py 127.0.0.1 18080`
- serial console to a log file

The forward runs one relay process per guest connection. The first runs used
the chardev form (`…-tcp:127.0.0.1:18080`). That opens one host connection
and puts every guest connection onto it, which is invisible while each
response is read to the end. When a device aborted a download halfway, the
rest of the artifact poisoned the next request ("Received HTTP/0.9"). That
was a harness defect, not a hawier one.

The overlay makes every scenario start from slot A at no copy cost.
`restrict=on` isolates the guest from the host and from the internet, except
for the one forwarded address.

The waits are a single loop with a deadline. Each pass checks the condition,
then checks every QEMU pid it depends on (`kill -0`), then sleeps. This is the
CLAUDE.md rule about waits that must be able to give up.

### D11. Server: one compatibility predicate in three places

There is one SQL fragment with one parameter, the DS type. It is used in three
places:

```
(t.target_type IS NULL AND NOT <required>)
OR EXISTS (SELECT 1 FROM sp_target_type_ds_type r
           WHERE r.target_type = t.target_type AND r.distribution_set_type = $ds_type)
```

- **Assignment.** Confirmed against the hawkBit 1.0.0 source
  (`JpaDeploymentManagement.checkForTargetTypeCompatibility`,
  `RestConfiguration` maps `SP_TARGET_TYPE_INCOMPATIBLE` to `BAD_REQUEST`):
  the check runs over the whole request before anything is written, for both
  shapes: one set to many targets, and many sets to one target. The message
  is `Targets of types [..] are not compatible with distribution set of type
  ..` or, for the second shape, `Target of type .. is not compatible with
  distribution set of types [..]`. hawier uses the same wording.
  The two mgmt handlers check every target of the request
  inside the transaction before the first `assign_in_tx`. The check happens
  after the per-target row lock, so a concurrent type change cannot slip in
  between. The first incompatible target fails the whole request with a new
  `AppError` variant: 400, `target.type.incompatible`, exceptionClass
  `org.eclipse.hawkbit.repository.exception.IncompatibleTargetTypeException`.
- **Rollouts.** The fragment is ANDed into the host-side target query that
  backs `count_matching_targets`, the strategy snapshot, group membership and
  dynamic groups. The strategy extension's contract does not change: it still
  receives counts and definitions and returns selectors. Compatibility stays a
  host-side filter applied before any decision. Nothing crosses the extension
  boundary that did not cross it before.
- **Auto-assignment.** The fragment is ANDed into `matching_target_ids`.

`target.type.required` is read once per request or tick through the existing
`tenant::config_bool`, so it is cached per node like the other device-path
keys.

The predicate uses `sp_target.target_type` and the relation's primary key.
The rollout query gains one semi-join. A task checks it with EXPLAIN on the
`make load` fleet before the commit.

### D12. CI workflow

`.github/workflows/e2e.yml` runs on `ubuntu-24.04`, triggered by
`workflow_dispatch` and by `push`/`pull_request` with a paths filter:
- `e2e/**`
- `apps/server/src/{ddi,deploy_logic.rs,rollout*.rs,mgmt/targets.rs,mgmt/distribution_sets.rs}`
- `libs/api/**`
- the workflow itself

The steps:
1. Enable KVM with the udev rule.
2. Install `qemu-system-x86` and `shellcheck`.
3. Restore the caches:
   - `target/e2e/dl` and `target/e2e/ccache`, keyed by the Buildroot pin plus
     `hashFiles('e2e/**')`, with a restore-key fallback on the pin alone
   - the cargo cache
4. Run `make e2e-image`.
5. Run `make e2e`.
6. Upload the report, and the logs on failure.

The job runs under `timeout-minutes: 60`. Third-party actions are pinned by
commit SHA. The runner image ships podman next to docker, and discovery would
pick podman, so the job sets `ENGINE: docker` explicitly. The cargo side is
cached by `actions/cache` directory by directory: a whole-`target/` cache
action would take `target/e2e` along with it.

The space estimate stays well inside a runner's free disk:

| Item | Size |
|---|---|
| Buildroot output | ~2 GiB |
| downloads | ~300 MiB |
| ccache | ~500 MiB |
| dev `target/` | ~4 GiB |

### D13. Findings from suricatta

A scenario may fail because hawier's DDI differs from hawkBit's in a way
suricatta notices. Likely places:
- the feedback body (`id`, `time`, `status.details`)
- the choice between `download` and `download-http`
- which artifact hashes are offered
- range resume
- whether the `configData` link is present

Such a failure is a finding, never something for the harness to paper over.
A small, clear fix goes into its own commit before the scenario's commit. A
larger one goes to API_NOTES and a separate change, and the scenario stays red
until then. The harness does not rewrite URLs, does not patch suricatta, and
does not relax SWUpdate settings.

## Risks / Trade-offs

- **[Risk] The pinned SWUpdate's GRUB state handling differs from D2's
  table.** For example, the variable names, or when it moves `ustate` from 1
  to 2. → The trial logic is written against the pinned version's
  `bootloader/grub.c` and `core/state.c`, and the `fallback` scenario proves
  it. Bumping the version means rerunning `make e2e`.
- **[Risk] The cold build exceeds 30 minutes on a hosted runner.** → The
  external toolchain and tinyconfig cut the two largest costs. The first CI
  run measures the time, and the result goes into the README. If the build is
  still slow, the next lever is to drop OpenSSL for mbedTLS, then to cache
  `br-output` itself (about 2 GiB, inside the 10 GiB cache quota).
- **[Risk] KVM is unavailable on some runner images.** → TCG with scaled
  deadlines. The run prints which mode it used, so a slow run is not mistaken
  for a hang.
- **[Trade-off] The compatibility check is a behaviour change.** A tenant that
  configured type relations and then assigned across them used to succeed and
  now gets 400. That is hawkBit's behaviour, and nobody can rely on it in
  hawkBit. It goes into the release notes in the README, not behind a flag.
- **[Trade-off] Plain HTTP between device and node.** This matches the stand
  (no TLS). The hardened position on transport security is not tested here.
  That is the non-goal stated above.
- **[Risk] `restrict=on` together with `guestfwd` behaves differently across
  QEMU versions.** → The harness checks QEMU's version at start-up. If the
  forwarded address is unreachable, the first poll fails fast with a named
  error, not after the deadline.
- **[Risk] Leftover processes after a crash of the harness.** → One `trap` on
  EXIT, INT and TERM kills the QEMU pids, stops the node, drops the database
  and stops PostgreSQL. The clean-pass scenario checks that nothing is left.

## Migration Plan

- The data needs no migration: there is no schema change, and the new key
  defaults to `false`.
- The deltas modify capabilities of `baseline-existing-capabilities`, which
  is archived (2026-10-26).
- To roll back, revert the commits. The image tree and the harness are
  self-contained under `e2e/` and `target/e2e/`.

## Open Questions

- Settled: Buildroot 2026.02.3, sha256 from its signed `.sign`. The Bootlin
  x86-64 musl stable 2025.08-1 toolchain and every package are pinned by that
  release's hash files, and the kernel through the external tree (D3).
- Whether `ext-rollout-bucket` needs `modulo`/`sliceSize` overrides for a
  two-device fleet, or whether `finalWave` alone covers it. The first
  `rollout` run settles it, and the parameters are recorded in the scenario.
