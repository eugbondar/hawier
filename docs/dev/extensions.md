# Extensions

Everything that can be moved out of the core has been: storage, event
delivery, retention, rollout, console tabs and permissions are **wasm
modules** that hawier loads and calls. There are no in-process
implementations, so a "reference" and a "third-party" implementation of a
point are one and the same. A module has no file system, no sockets and no
node secrets: it sees only the `HAWIER_EXT_*` variables, and the network
only to hosts declared in its manifest, only through the host.

| Extension | Where | Module | What it decides |
|---|---|---|---|
| Storage (files) | `apps/ext-fs` | `hawier_ext_fs.wasm` | the artifact path in the tree |
| Storage (S3) | `apps/ext-s3` | `hawier_ext_s3.wasm` | presigned URL; holds the keys |
| Event delivery | `apps/ext-webhook` | `hawier_ext_webhook.wasm` | whom to send what to, the signature |
| Retention policy | `apps/ext-retention` | `hawier_ext_retention.wasm` | which artifacts may be deleted |
| Rollout (classic) | `apps/ext-rollout-classic` | `hawier_ext_rollout_classic.wasm` | hawkBit percentages and thresholds |
| Rollout (canary) | `apps/ext-rollout-bucket` | `hawier_ext_rollout_bucket.wasm` | random waves over stable buckets |
| Console tabs | `apps/ext-console` | `hawier_ext_console.wasm` | which nodes and views are in the tree |
| Permissions | `apps/ext-authz` | `hawier_ext_authz.wasm` | whether this identity may perform this action |

## Running a node with modules

```bash
make ext        # build the modules (target wasm32-wasip1)
HAWIER_EXT_STORAGE=$PWD/target/wasm32-wasip1/debug/hawier_ext_fs.wasm \
HAWIER_EXT_FS_ROOT=$PWD/artifacts HAWIER_ARTIFACT_DIR=$PWD/artifacts make run
```

Without a storage extension the node serves polling and deployments but
answers artifact downloads with `503` — there is deliberately no fallback.
A wrong module path keeps the node from starting at all. The variables for
each point and the parameters file are in
[configuration.md](configuration.md).

Contracts: specs
[`extension-host`](../../openspec/specs/extension-host/spec.md) and
[`first-party-extensions`](../../openspec/specs/first-party-extensions/spec.md);
why it is built this way and what comes next:
[EXTENSIONS.md](../../EXTENSIONS.md).
