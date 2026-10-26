# API_NOTES — shortcomings of the inherited API and the plan to resolve them

This file records the limitations of the hawkBit-compatible APIs that hawier must
resolve in pre-production versions, and the contractual requirements on clients.
(The analogue of SCHEMA_NOTES.md, but for the API rather than the DB schema.) This is a **backlog**: what the API
does today is in the specs [`ddi-api`](openspec/specs/ddi-api/spec.md),
[`artifact-delivery`](openspec/specs/artifact-delivery/spec.md),
[`mgmt-api`](openspec/specs/mgmt-api/spec.md) and neighbouring ones in `openspec/specs/`;
an item from here that is taken into work becomes an OpenSpec change.

## 1. SHORTCOMING: DDI cannot hand polling off to sub-nodes

**Problem.** In the hawkBit DDI protocol the polling base URL
(`/{tenant}/controller/v1/{id}`) is baked into the device configuration. The server
cannot tell the device with a link "keep polling over there on that node" — the
protocol has no such resource. Yet polling is exactly the main load of a
distributed system: 60 million devices polling once a minute ≈ **1 million RPS**,
and it has to be carried by geo-distributed sub-nodes, not by the central node.
Stock hawkBit covers this with an external LB/DNS, which does not solve
pinning a device to the nearest node by means of the protocol itself.

**Plan (pre-production, an additive hawier extension).** The controller-base
response gets a polling-delegation link — the device is handed the sub-node
pinned to it:

```json
{
  "config": { "polling": { "sleep": "00:01:00" } },
  "_links": {
    "pollingBase": { "href": "https://sub-eu-1.example/DEFAULT/controller/v1/dev-42" }
  }
}
```

- a device that sees `pollingBase` continues polling over this link;
- if the pinned sub-node is unavailable, the device falls back to the
  configured central URL and receives a new assignment;
- the central node assigns a sub-node by geography/load (the algorithm is a separate
  decision: geo-IP, explicit target binding, consistent hashing);
- the extension is additive: old hawkBit clients ignore the unknown link
  and keep working as before — the response schema is not broken.

A special case that already works normally today: artifact download links
(`_links.download-http`) may point to any node/CDN/S3 — this is a
standard hawkBit technique, and hawier will use it for byte
traffic independently of polling delegation.

## 2. DOCTRINE: clients must follow links

Every hawier client (including our `hawier-device` and future firmware) must:

- **follow all `_links` as they are** (HATEOAS) — not compose sub-resource URLs
  from its configured base URL;
- derive action sub-resources (`.../feedback`) **only from the href it
  arrived by** (`{href_without_query}/feedback`) — see
  `DdiClient::feedback_href`;
- the only URL the client builds itself is the initial poll.

Without this rule delegation to sub-nodes is impossible: a client that builds URLs
itself will go back to the central node. It is known that some third-party clients
(for example hawkbit-sdk) construct URLs from the base — such clients will keep
working, but will not get geo-distribution. `hawier-device` follows the doctrine
(spec [`device-simulator`](openspec/specs/device-simulator/spec.md)) with one
exception: when `--attributes` is given and there is no `configData` link, it
builds that URL from the base (§9.9).

## 3. FUTURE: rollouts in a geo-distributed topology

The direction is recorded:

- **rollout management — the central node only**: creation, splitting into
  groups, thresholds, approve/deny, start/pause/stop;
- **sub-nodes — executors of the will and reporters**: they serve deployments and
  artifacts, accept device feedback, store device statuses and messages
  (`sp_action_status`, `sp_action_status_messages`) and thereby
  report to the center "will carried out / not carried out";
- today the report converges automatically through the shared DB (nodes are stateless);
- **open design question** (to be worked out before geo-distribution): when
  sub-nodes get regional storages, a designed reporting channel upward
  will be needed — a queue / batched replication of device statuses and logs
  from sub to center, with idempotency and tolerance for lag.
  Advancing rollout groups remains the center's decision, based on
  aggregated reports.

## 4. FUTURE: resumable artifact upload (a hawier extension)

**Fact:** the hawkBit Mgmt API has no resumption of an interrupted upload —
only a whole multipart POST (`MultipartFile`); `Range`/`Content-Range`
are used exclusively on *download*. "Topping up" in the sense of adding
new artifacts to an existing SM/DS at any time is supported normally
(repeated `POST /softwaremodules/{id}/artifacts`) and implemented in the UI.

**Sketch of a future additive extension** (for when large
images need to be uploaded over unstable channels):

- `POST /rest/v1/softwaremodules/{id}/artifacts/uploads` → `{uploadId, offset:0}`
  (a staging file on the control node);
