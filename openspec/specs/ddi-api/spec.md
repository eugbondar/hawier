# ddi-api Specification

## Purpose

The device-facing Direct Device Integration (DDI) API: the hawkBit-compatible
contract that devices use to poll, receive deployments, confirm or cancel them,
and report progress. All paths are under `/{tenant}/controller/v1/{controllerId}`.

## Requirements

### Requirement: DDI endpoint surface
The server SHALL serve these hawkBit-compatible DDI resources under
`/{tenant}/controller/v1/{controllerId}`:
- `GET` (the controller base, also called the poll)
- `GET deploymentBase/{actionId}` and `POST deploymentBase/{actionId}/feedback`
- `GET installedBase/{actionId}` and `PUT installedBase`
- `GET cancelAction/{actionId}` and `POST cancelAction/{actionId}/feedback`
- `PUT configData`
- `GET softwaremodules/{smId}/artifacts`
- `GET softwaremodules/{smId}/artifacts/{fileName}`, also serving the `{fileName}.MD5SUM` companion
- `GET confirmationBase`, `POST confirmationBase/activateAutoConfirm`, `POST confirmationBase/deactivateAutoConfirm`
- `GET confirmationBase/{actionId}` and `POST confirmationBase/{actionId}/feedback`

AMQP/DMF SHALL NOT be offered. Tenant names in the path SHALL match
case-insensitively.

#### Scenario: Tenant case does not matter
- **WHEN** a device of tenant `DEFAULT` polls `/default/controller/v1/dev-1` with a valid token
- **THEN** the server answers as for `/DEFAULT/controller/v1/dev-1`

#### Scenario: Sub-resource of an unregistered controller
- **WHEN** a gateway or anonymous actor requests any non-poll resource for a controllerId that has no target
- **THEN** the server answers 404

### Requirement: Device authentication
Every DDI request SHALL be authenticated before any other processing, by one of these modes:
- `Authorization: TargetToken <token>` SHALL be accepted only if the tenant setting `authentication.targettoken.enabled` is on, the target exists, and the token equals that target's security token. The comparison SHALL be constant-time.
- `Authorization: GatewayToken <key>` SHALL be accepted only if `authentication.gatewaytoken.enabled` is on and `authentication.gatewaytoken.key` is non-empty and equals the key. The target need not exist.
- A request with neither header SHALL be rejected unless the node runs with `HAWIER_DDI_ANONYMOUS=true`. That mode is for development only.

A rejection SHALL be 401 with `WWW-Authenticate: Basic realm="hawier"` and an
ExceptionInfo body with `errorCode` `hawkbit.server.error.unauthorized`.
Authorization roles and grants of the management API SHALL NOT affect DDI.

#### Scenario: Wrong target token
- **WHEN** a device polls with a TargetToken that does not match its target
- **THEN** the server answers 401

#### Scenario: Target token cannot register a device
- **WHEN** a TargetToken request names a controllerId that has no target
- **THEN** the server answers 401 and creates nothing

#### Scenario: Gateway token toggle
- **WHEN** a device polls with the correct gateway key while `authentication.gatewaytoken.enabled` is off
- **THEN** the server answers 401

#### Scenario: Management grants are irrelevant to devices
- **WHEN** the authorization policy denies every management permission
- **THEN** DDI poll, deployment, download and feedback keep working

### Requirement: Plug-and-play registration
A poll from a gateway-token or anonymous actor for an unknown controllerId SHALL create the target, and the tenant if it is missing. The new target SHALL have:
- name = controllerId
- a random security token
- update status REGISTERED
- creator `CONTROLLER_PLUG_AND_PLAY`
- controller attributes requested

Concurrent first polls SHALL create exactly one target.

#### Scenario: First poll registers and asks for attributes
- **WHEN** an unknown controller polls with a valid gateway token
- **THEN** the server answers 200, the target exists with status REGISTERED, and the response carries `_links.configData`

### Requirement: Poll response
The poll SHALL return `{"config":{"polling":{"sleep":"HH:MM:SS"}},"_links":{...}}`.
- The sleep SHALL be the tenant `pollingTime`, default `00:05:00`.
- `_links` SHALL be omitted when there are no links.
- Link hrefs SHALL be absolute. They SHALL be built from the request's `X-Forwarded-Proto` (default `http`) and `Host` headers.

