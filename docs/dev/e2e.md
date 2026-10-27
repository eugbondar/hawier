# Device end-to-end run

`make e2e` checks hawier not with its own simulator but with
SWUpdate/suricatta — the client real fleets run. Devices from the reference
image (Buildroot, SquashFS, A/B) are brought up in QEMU, the node runs in
the hardened profile, and six scenarios run: a full A/B cycle, refusal on
signature, refusal on the device's hardware revision, the server's refusal
of an incompatible assignment, a rollout over a mixed fleet, and rollback
of a slot that failed to boot. Behaviour: specs
[`device-e2e`](../../openspec/specs/device-e2e/spec.md) and
[`reference-device-image`](../../openspec/specs/reference-device-image/spec.md).

## Host requirements

A container engine (the Buildroot build runs in a builder container),
`qemu-system-x86_64` and `qemu-img`, `curl`, `python3`. KVM is used if
`/dev/kvm` is readable and writable; otherwise software emulation with
deadlines ×6 (`E2E_ACCEL=tcg` forces it). The first image build downloads
sources; a repeat build downloads and rebuilds nothing. `make e2e-image`
refuses an image that outgrows its size budget and names the quantity; the
limits are in the `reference-device-image` spec.

## Running

```bash
make e2e-image
```

```bash
make e2e
```

```bash
E2E_ONLY=fallback make e2e
```

Scenarios: `incompatible-assignment`, `update`, `bad-signature`,
`wrong-revision`, `rollout`, `fallback`. The result of each (PASS/FAIL and
duration) is in `target/e2e/report.txt`; serial logs of the devices, the
node log and the action history of each target are in `target/e2e/logs/`.
The node under test is a dev binary on `127.0.0.1:18080` with a database
`hawier_e2e` of its own; after the run no QEMU, no node and no running
PostgreSQL is left.

## A device by hand

To look at the image: a copy of the disk on top of the reference, revision
and identity via fw_cfg, as the harness does. Without an identity SWUpdate
does not start and says so on the console; login is root with no password
on `ttyS0` (test image).

```bash
qemu-img create -f qcow2 -F raw -b "$PWD/target/e2e/out/disk.img" /tmp/dev.qcow2
```

```bash
qemu-system-x86_64 -accel kvm -m 192 -nographic -drive file=/tmp/dev.qcow2,if=virtio,format=qcow2 -netdev user,id=n0,restrict=on -device virtio-net-pci,netdev=n0 -fw_cfg name=opt/org.hawier/hwrevision,string=rev1
```
