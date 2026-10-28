# rollout-engine Specification

## Purpose

The rollout lifecycle and the control-node engine that drives it. The engine
splits targets into groups through the rollout strategy extension, starts and
advances groups, applies strategy verdicts, auto-assigns through target filters,
and handles stop, delete, retry and dynamic groups.

## Requirements

### Requirement: Rollout creation
`POST /rest/v1/rollouts` SHALL require:
- `name`, unique per tenant (a duplicate gets 409)
- `distributionSetId` of a set that is not deleted and not invalid (otherwise 404)
- `targetFilterQuery`

It SHALL create the rollout and its declared groups in CREATING, in one transaction that also emits `rollout.created`. A rejected create SHALL leave no rollout and no event.

Groups:
- explicit `groups[]` SHALL take precedence over `amountGroups` (default 1, clamped to 1..500, equal percentages)
- the default success threshold SHALL be 100

`type` SHALL default to forced and `weight` to 1000. An explicit `strategy` that no loaded rollout module accepts SHALL get 400.

#### Scenario: Create answers creating
- **WHEN** a valid rollout is posted
- **THEN** the response is 201 with `status: creating`, and a `rollout.created` event is pending in the outbox

#### Scenario: Unknown strategy
- **WHEN** a rollout names `strategy: nonsense`
- **THEN** the server answers 400 and no rollout exists

### Requirement: Materialization through the strategy
On each tick, for each CREATING rollout, the engine SHALL:
1. Ask the strategy extension for a group plan from a snapshot.
2. Claim the rollout so that no two engines materialize it.
3. Apply the plan.

Group membership SHALL be computed by the host from the plan's selectors (share, filter, bucket, rest). No target SHALL be placed in two groups of one rollout. Declared groups the plan does not claim SHALL be removed. The rollout SHALL then become READY, or WAITING_FOR_APPROVAL when `rollout.approval.enabled` is on. Without a strategy extension, a rollout SHALL stay in CREATING.

#### Scenario: Two equal groups
- **WHEN** a rollout over 4 targets with two 50% groups is materialized by the classic strategy
- **THEN** it is READY with `totalTargets` 4 and two READY groups of 2

#### Scenario: Odd split
- **WHEN** 7 targets are split into 3 groups
- **THEN** the group sizes sum to 7

### Requirement: Approval
`POST .../approve` SHALL move a WAITING_FOR_APPROVAL rollout to READY. `POST .../deny` SHALL move it to APPROVAL_DENIED. Both SHALL record the decider and an optional remark, shown as `approveDecidedBy` and `approvalRemark`. Either call from another state SHALL get 409.

#### Scenario: Deny
- **WHEN** an operator denies a waiting rollout with a remark
- **THEN** the rollout is APPROVAL_DENIED and shows the decider and the remark

### Requirement: Start
`POST .../start` SHALL be accepted only for READY (otherwise 409) and SHALL set STARTING. The engine SHALL then, under a claim:
1. Create an inactive SCHEDULED action for every member of every group.
2. Activate the first group.
3. Set the rollout RUNNING.
4. Emit `rollout.started`.

A STARTING rollout whose `startAt` lies in the future SHALL wait until then.

#### Scenario: Only the first group is live
- **WHEN** a two-group rollout is started
- **THEN** group 1 is RUNNING, group 2 is SCHEDULED, and only group-1 devices see a deployment

### Requirement: Group activation
Activating a group SHALL make each of its scheduled actions active, as RUNNING or, when confirmation gates it, WAIT_FOR_CONFIRMATION. Without multi-assignment it SHALL supersede the target's other active actions, with the same handshake or autoclose rules as a manual assignment. It SHALL mark each target assigned and PENDING.

#### Scenario: Group activation supersedes
- **WHEN** a group is activated for a target that has an older active action
- **THEN** the older action is CANCELING (or CANCELED with autoclose) and the rollout's action is RUNNING

