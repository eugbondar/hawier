# first-party-extensions Specification

## Purpose

The extension modules that ship in the monorepo (`apps/ext-*`, built with
`make ext`) and that integration tests load as-is. This spec covers the decision
each one makes and the parameters it reads.

## Requirements

### Requirement: Filesystem storage module
`hawier-ext-fs` SHALL implement `storage.locate`. For every operation it SHALL answer the path `{HAWIER_EXT_FS_ROOT}/{TENANT}/{sha1[-4..-2]}/{sha1[-2..]}/{sha1}`. The root defaults to `/var/lib/hawier` and the tenant is upper-cased. For a sha1 that is not hex, or is shorter than 4 characters, it SHALL answer `Absent`.

#### Scenario: Layout
- **WHEN** it locates sha1 `2aae…46ed` for tenant `default`
- **THEN** the path ends with `DEFAULT/46/ed/2aae…46ed`

#### Scenario: Traversal attempt
- **WHEN** the sha1 is `../../etc/passwd`
- **THEN** it answers `Absent`

### Requirement: S3 storage module
`hawier-ext-s3` SHALL implement `storage.locate` by returning a presigned URL with the same key layout as the filesystem module.

The method SHALL equal the operation, the expiry SHALL be now + `HAWIER_EXT_S3_PRESIGN_TTL_SECS` (default 900), and redirect SHALL be permitted for GET only.

Parameters:
- required: `HAWIER_EXT_S3_BUCKET`, `HAWIER_EXT_S3_ENDPOINT` (with a scheme), `HAWIER_EXT_S3_ACCESS_KEY`, `HAWIER_EXT_S3_SECRET_KEY`
- optional: `HAWIER_EXT_S3_REGION` (default `us-east-1`), `HAWIER_EXT_S3_PATH_STYLE` (default true)

By default it SHALL sign locally with SigV4 query presigning (`UNSIGNED-PAYLOAD`, signed header `host`). With `HAWIER_EXT_S3_SIGNER_URL` set, it SHALL declare that host as `http_out` and obtain the URL from that signer instead.

#### Scenario: AWS reference vector
- **WHEN** it presigns the AWS documentation example request
- **THEN** the signature equals the published one

#### Scenario: Download is redirected
- **WHEN** a device downloads through a node using the S3 module
- **THEN** the node answers 302 to a presigned GET URL

### Requirement: Webhook event-sink module
`hawier-ext-webhook` SHALL implement `event_sink.plan`.

Matching:
- a subscription mask of `*`, or an empty mask, SHALL match every event kind
- otherwise the mask SHALL be a comma-separated list of exact kinds; prefixes SHALL NOT match

Each matched subscription SHALL get a POST whose body is the JSON event, with these headers:
- `content-type: application/json`
- `x-hawier-event: <kind>`
- `x-hawier-delivery: <event id>`
- with a non-empty secret, also `x-hawier-signature: sha256=<hex HMAC-SHA256(secret, body)>`

#### Scenario: Signed delivery
- **WHEN** a subscription with a secret matches a `rollout.started` event
- **THEN** the planned POST carries a signature header that verifies with that secret over the body

### Requirement: Retention policy module
`hawier-ext-retention` SHALL implement `retention.select` for scope `artifact` only. It SHALL read the rule parameters:
- `keepPerModule`, default 3
- `releasePattern`, default `-dev`, `-snapshot`, `+build`, matched as case-insensitive substrings
- `deleteAssigned`, default false

It SHALL never select:
- release artifacts, meaning those matching no marker
- artifacts in a distribution set
- artifacts ever assigned, unless `deleteAssigned` is true
- the newest `keepPerModule` development builds per module

The delete list SHALL be sorted and the reason SHALL summarise the counts.

#### Scenario: Keep newest dev builds
- **WHEN** a module has five unassigned `-dev` artifacts and `keepPerModule` is 3
- **THEN** the two oldest are selected for deletion

### Requirement: Classic rollout strategy module
`hawier-ext-rollout-classic` SHALL implement `rollout.plan` and `rollout.evaluate` for the strategy names "" (empty), `default`, `classic` and `percentage`. Any other name SHALL get an error.

Plan:
- every declared group but the last SHALL get `ceil(total × targetPercentage / 100)` targets (percentage default 100), taken from targets not yet in a group
- the last group SHALL get the rest
- per-group target filters SHALL be ignored, which is hawkBit-classic semantics

Evaluate:
- an empty group SHALL be finished
- a group whose error share reaches the error threshold, with at least one error, SHALL be errored and SHALL pause the rollout
- a group whose finished share reaches the success threshold SHALL be finished

#### Scenario: Even split loses nobody
- **WHEN** a rollout over 11 targets declares two 50% groups
- **THEN** the groups hold 6 and 5 targets

### Requirement: Bucket canary strategy module
`hawier-ext-rollout-bucket` SHALL implement both rollout points for the strategy names `bucket`, `canary` and `wave`.

Parameters:
- `modulo`, default 100000, minimum 2
- `sliceSize`, default 1000, clamped to 1..modulo
- `waves`, default 5, clamped to 1..100
- `finalWave`, default true

Plan:
- it SHALL plan `waves` groups `wave-N`, each a bucket slice
- each slice start SHALL come from a seed derived from the rollout id, so a rollout always picks the same devices
- with `finalWave`, a final `rest` group SHALL follow
- thresholds SHALL be copied from the first declared group

A device's bucket SHALL be derived from its controller id. Declared groups the plan does not use SHALL be removed. A target SHALL never be placed in two groups.

#### Scenario: Waves cover the fleet once
- **WHEN** a 40-target rollout uses 2 waves with the final wave
- **THEN** there are 3 groups covering all 40 targets exactly once

### Requirement: Console declaration module
`hawier-ext-console` SHALL implement `console.surfaces`. It SHALL declare these extra views and no surfaces:
- targets `field-trial`: `tag=="field-trial"`, sorted by last poll descending
- targets `never-polled`: `pollState==never`
- rollouts `stalled`: `status==paused`

#### Scenario: Declared views reach the sidebar
- **WHEN** the console starts against a node with this module
- **THEN** the targets node lists `field-trial` and `never-polled` as views marked as declared

### Requirement: Role-based authorization module
`hawier-ext-authz` SHALL implement `authz.decide`. It SHALL allow in exactly these cases:
- the tenant has no grants at all, so enforcement is unconfigured
- the user holds `ROLE_ALL`
- the user holds the requested permission

Otherwise it SHALL deny with a reason naming the user, tenant and missing permission.

#### Scenario: Missing permission
- **WHEN** a user whose roles grant only READ_TARGET attempts CREATE_TARGET in a tenant with grants
- **THEN** the verdict is deny and the reason names CREATE_TARGET