- `PUT .../uploads/{uploadId}` with `Content-Range: bytes a-b/total` — append when
  `a == offset`, otherwise `409 {offset}` (the client continues from the current one);
- `GET .../uploads/{uploadId}` → `{offset}` — the resume point;
- `POST .../uploads/{uploadId}/finalize?filename=&sha1sum=...` — stream into
  content-addressed storage, verify hashes as in the multipart path, write
  `sp_artifact`;
- clean up staging files older than 24 h. The multipart endpoint stays
  unchanged; the extension is purely additive.

**Other small things for the future** (nothing is blocked by them now):

- client-side resume of an interrupted *download*: the server already serves single-range 206
  with ETag=sha1, but `hawier-device` re-downloads the file from scratch on a break — teach the
  client to continue from an offset via `Range` + `If-Match`;
- both device-matrix test cases recorded here earlier ("a clean soft
  is installed by default", "two devices with one controller-id") have been added and
  pass.

## 5. IMPLEMENTED: additive extensions of the hawkBit API

Everything hawier added to hawkBit or fixed in it is now described
normatively in the specs; the detailed history of decisions is in the git history of this file.
Here is only the list and why each one was needed.

| What | Why | Spec |
|---|---|---|
| `GET /distributionsets/{id}/tags` | the DS card needs tags in a single request | `mgmt-api` |
| `GET /incidents`, `GET /actionlog` | logs as ordinary console resources | `mgmt-api`, `incidents`, `ui-state-api` |
| `assignedDistributionSetName` and neighbouring `…Name`, `autoAssignDistributionSetName` | a list column needs a label, not a bare id | `mgmt-api` |
| `DELETE /targets/{id}/actions`, `rolloutgroup*` filters, `q`/`sort` on a group's targets | catch up with the hawkBit contract | `mgmt-api`, `deployment-assignment` |
| `GET /targetgroupdetails` | a list of groups with total, filter and sorting; the old `/targetgroups` is untouched | `mgmt-api` |
| metadata pagination | an honest `total` | `mgmt-api` |
| RFC 6266 `Content-Disposition` on mgmt download | a non-ASCII name crashed the response with a 500 (**behavior change**; DDI is not yet fixed — §9.2) | `mgmt-api` |
| `RETRIEVED` once under the action lock | two devices with one controller-id wrote duplicates | `ddi-api` |
| status repeat-collapse, `repeats`/`lastReportedAt` | amplification loop: 2.7 million rows for 1000 devices | `ddi-api` |
| overload — `503` + `Retry-After`, not `500` | `500` invites the fleet to retry immediately | `ddi-api`, `incidents` |
| `/webhooks`, `/webhookdeliveries` | events without AMQP/DMF | `event-delivery` |
| `/retentionrules` | scheduled artifact retention | `artifact-retention` |
| `rollout.paused` also from the operator | the "rollout stopped" subscription stayed silent on manual pauses | `event-delivery` |
| `/roles`, `/grants`, permissions in `userinfo`; enforcement from the first grant | the first *role* locked the operator out with no way back | `access-control` |
| `GET /ui/v1/surfaces` | console extension tabs | `ui-state-api` |
| `pollState`; `pollingOverdueTime` is finally read | liveness as a list column; formula as in hawkBit | `mgmt-api`, `tenant-configuration` |
| `GET /system/extensions` and parameters | what the node delegates and to whom | `extension-host` |

**Remaining:** the node answers 503 only after it has already hit the resource, not in advance.
There is no limiter on concurrent downloads, and `polling.sleep` is still
a tenant constant — that is, the node cannot ask "the whole fleet, poll less often",
it can only refuse individual requests.

## 6. HARDENED: hawkBit semantics that a future hardened mode throws out

Principle: **a device in an indeterminate state is unacceptable**. Any
semantics after which it is impossible to say what is currently installed on the device
and what it will do next — out. The recovery path is always one and
simple: a new rollout. For now hawier is fully compatible with hawkBit — these
items only accumulate here as they are discovered (like SCHEMA_NOTES for the
schema) and will be implemented all at once when the hardened profile is enabled.

The build side of this same profile is described in **EXTENSIONS.md §5**: hardened is
a build *with* the extension mechanism, but without the classic rollout
extension (`ext-rollout-classic`), which carries the same hawkBit semantics —
percentages and thresholds that ignore the stored condition ordinals — and with the tenant
key `target.type.required=true` (item 6). The profile is exercised by `make e2e`
on a real client (SWUpdate/suricatta in QEMU).
Both lists describe one profile and change in agreement.

Format: what it is in hawkBit → why it is insane → the hardened solution.

1. **CancelRejected — "cancel of the cancel".** A device can *reject*
   cancelAction, and the action goes on living as if nothing had happened.
   The operator cancelled the update, for example, because of a bug found in the
   firmware — and the device "changed its mind" and keeps installing it; what
   state it ends up in is unknown, and nobody is obliged to know it.
   Hardened: cancellation is final. A device that physically cannot
   interrupt an installation reports failure — but the action is closed
   as CANCELED in any case and the device gets no new links to it.
   Need to deliver the update again — a new rollout.
2. **Deny confirmation — eternal limbo.** A confirmation denied by the device
   leaves the action in WAIT_FOR_CONFIRMATION forever: it is active,
   occupies a slot, the supersede logic has to account for it, and when it ends
   is unknown (our `ddi` pins this behavior down as
   compatibility). Hardened: deny closes the action; changed your mind —
   a new rollout/assignment.
3. **Offline PUT installedBase — blind trust in the device.** A device can
   declare *any* existing DS as "installed", and the server
   records installedDS and IN_SYNC without any verification. The fleet
   inventory then is the opinion of the devices, not a fact of the server. Hardened:
   remove offline declaration, or accept it only as an unreliable
   mark that requires confirmation by the regular update cycle.
4. **Soft/attempt without a deadline.** A device can postpone an attempt update
   forever, the server does not escalate or highlight this in any way —
   the action just hangs active. Hardened: a mandatory server-side
   expiry for attempt actions (escalation to forced or closing with
   an error on timeout — a tenant policy).
5. **Force-quit with a live device.** `DELETE ...?force=true` closes the
   action on the server while the device may be writing the
   firmware right now; the server then considers the old DS installed, the device — the
   new one. Hardened: force-quit puts the target into an explicit state
   "unknown" until the device's next honest report (poll +
   configData/installedBase cycle), and this state is visible in filters.