### Requirement: Evaluation and advance
For RUNNING rollouts the engine SHALL take per-group counts of every RUNNING group and ask the strategy for a verdict. The finished count of a group SHALL be its FINISHED actions, plus its DOWNLOADED actions only when the rollout is download-only: for any other action type DOWNLOADED means the device has not installed yet, and SHALL NOT count towards the success threshold. Only the engine SHALL apply the verdict, as follows:
- groups judged errored become ERROR
- a pause reason pauses the rollout and emits `rollout.paused`
- a group judged to have met its success condition SHALL start the group that follows it in id order, when that group is still SCHEDULED. The group itself SHALL stay RUNNING. When it is the last group of a dynamic rollout, the engine SHALL grow the rollout as for dynamic rollouts.

Meeting the success condition SHALL NOT finish a group. A RUNNING group SHALL become FINISHED when none of its actions is left in a non-terminal status. FINISHED, ERROR and CANCELED are terminal; for a download-only rollout DOWNLOADED is terminal too. This SHALL hold whether or not the group met its success condition, and is decided by the engine, not by the strategy. Several groups of one rollout MAY therefore be RUNNING at once.

With no group running, the engine SHALL activate the next SCHEDULED group in id order. With no group RUNNING or SCHEDULED left, a non-dynamic rollout SHALL become FINISHED and emit `rollout.finished`. A rollout SHALL NOT become FINISHED while an action of one of its groups that is not in ERROR is still in a non-terminal status.

#### Scenario: Success advances
- **WHEN** every action of group 1 finishes
- **THEN** in the same tick group 1 becomes FINISHED and group 2 RUNNING

#### Scenario: The threshold starts the next group, it does not end this one
- **WHEN** group 1 has a 50% success threshold and half of its actions have finished while the rest are still running
- **THEN** group 2 becomes RUNNING and group 1 stays RUNNING; group 1 becomes FINISHED once its remaining actions reach a final state

#### Scenario: Devices that never poll keep the rollout running
- **WHEN** a rollout over 42 targets in 3 groups with a 50% success threshold has every group past its threshold, and 6 of its targets never poll
- **THEN** the rollout stays RUNNING, the groups holding those 6 targets stay RUNNING, and `totalTargetsPerStatus` shows 36 finished and 6 running

#### Scenario: A settled group finishes below its threshold
- **WHEN** every action of a group reaches a final state, its success threshold is not met and its error threshold is not reached
- **THEN** the group becomes FINISHED and the rollout moves on to the next scheduled group

#### Scenario: Error threshold pauses
- **WHEN** 3 of 6 targets in a group with a 50% error threshold fail
- **THEN** the group is ERROR and the rollout PAUSED; at 2 of 6 it would have stayed RUNNING

#### Scenario: Completion
- **WHEN** all groups finish
- **THEN** the rollout is FINISHED with `totalTargetsPerStatus.finished` equal to the total

#### Scenario: Downloaded is not installed
- **WHEN** a soft rollout's group 1 has a 100% success threshold and its only device reports `downloaded` and defers installation
- **THEN** group 1 stays RUNNING and group 2 stays SCHEDULED; once the device installs, group 1 becomes FINISHED and group 2 RUNNING

#### Scenario: Download-only advances on downloaded
- **WHEN** a download-only rollout's group 1 has a 100% success threshold and its only device reports `downloaded`
- **THEN** group 1 becomes FINISHED and group 2 RUNNING

### Requirement: Operator controls
- `POST .../pause` (RUNNING only) SHALL pause the rollout and emit `rollout.paused` with reason `paused by <user>`.
- `POST .../resume` (PAUSED only) SHALL resume it. After an error pause, resuming SHALL proceed to the next scheduled group.
- `POST .../triggerNextGroup` (RUNNING only) SHALL force-activate the next scheduled group regardless of thresholds. With none left it SHALL answer 409.

Any lifecycle call from a wrong state SHALL get 409.

#### Scenario: Resume after error pause
- **WHEN** an error-paused rollout is resumed
- **THEN** it is RUNNING again

#### Scenario: Forced advance
- **WHEN** an operator triggers the next group while group 1 is below its success threshold
- **THEN** group 2 becomes RUNNING

### Requirement: Stop and delete
`POST .../stop` (RUNNING or PAUSED) SHALL set STOPPING. The engine SHALL then:
- cancel scheduled actions
- move active actions to CANCELING with `Update Server: rollout stopped`
- set the rollout STOPPED

