# Spec Delta

## MODIFIED Requirements

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
| `target.type.required` | false | untyped targets receive no assignment (hawier addition, hardened profile) |

Every key in the table except `target.type.required` SHALL be a hawkBit key
with hawkBit's meaning.

Boolean values SHALL be true for `true`, `TRUE` or `1`.

#### Scenario: Defaults in force
- **WHEN** a fresh tenant assigns a set
- **THEN** the set is locked, no confirmation is required, supersede uses the cancel handshake, and an untyped target accepts the set

#### Scenario: Hardened key listed
- **WHEN** `GET /rest/v1/system/configs` is called on a fresh tenant
- **THEN** the response contains `target.type.required` with value `false` and `global: true`
