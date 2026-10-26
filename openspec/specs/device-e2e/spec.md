# device-e2e Specification

## Purpose

An end-to-end check of hawier against a real update client. QEMU devices built
from the reference image run SWUpdate's suricatta against a hawier node in the
hardened profile, through full update cycles and the refusals the hardened
policy promises. The same make target runs locally and in hosted CI.

## Requirements

### Requirement: One make target runs the whole check
`make e2e` SHALL build whatever of the reference image is out of date, then
run every scenario. The run SHALL bring up:
- PostgreSQL from the stand compose file
- a hawier node built with the `dev` profile, together with its extension
  modules

It SHALL tear both down afterwards, whether or not the scenarios passed.

`E2E_ONLY=<scenario>` SHALL run a single scenario. A missing
`qemu-system-x86_64` SHALL fail the run before anything starts, with a message
naming the tool. Without `/dev/kvm` the run SHALL fall back to software
emulation, scale its timeouts, and say so.

#### Scenario: Clean pass
- **WHEN** `make e2e` runs on a machine with a container engine and QEMU
- **THEN** every scenario passes, the exit status is 0, and no QEMU, hawier or PostgreSQL process is left running

#### Scenario: No QEMU
- **WHEN** `make e2e` runs on a machine without `qemu-system-x86_64`
- **THEN** it fails at once with a message naming `qemu-system-x86_64`

### Requirement: The node under test is hardened
The node under test SHALL run without the classic rollout extension. Its
rollout strategy SHALL be a non-classic module. The tenant key
`target.type.required` SHALL be on. Before any scenario starts, the harness
SHALL confirm all three from the node's own extension inventory and
configuration API. A node that does not match SHALL fail the run.

#### Scenario: Classic strategy loaded by mistake
- **WHEN** the node under test reports the classic rollout extension as loaded
- **THEN** the run fails before booting any device, naming the extension

### Requirement: Devices are provisioned, typed and isolated
Every device SHALL be created through the management API before it boots,
with:
- its controller id
- its target token
- the target type of its hardware revision

Each hardware revision SHALL have its own distribution set type, and its
target type SHALL list exactly that one set type as compatible. Each update
SHALL be uploaded as the single artifact of a software module in a set of the
matching type.

Devices SHALL reach hawier only through the emulator's user-mode network,
restricted to the node's address. They SHALL have no other network access.

#### Scenario: Device registers as provisioned
- **WHEN** a `rev1` device boots for the first time
- **THEN** the existing target with its controller id records the poll, and no other target is created

### Requirement: Scenarios
The harness SHALL run these scenarios. Each one SHALL start from freshly
provisioned devices on slot A.

| Scenario | Action | Expected outcome |
|---|---|---|
| `update` | assign a `rev1` set with version 2 to a `rev1` device | action FINISHED; installed set is the new set; attributes `slot=b`, `version=2` |
| `bad-signature` | assign a `rev1` set whose `.swu` is signed with a foreign key | action ERROR; attributes still `slot=a`, `version=1` |
| `wrong-revision` | assign a `rev1`-typed set whose `.swu` declares `rev2` | action ERROR; slot unchanged |
| `incompatible-assignment` | assign a `rev2` set to a `rev1` device, and any set to an untyped target | both 400 `target.type.incompatible`; no action created |
| `rollout` | a rollout of a `rev1` set whose filter matches one `rev1` and one `rev2` device | `totalTargets` 1; the `rev1` device ends FINISHED; the `rev2` device is never offered a deployment |
| `fallback` | assign a `rev1` set whose slot image has no init | action ERROR; the device runs again on slot A |

#### Scenario: Full update cycle
- **WHEN** the `update` scenario runs
- **THEN** the device downloads the `.swu` from hawier, reboots into slot B, and hawier shows the action FINISHED with the device's attributes `slot=b` and `version=2`

#### Scenario: Server refuses an incompatible set
- **WHEN** the `incompatible-assignment` scenario runs
- **THEN** both assignment calls get 400, and neither target has an action

### Requirement: Every wait gives up and explains
Every wait in the harness SHALL have a deadline. In the same loop, it SHALL
also check that every QEMU process it depends on is still alive. A device
that exits or a deadline that passes SHALL fail the scenario at once.

On failure, the harness SHALL keep the following under `target/e2e/logs/`:
- the serial console of every device
- the node's log
- the target's action history from the management API

It SHALL print where they are. `target/e2e/report.txt` SHALL list each
scenario with its result and duration.

#### Scenario: Device kernel panics
- **WHEN** a device's QEMU process exits in the middle of a scenario
- **THEN** that scenario fails within one wait interval, and its serial log is kept and named in the output

### Requirement: Hosted CI within budget
`.github/workflows/e2e.yml` SHALL run `make e2e-image` and `make e2e` on a
hosted Linux runner. It SHALL call make targets only. It SHALL:
- enable KVM when the runner offers it
- cache the Buildroot download directory and the compiler cache, keyed by the
  pinned Buildroot version and the external tree's content
- cap the job at 60 minutes
- upload `target/e2e/report.txt`, and on failure `target/e2e/logs/`, as job
  artifacts

It SHALL be runnable on demand. It SHALL also run when the external tree, the
harness, the DDI, assignment or rollout code, or the workflow itself changes.

#### Scenario: Warm cache
- **WHEN** the workflow runs with a cache hit and an unchanged external tree
- **THEN** the image build downloads no sources