`DELETE /rest/v1/rollouts/{id}` SHALL hard-delete a rollout that has no actions and is CREATING, READY, WAITING_FOR_APPROVAL or APPROVAL_DENIED. Otherwise it SHALL set DELETING. The engine SHALL then cancel the rollout as for stop and mark it DELETED.

#### Scenario: Stop a running rollout
- **WHEN** a running rollout is stopped
- **THEN** after the next tick it is STOPPED, its scheduled actions are canceled, and active ones are CANCELING

### Requirement: Retry
`POST .../retry` SHALL be accepted only for a FINISHED, STOPPED or PAUSED rollout with at least one ERROR action; otherwise it SHALL answer 409. It SHALL create a new rollout `<name>_retry_<timestamp>` in CREATING, with filter `controllerid=in=(<failed ids>)` and one 100% group.

#### Scenario: Retry the failures
- **WHEN** a finished rollout had 2 failed targets and is retried
- **THEN** a new rollout exists whose filter names exactly those 2 controller ids

### Requirement: Dynamic rollouts
A rollout created with `dynamic: true` SHALL NOT finish when its static groups are done. Newly matching targets that are in no group SHALL form groups named `<name>-dynamic-<n>`, which are created and activated immediately. The engine SHALL look for them when no group is running, and when the last group has met its success condition, so that a device of the last group that never polls does not keep newcomers out. The rollout SHALL stay RUNNING.

#### Scenario: Late-joining device
- **WHEN** a target matching a dynamic rollout's filter registers after the static groups finished
- **THEN** it is placed in a new dynamic group and receives the deployment

#### Scenario: Late-joining device while a straggler remains
- **WHEN** the last group of a dynamic rollout has met its success threshold, one of its targets never polls, and a new matching target registers
- **THEN** the new target is placed in a new dynamic group and receives the deployment, while the last group stays RUNNING

### Requirement: Target filter auto-assignment
On the engine loop, every target filter with an auto-assign set SHALL assign
that set to each matching target that meets two conditions:
- the target's assigned and installed sets both differ from it
- the target may receive it under the target type compatibility rules of
  assignment

An incompatible target SHALL be skipped without an error. It SHALL NOT stop
the assignment to the others. The assignment SHALL use the filter's action
type (default FORCED), its weight, its confirmation flag, and the operator who
configured it as initiator. Running again SHALL NOT create duplicate actions.

#### Scenario: Matching device is picked up
- **WHEN** a filter `attribute.hw==rev2` auto-assigns set S and a device reports `hw=rev2`
- **THEN** that device gets an action for S on the next tick, and a second tick creates none

#### Scenario: Incompatible device is skipped
- **WHEN** a filter auto-assigns a set of type `os-rev1` and it matches a `rev1`-typed and a `rev2`-typed target
- **THEN** only the `rev1` target gets an action, and the tick completes without an error

### Requirement: Engine placement and idempotence
The engine SHALL run only on the control node, ticking every 2 seconds. It SHALL claim each rollout with a non-blocking row lock, so that a rollout is never processed twice at once. Every transition SHALL be idempotent, so the engine resumes where it stopped after a restart. A failure on one rollout SHALL NOT stop the others in the same tick. The strategy extension SHALL be called outside the claim, and its verdict SHALL apply only to the rollout's current state.

#### Scenario: Restart mid-rollout
- **WHEN** the control node restarts while a rollout is RUNNING
- **THEN** the engine continues the rollout from its stored state without duplicating actions

### Requirement: Rollouts contain only compatible targets
A rollout SHALL count, place and assign only those filter-matching targets
that may receive its set under the target type compatibility rules of
assignment. This includes the typed-targets-only rule when
`target.type.required` is on. The rule SHALL cover:
- `totalTargets`
- the snapshot given to the strategy extension
- group membership
- dynamic groups

A matching target that is not compatible SHALL be left out silently. It
SHALL NOT fail the rollout.

#### Scenario: Mixed fleet
- **WHEN** a rollout of a set of type `os-rev1` has a filter matching one `rev1`-typed and one `rev2`-typed target
- **THEN** after materialization `totalTargets` is 1, and only the `rev1` target is in a group

#### Scenario: Late incompatible device
- **WHEN** a `rev2`-typed target matching a dynamic rollout of an `os-rev1` set registers
- **THEN** no dynamic group is created for it