The links SHALL be selected as follows. The current action is the target's active action with the highest weight, oldest id first among equal weights.
- `cancelAction/{id}` SHALL appear if the current action is CANCELING. `deploymentBase` SHALL NOT appear in that case.
- `deploymentBase/{id}?c=<opaque>` SHALL appear if the current action is in any other active state except WAIT_FOR_CONFIRMATION.
- `confirmationBase` SHALL appear if and only if the tenant setting `user.confirmation.flow.enabled` is on.
- `installedBase/{id}` SHALL appear when the target has an installed distribution set. The id SHALL be the newest FINISHED action of that set.
- `configData` SHALL appear while the server requests controller attributes from this target.

#### Scenario: Idle device
- **WHEN** a registered device with no actions polls and `pollingTime` is unset
- **THEN** the response sleep is `00:05:00` and there is no `_links` object

#### Scenario: Assignment makes the deployment link appear
- **WHEN** a distribution set is assigned to the target and the device polls
- **THEN** `_links.deploymentBase` is present and points at the new action

#### Scenario: Links behind a TLS proxy
- **WHEN** the poll arrives with `X-Forwarded-Proto: https` and `Host: updates.example.com`
- **THEN** every href in the response starts with `https://updates.example.com/`

### Requirement: Poll records device liveness cheaply
A poll SHALL record the device's last poll time. It SHALL write only when the stored value is empty or older than `HAWIER_POLL_TOUCH_INTERVAL_MS` (default 30000). The device address SHALL be taken from the first hop of `X-Forwarded-For` when present.

#### Scenario: Frequent polls do not rewrite the row
- **WHEN** a device polls twice within the touch interval
- **THEN** its last-poll time is written once

### Requirement: Deployment base
`GET deploymentBase/{actionId}` SHALL return `{id, deployment:{download, update, chunks[], maintenanceWindow?}, actionHistory?}` for an action of this target. It SHALL return 404 for an action of another target, a missing action, or a CANCELING action.

The `download` and `update` values SHALL be:

| Action type | download | update |
|---|---|---|
| FORCED | `forced` | `forced` |
| DOWNLOAD_ONLY | `forced` | `skip` |
| TIMEFORCED, forced time reached | `forced` | `forced` |
| TIMEFORCED, before forced time | `attempt` | `attempt` |
| SOFT | `attempt` | `attempt` |

The chunks SHALL be:
- one per software module of the set, ordered by module id
- `part` = the module type key, with legacy names `application`→`bApp` and `runtime`→`jvm`
- the module `name` and `version`
- only target-visible module metadata
- the module's artifacts, each with `filename`, `hashes` (`sha1`, `md5`, `sha256` when known), `size`, and `_links.download-http` plus `_links.md5sum-http`

`actionHistory=N` SHALL add the action's last N status messages, newest first. A negative N SHALL return all messages. A non-integer SHALL be rejected with 400.

#### Scenario: Multi-module distribution set
- **WHEN** a set with an `application` module and an `os` module, each with two artifacts, is deployed
- **THEN** the chunks are `bApp` and `os` with four artifacts in total, each with download and md5sum links

#### Scenario: Timeforced action passes its deadline
- **WHEN** a TIMEFORCED action's forced time passes
- **THEN** the next deploymentBase reports `update: forced` where it reported `attempt` before

### Requirement: Retrieval is recorded once
The first `GET deploymentBase` of an active action SHALL write exactly one RETRIEVED status. Later fetches, and concurrent fetches by devices sharing a controllerId, SHALL NOT write another.

#### Scenario: Two fetches
- **WHEN** the device fetches the same deploymentBase twice
- **THEN** the action has exactly one RETRIEVED status row

### Requirement: Deployment feedback state machine
`POST deploymentBase/{actionId}/feedback` SHALL apply the reported execution as follows:

| execution | written status | action after | target after |
|---|---|---|---|
| `proceeding`, `resumed`, `scheduled` | RUNNING | RUNNING, active | unchanged |
| `download` | DOWNLOAD | DOWNLOAD, active | unchanged |
| `downloaded`, non DOWNLOAD_ONLY action | DOWNLOADED | DOWNLOADED, active | unchanged |
| `downloaded`, DOWNLOAD_ONLY action | DOWNLOADED | DOWNLOADED, closed | set installed, IN_SYNC |
| `closed` + `success` | FINISHED | FINISHED, closed | set installed, IN_SYNC |
| `closed` + other result | ERROR | ERROR, closed | ERROR |
| `canceled`, `rejected` | (none) | 400; these belong to the cancel channel | (none) |

- A target that still has other active actions SHALL become PENDING instead of IN_SYNC.
- Feedback for an action of another target, or a missing action, SHALL get 404.
- Feedback for a closed action SHALL get 410 Gone.
- A non-null `code` SHALL be recorded as the action's last status code.
- Messages longer than 512 characters SHALL be truncated.

