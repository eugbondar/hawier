# Spec Delta

## MODIFIED Requirements

### Requirement: Evaluation and advance
For RUNNING rollouts the engine SHALL take per-group counts of every RUNNING group and ask the strategy for a verdict. Only the engine SHALL apply the verdict, as follows:
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

### Requirement: Dynamic rollouts
A rollout created with `dynamic: true` SHALL NOT finish when its static groups are done. Newly matching targets that are in no group SHALL form groups named `<name>-dynamic-<n>`, which are created and activated immediately. The engine SHALL look for them when no group is running, and when the last group has met its success condition, so that a device of the last group that never polls does not keep newcomers out. The rollout SHALL stay RUNNING.

#### Scenario: Late-joining device
- **WHEN** a target matching a dynamic rollout's filter registers after the static groups finished
- **THEN** it is placed in a new dynamic group and receives the deployment

#### Scenario: Late-joining device while a straggler remains
- **WHEN** the last group of a dynamic rollout has met its success threshold, one of its targets never polls, and a new matching target registers
- **THEN** the new target is placed in a new dynamic group and receives the deployment, while the last group stays RUNNING
