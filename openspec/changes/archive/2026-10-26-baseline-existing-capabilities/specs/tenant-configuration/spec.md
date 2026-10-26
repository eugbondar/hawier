# Spec Delta

## Purpose

Per-tenant configuration keys, compatible with hawkBit's system configuration
API, that change server behavior: their defaults, how they are read and
written, and how fast changes take effect.

## ADDED Requirements

### Requirement: Effective keys and defaults
The server SHALL honor these tenant keys:

| Key | Default | Effect |
|---|---|---|
| `pollingTime` | `00:05:00` | DDI poll sleep; liveness window |
| `pollingOverdueTime` | `00:05:00` | liveness window |
| `maintenanceWindowPollCount` | `3` | poll shortening before a maintenance window |
| `authentication.targettoken.enabled` | `HAWIER_DDI_TARGETTOKEN_ENABLED` (true) | target token auth |
| `authentication.gatewaytoken.enabled` | `HAWIER_DDI_GATEWAYTOKEN_ENABLED` (false) | gateway token auth |
| `authentication.gatewaytoken.key` | empty | gateway key |
| `rollout.approval.enabled` | false | rollouts wait for approval |
| `repository.actions.autoclose.enabled` | false | supersede closes at once |
| `user.confirmation.flow.enabled` | false | device confirmation |
| `multi.assignments.enabled` | false | concurrent actions by weight |
| `implicit.lock.enabled` | true | lock set and modules on use |
| `action.cleanup.auto.expiry` | unset | action cleanup age (ms) |
| `action.cleanup.auto.status` | unset | action cleanup statuses |

Boolean values SHALL be true for `true`, `TRUE` or `1`.

#### Scenario: Defaults in force
- **WHEN** a fresh tenant assigns a set
- **THEN** the set is locked, no confirmation is required, and supersede uses the cancel handshake

### Requirement: System configuration API
`GET /rest/v1/system/configs` SHALL return every known key. A stored value SHALL be shown with `global: false`; otherwise the default SHALL be shown with `global: true`. Values SHALL be typed: booleans as booleans, integer strings as numbers.

Writes:
- `PUT /rest/v1/system/configs/{key}` SHALL set one key, from `{"value": x}` or a bare scalar
- `PUT /rest/v1/system/configs` SHALL set several keys from one object
- `DELETE /rest/v1/system/configs/{key}` SHALL revert a key to its default, answering 204
- a non-scalar value SHALL get 400

#### Scenario: Set and revert
- **WHEN** `multi.assignments.enabled` is set to true and then deleted
- **THEN** it reads `true, global: false`, then `false, global: true`

### Requirement: Malformed values do not take lists down
A malformed `pollingTime` or `pollingOverdueTime` SHALL NOT make any list fail. The server SHALL fall back to the default window. Examples of malformed values are `5 minutes`, `300` and an empty string.

#### Scenario: Bad polling time
- **WHEN** `pollingTime` is stored as `5 minutes`
- **THEN** the target list still answers 200

### Requirement: Change propagation
A configuration write SHALL take effect immediately on the node that wrote it. Device-path reads MAY be served from a per-node cache. Every other node SHALL observe the change within `HAWIER_TENANT_CACHE_TTL_MS` (default 5000; 0 disables the cache).

#### Scenario: Gateway toggle on another node
- **WHEN** the gateway token is enabled through the control node
- **THEN** a sub node accepts gateway-token polls within the cache TTL