#### Scenario: Healthy update chain
- **WHEN** a device polls, fetches the deployment, reports proceeding, downloads, reports downloaded, then reports closed/success
- **THEN** the status history is RUNNING, RETRIEVED, RUNNING, DOWNLOAD, DOWNLOADED, FINISHED, and the target is IN_SYNC with the set installed

#### Scenario: Failure closes the action
- **WHEN** a device reports closed/failure
- **THEN** the action is ERROR and closed, the target is ERROR, and further feedback gets 410

#### Scenario: Feedback after operator force-quit
- **WHEN** an operator force-quits an action and the device then reports feedback for it
- **THEN** the server answers 410

### Requirement: Device status quota and repeat collapse
Per action, the server SHALL accept at most 800 status rows written by the device. Beyond that, device feedback SHALL get 403 with `errorCode` `hawkbit.server.error.repo.quotaExceeded`. The action SHALL stay active, and operator operations SHALL NOT be subject to the quota.

A device status identical to the last row (same status, code and message set) SHALL NOT add a row. It SHALL instead increase a repeat counter and the last-reported time. The management API SHALL expose these as `repeats` and `lastReportedAt`.

#### Scenario: Spamming device hits the quota
- **WHEN** a device has already written 800 status rows for an action and posts another feedback
- **THEN** it gets 403, and an operator can still force-quit the action

#### Scenario: Repeated proceeding is counted
- **WHEN** a device reports the same `proceeding` message 21 times
- **THEN** one status row exists with `repeats` = 20

### Requirement: Cancellation channel
`GET cancelAction/{id}` SHALL return `{id, cancelAction:{stopId:id}}` only for a CANCELING action of this target. Otherwise it SHALL return 404.

Feedback to the cancel channel SHALL be rejected as follows: missing action → 404, closed action → 410, action not CANCELING → 400. Otherwise:
- `canceled`, or any `closed`, SHALL close the action as CANCELED. The target SHALL become IN_SYNC if it has an installed set and no other active action remains, REGISTERED if it has no installed set.
- `rejected` SHALL write CANCEL_REJECTED and return the action to RUNNING. This is hawkBit-compatible behavior.
- Progress executions SHALL be recorded as CANCELING without changing state.

#### Scenario: Device complies with cancellation
- **WHEN** an operator cancels a running action and the device posts `canceled`
- **THEN** the action history ends RUNNING, RETRIEVED, CANCELING, CANCELED, and the action is closed

#### Scenario: Device rejects cancellation
- **WHEN** the device posts `rejected` on the cancel channel
- **THEN** the action is RUNNING and active again

### Requirement: Confirmation flow
With `user.confirmation.flow.enabled` on and no active auto-confirm, new assignments SHALL start in WAIT_FOR_CONFIRMATION. A per-rollout or per-filter `confirmationRequired` flag SHALL be combined with the tenant setting by logical AND.

`GET confirmationBase` SHALL return `{autoConfirm:{active, initiator?, remark?, activatedAt?}, _links}`:
- `activateAutoConfirm` or `deactivateAutoConfirm`, depending on the current state
- `confirmationBase/{id}` when the current action waits for confirmation

`GET confirmationBase/{id}` SHALL return `{id, confirmation:<deployment>, actionHistory?}` only for a waiting action. It SHALL NOT write RETRIEVED.

Feedback rules:
- `confirmed` SHALL move the action to RUNNING.
- `denied` SHALL record the denial and leave the action in WAIT_FOR_CONFIRMATION.
- A missing action SHALL get 404, a closed action 410, and an action that is not waiting 400.

`activateAutoConfirm` SHALL:
- answer 409 if auto-confirm is already active
- record the initiator (default: the controllerId) and the remark
- move every waiting action of the target to RUNNING

`deactivateAutoConfirm` SHALL be idempotent and answer 200.

#### Scenario: Deny then confirm
- **WHEN** a device denies a waiting action and later confirms it
- **THEN** the action stays WAIT_FOR_CONFIRMATION after the denial and becomes RUNNING after the confirmation

#### Scenario: Auto-confirm skips the wait
- **WHEN** a device has activated auto-confirm and a set is then assigned
- **THEN** the new action starts RUNNING

### Requirement: Controller attributes
`PUT configData` SHALL accept `{data:{key:value}, mode?}` with these modes:
- `merge` (the default) upserts the listed keys
- `replace` replaces all attributes
- `remove` deletes the listed keys

Each request SHALL be all-or-nothing. A key or value longer than 128 characters SHALL be rejected with 400. A result with more than 100 attributes SHALL be rejected with 403 and change nothing. A successful PUT SHALL clear the server's request for attributes, so the `configData` link disappears from the poll.