6. **An untyped target accepts anything.** A target without a target type
   is compatible with any DS — that is how it is in hawkBit, and in hawier by default.
   The "firmware is not for this board" check fires only if the operator
   did not forget to assign a type: a forgotten type silently opens the board to any
   image. Hardened: the tenant key `target.type.required=true` — a target without
   a type receives nothing (assignment — `400 target.type.incompatible`,
   rollout and auto-assignment skip it). **Implemented** by the change
   `qemu-swupdate-e2e`; enabling it is part of the hardened profile (EXTENSIONS.md §5).
   Compatibility: the key is off by default, a stock client sees no difference.

## 7. SHORTCOMING: per-group `targetFilterQuery` is accepted and ignored

**Problem.** `POST /rest/v1/rollouts` accepts a `targetFilterQuery` on every group,
puts it into `sp_rollout_group.target_filter`
(`mgmt/rollouts.rs`, the bind in the group INSERT) — and that is all: the engine
(`rollout::materialize`) splits targets **only** by `target_percentage`
and never reads the column. The operator gives the groups meaningful filters,
gets 201 and a rollout split quite differently from what they asked for. No error,
no warning — just a silently wrong split. Likewise unread are
`success_action`/`error_action`: the constants NEXTGROUP/PAUSE are written, and
`evaluate_running` behaves as THRESHOLD + NEXTGROUP in any case.
The schema side of the same thing — SCHEMA_NOTES #22.

**Status (2026-09-06).** The split has become an extension point
(EXTENSIONS.md §4.7), and the strategy receives the group definitions in full,
including the per-group filter. The engine no longer silently drops it:
it is now the decision of the specific strategy. The classic one
(`hawier-ext-rollout-classic`) still ignores the filter — deliberately,
because "classic" means the semantics people already have — and
this is stated in its own documentation. A strategy that
honours it is `GroupSelector::Filter`, that is, a different module.

**Remaining:** warn the operator on create that the chosen
strategy does not use the per-group filter they specified. Today they still
get 201 and a rollout split not as they asked — only now
the reason is named.

**Compatibility.** Stock hawkBit applies the filter, so this is
a divergence of behavior, not of format: the request and response bodies match.

## 8. EXTENSION: remote-shell as an additional controller-base link

A planned additive DDI extension (details — EXTENSIONS.md §4.3); its contract is
recorded here because it touches the poll response.

The device gets a `remoteShell` link in `_links` — **only when there is a command
for it** and the tenant toggle is on:

```json
{
  "config": { "polling": { "sleep": "00:01:00" } },
  "_links": {
    "remoteShell": { "href": "https://sub-eu-1.example/DEFAULT/controller/v1/dev-42/shell/8123" }
  }
}
```

- the device GETs the command over the link, executes it and POSTs the output in chunks
  to feedback derived **from the href it arrived by** — doctrine §2
  is respected, no URLs from the base;
- the channel is store-and-forward through Postgres, so any
  sub-node serves the command, and a dead control node does not get in the way;
