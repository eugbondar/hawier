# Spec Delta

## Purpose

How assigning a distribution set to a target creates and governs actions.
Covers action types, supersede and autoclose, confirmation gating, implicit
locking, multi-assignment, operator cancellation, force-quit, escalation, and
invalidation.

## ADDED Requirements

### Requirement: Assignment creates an active action
Assigning a distribution set to a target SHALL be serialized per target. It SHALL, in one transaction:
- create one active action in RUNNING, or WAIT_FOR_CONFIRMATION when gated by confirmation
- write a first status `Update Server: Assignment initiated by user '<user>'`
- set the target's assigned set
- mark the target PENDING
- emit `assignment.created`

The action type SHALL default to FORCED, and the weight SHALL default to 1000.

#### Scenario: Forced assignment
- **WHEN** an operator assigns a set to a registered target with confirmation off
- **THEN** one active RUNNING action exists, the target is PENDING, and the set is the target's assigned set

### Requirement: Assignment API reports what it did
`POST /rest/v1/targets/{controllerId}/assignedDS` and `POST /rest/v1/distributionsets/{id}/assignedTargets` SHALL answer `{assigned, alreadyAssigned, total, assignedActions[{id, _links.self}]}`.

Assigning the set a target already has SHALL be a no-op counted in `alreadyAssigned`, unless multi-assignment is enabled. An assignment naming a deleted set SHALL get 404.

#### Scenario: Repeat assignment
- **WHEN** the same set is assigned to the same target twice with multi-assignment off
- **THEN** the first response reports `assigned: 1` and the second `assigned: 0, alreadyAssigned: 1`

### Requirement: Supersede with cancel handshake by default
Without multi-assignment, a new assignment SHALL supersede the target's earlier active actions. An action that is already CANCELING SHALL be left alone.

By default each superseded action SHALL become CANCELING, stay active, and get the message `Update Server: cancel obsolete action due to new update`. The device is then offered the cancel channel.

With `repository.actions.autoclose.enabled`, each superseded action SHALL instead be closed as CANCELED at once, without a handshake.

#### Scenario: Default supersede
- **WHEN** set B is assigned while set A's action is active
- **THEN** A's action is CANCELING and active, and the poll offers `cancelAction` for it

#### Scenario: Autoclose
- **WHEN** autoclose is enabled and set B is assigned over an active A
- **THEN** A's action is CANCELED and inactive, and the poll offers only B's deployment

### Requirement: Implicit lock
With `implicit.lock.enabled` (default true), assigning a set or creating a rollout for it SHALL lock the set and all its software modules against modification.

#### Scenario: Lock on assignment
- **WHEN** an unlocked set is assigned with the default configuration
- **THEN** the set and each of its modules are locked

#### Scenario: Lock disabled
- **WHEN** `implicit.lock.enabled` is false and the set is assigned
- **THEN** the set stays unlocked

### Requirement: Confirmation gate
A new action SHALL start in WAIT_FOR_CONFIRMATION exactly when all three hold:
- `user.confirmation.flow.enabled` is on
- the target has no active auto-confirm
- the assignment's `confirmationRequired` is not false

Activating auto-confirm, through DDI or the management API, SHALL move the target's waiting actions to RUNNING. Activating it twice SHALL get 409.

#### Scenario: Explicit opt-out
- **WHEN** the flow is on and an assignment sets `confirmationRequired: false`
- **THEN** the action starts RUNNING

### Requirement: Operator cancellation and force-quit
`DELETE /rest/v1/targets/{controllerId}/actions/{actionId}` SHALL soft-cancel the action: it becomes CANCELING and the call answers 204. `?force=true` SHALL close it as CANCELED at once. A soft cancel of an action that is already CANCELING SHALL get 410. `DELETE /rest/v1/targets/{controllerId}/actions` SHALL cancel every active action of the target the same way and answer `{canceled, total}`.

A canceled action SHALL be shown by the management API with `status: finished`, `detailStatus: canceled`, `type: cancel`.

#### Scenario: Force-quit under a working device
- **WHEN** an operator force-quits an action the device is installing
- **THEN** the action is CANCELED immediately and the device's next feedback gets 410

### Requirement: Escalation to forced
`PUT /rest/v1/targets/{controllerId}/actions/{actionId}` with `{"forceType":"forced"}` SHALL turn an active action into FORCED. Any other value SHALL get 400. An inactive action SHALL get 404.

#### Scenario: Deferring device is forced
- **WHEN** a device defers a SOFT action and the operator escalates it to forced
- **THEN** the device installs on its next cycle and the action ends FINISHED

### Requirement: Action deletion
`DELETE /rest/v1/actions/{id}` SHALL delete only an inactive action, answering 204. An active action SHALL get 409 and a missing one 404.

#### Scenario: Active action cannot be deleted
- **WHEN** an operator deletes an action that is still RUNNING
- **THEN** the server answers 409 and the action remains

### Requirement: Distribution set invalidation
`POST /rest/v1/distributionsets/{id}/invalidate` SHALL mark the set invalid.

`actionCancelationType` SHALL control its actions:
- `none` (the default) leaves them alone
- `soft` makes them CANCELING
- `force` makes them CANCELED

With `cancelRollouts: true`, every rollout of the set not yet FINISHED, STOPPED or STOPPING SHALL move to STOPPING. The response SHALL report `affectedActions` and `affectedRollouts`.

#### Scenario: Forced invalidation
- **WHEN** a set with two active actions is invalidated with `actionCancelationType: force`
- **THEN** both actions are CANCELED and the response reports two affected actions