#### Scenario: Merge, replace, remove
- **WHEN** a device merges `{a:1,b:2}`, then replaces with `{c:3}`, then removes `c`
- **THEN** the attributes are `{a,b}`, then `{c}`, then empty

### Requirement: Installed base
`GET installedBase/{id}` SHALL answer 200 only for a FINISHED action of this target. Otherwise it SHALL answer 404.

`PUT installedBase {name, version}` SHALL record an offline installation of the named, non-deleted set. An unknown set SHALL get 404. If that set is already installed, the PUT SHALL be a no-op answering 200.

The recorded installation SHALL:
- create a closed FINISHED action
- mark the set installed (and assigned, if nothing was assigned)
- set the target IN_SYNC
- leave active actions untouched

#### Scenario: Offline report becomes visible
- **WHEN** a device PUTs installedBase for an existing set it was never assigned
- **THEN** the management API shows that set installed on the target, with a FINISHED action for it

### Requirement: Multi-assignment serving order
With `multi.assignments.enabled` on, assignments SHALL NOT supersede one another. DDI SHALL serve the active action with the highest weight first, and the oldest action among equal weights. The default weight SHALL be 1000. Without multi-assignment, re-assigning the set already assigned SHALL be a no-op.

#### Scenario: Weight beats age
- **WHEN** an action of weight 500 exists and a newer action of weight 700 is assigned
- **THEN** the poll links to the weight-700 action

#### Scenario: Partial completion leaves the target pending
- **WHEN** the heavier of two active actions finishes
- **THEN** the target is PENDING, and it becomes IN_SYNC when the second finishes

### Requirement: Maintenance windows
An assignment MAY carry a maintenance window. The window SHALL have:
- a Quartz-style cron schedule of 6–7 fields
- an `HH:mm:ss` duration greater than zero
- a fixed `±hh:mm` UTC offset

All three fields SHALL be given together or not at all. The management API SHALL reject with 400, creating no action:
- an incomplete window
- an IANA zone name instead of an offset
- a schedule with no open or upcoming window

The window SHALL be open if and only if some fire time F satisfies F ≤ now < F + duration.

For an action with a window, deploymentBase SHALL report `maintenanceWindow` as `available` or `unavailable`. While it is unavailable, `update` SHALL be `skip` and `download` SHALL keep its value.

When the window is closed and has a next start, the poll sleep SHALL be shortened to (next start − now) / `maintenanceWindowPollCount`:
- `maintenanceWindowPollCount` defaults to 3; invalid values fall back to 3
- the result is clamped between 30 seconds and the default sleep

#### Scenario: Closed window downloads but does not install
- **WHEN** a device fetches a deployment whose window is closed
- **THEN** it sees `maintenanceWindow: unavailable` and `update: skip`, may download, and the action stays active

#### Scenario: Window opens
- **WHEN** the window of a downloaded action opens
- **THEN** deploymentBase reports `available`, and the device installs and closes the action

#### Scenario: Invalid window rejected
- **WHEN** an assignment gives the window offset as `Europe/Berlin`
- **THEN** the management API answers 400 and no action exists

### Requirement: Overload answers 503 with Retry-After
When the node runs out of database connections or file descriptors, or a dependency is declared unavailable, the server SHALL answer 503 with:
- a `Retry-After` of 3–7 seconds
- `errorCode` `hawkbit.server.error.serviceUnavailable`

Other internal failures SHALL answer 500 without `Retry-After`.

#### Scenario: Pool exhausted
- **WHEN** a DDI request cannot obtain a database connection in time
- **THEN** the device receives 503 with a Retry-After between 3 and 7

### Requirement: DDI errors and journaling
DDI 5xx responses SHALL be journaled as incidents. DDI 4xx responses SHALL NOT be journaled. Both SHALL carry the incident number in the body.

#### Scenario: Bad token does not flood the journal
- **WHEN** a device repeatedly fails authentication
- **THEN** no incident rows are written for those 401s

### Requirement: When the server asks for attributes
The server SHALL ask a device for its controller attributes, by offering the
`configData` link in the poll, in these cases, as hawkBit 1.0 does:
- a target registered through plug-and-play
- a target created through `POST /rest/v1/targets`
- a target whose action has just ended FINISHED, or DOWNLOADED for a
  download-only action
- a target whose `requestAttributes` an operator has set through the
  management API

A successful `PUT configData` SHALL clear the request.

#### Scenario: Provisioned device reports on its first poll
- **WHEN** a target created through the management API polls for the first time
- **THEN** the poll offers `configData`

#### Scenario: Asked again after an update
- **WHEN** a device that had already reported its attributes finishes an update
- **THEN** its next poll offers `configData` again