- the extension is additive: a client that does not know the link ignores it;
- the link does not appear in the response while there are no commands — a regular poll
  does not change in cost.

A conscious risk: this is remote execution across the fleet. Therefore the toggle is
off by default, the `TARGET_REMOTE_SHELL` permission is required, every command
is written to `hawier_action_log`, and the decision "is the command allowed" is moved into
the `ShellPolicy` extension point.

## 9. SHORTCOMINGS found during the move to OpenSpec (2026-10-26)

Checking the code against the specs found divergences that were deliberately not
written into the specs: SHALL describes the intended behavior, and a defect written into a SHALL
would turn its own fix into a "breaking" delta. The numbers in parentheses are items
of the catalog in the design.md of the change `baseline-existing-capabilities`. Compatibility
is the same everywhere: clients that do not rely on the defect will not notice the fixes.

1. **DDI Range: an unsatisfiable or malformed range → 400, not 416** (16).
   A comment in the code promises 416; a client with a correct reaction to 416 does not
   get it. Plan: 416 with `Content-Range: bytes */<size>`.
2. **DDI `Content-Disposition` is not RFC 6266** (17): a non-ASCII file name crashes
   the download and `.MD5SUM` with a 500. On mgmt this is already fixed (§5). Plan: the same
   `filename` + `filename*` as on mgmt.
   Since §10 the upload refuses non-ASCII names, so only names already in a
   database migrated from hawkBit can hit this.
3. **Feedback on `deploymentBase` overwrites CANCELING** (18): `proceeding`
   on a cancelling action moves it back to RUNNING, and the cancellation is lost.
   Plan: on CANCELING accept progress only through the cancel channel, the rest
   — 409 or a record without a state change.
4. **The `DOWNLOAD` status is attached to the wrong action** (19): to the top one by weight,
   not to the one whose DS owns the module; insert errors (including
   quota) are swallowed. Plan: look up the action by module, surface the quota error.
5. **RSQL** (22): an existence field with `=in=(a,b)` uses only `a`;
   the key `attribute.<key>`/`metadata.<key>` is lowercased,
   so keys with capitals are never found; `*` in these fields does not
   escape `%` and `_`. Plan: all `=in=` arguments, the key without case change,
   common escaping.
6. **Tenant config** (23): `/system/configs` shows
   `authentication.targettoken.enabled=false (global)`, although `true` from the
   environment is in effect; `batch.assignments.enabled` is declared and affects nothing;
   unknown keys are silently accepted. Plan: show the effective value,
   remove or implement `batch.assignments`, 400 on an unknown key.
7. **An error body larger than 64 KiB is returned empty** (25). Plan: truncate
   `message` instead of dropping the body.
8. **Sublists ignore `q`/`sort` or return a bare array** (26): a DS's assigned
   and installed targets, assigned modules, rollout deploy groups,
   `targetgroups/{g}/assigned`; a module's artifacts, tags, type links. Plan: in the
   same way as `targetgroupdetails` — a resource alongside, leave the old path untouched.
9. **`hawier-device` builds the `configData` URL from the base** (28) if there is no link in
   the poll and attributes are set. Plan: wait for the link; without it do not send attributes.
10. **Compatibility of target and DS types was not checked** — found and
    fixed by the change `qemu-swupdate-e2e`. The link
    `targettypes/{id}/compatibledistributionsettypes` was stored, but neither
    assignment, nor rollout, nor auto-assignment read it: a set of any type
    went to a target of any type. hawkBit 1.0 refuses with `400
    target.type.incompatible` for the whole request and for rollout/auto-assignment
    takes only compatible targets. Now it is the same; a type with no
    compatible type accepts nothing, as in hawkBit. Compatibility:
    a client that assigned across its own type links now gets
    400 — exactly what it would get from hawkBit.

## 10. DIVERGENCE: artifact file names are printable ASCII (2026-10-27)

**Problem.** hawkBit accepts any artifact file name. A non-ASCII name answered
500 on the DDI download (§9.2), and a name with `/` could never be downloaded
over DDI, which addresses the artifact by its name as one path segment.

**Plan.** Done: the management upload refuses, with 400
`hawkbit.server.error.repo.invalidParameter` and nothing stored, a name that is
empty, longer than 256 bytes, outside printable ASCII, or contains `/` or `\`
(spec `mgmt-api`, change `english-only-repo`).

**Compatibility.** A client that uploads such names gets 400 instead of 201 and
has to rename the file. Names already stored (a database migrated from hawkBit)
are not touched: the management download serves them with RFC 6266, the DDI
download still has §9.2.

## File maintenance rule

How to maintain this file is in the rules of `openspec/config.yaml` (problem → plan
→ compatibility with old clients, by analogy with SCHEMA_NOTES.md).
