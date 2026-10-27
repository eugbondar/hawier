# Proposal

## Why

A rollout over 42 targets in 3 groups with `successCondition: THRESHOLD 50`,
where 6 devices never polled, showed `status: finished` with
`totalTargetsPerStatus` finished 36, running 6, and all three groups
`finished`. The engine treats "the group met its success condition" as "the
group is finished", so a rollout reports done while devices it is responsible
for are still being updated. hawkBit 1.0 does not: there the success condition
only fires the success action (start the next group); a group becomes FINISHED
only when none of its actions is left non-terminal, and the rollout finishes
only when no group is RUNNING or SCHEDULED. The classic strategy is promised
to reproduce hawkBit's thresholds, so hawier is the side that is wrong.

## What Changes

- A group whose success condition is met starts the group after it and stays
  RUNNING; it becomes FINISHED only when none of its actions is still in a
  non-terminal status (FINISHED, ERROR, CANCELED are terminal; DOWNLOADED too
  for a download-only rollout). Completion is a fact the host observes, not a
  strategy judgement.
- A group whose actions have all reached a final state becomes FINISHED even
  when its success threshold was never met (failures below the error
  threshold), so such a rollout moves on instead of staying RUNNING forever.
- Several groups may now be RUNNING at once: the one still settling its
  stragglers and the one its success started. Strategies see all of them.
- A rollout is FINISHED only when no group is RUNNING or SCHEDULED, so a
  rollout with a device that never polls stays RUNNING, as in hawkBit.
- A dynamic rollout grows a new group once its last group meets its success
  condition, not only when nothing is running, so a straggler in the last
  group does not lock late devices out.
- The strategy verdict's field `finished` is renamed `succeeded` (the old
  name is still accepted on the wire), so the contract says what the module
  decides.
- The in-console manual's description of a group's `finished` state and of
  the success threshold follows the new meaning.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities
- `rollout-engine`: "Evaluation and advance" separates "start the next group"
  from "group finished"; "Dynamic rollouts" grows after the last group's
  success, not only when nothing runs.
- `first-party-extensions`: "Classic rollout strategy module" judges that a
  group met its success condition rather than that it is finished.

## Impact

- `apps/server/src/rollout.rs` (verdict application, group completion, next
  group, dynamic growth), `libs/api/src/ext.rs` (`RolloutVerdict`),
  `apps/ext-rollout-classic`, `apps/ext-rollout-bucket` (field rename only).
- Integration tests in `apps/server/tests/it/rollout.rs`: a reproduction of
  the 42-target scenario.
- Manual: `apps/ui/docs/deployment/rollout-groups.md`,
  `apps/ui/docs/glossary.md`.
- Observable through Mgmt: rollouts and groups stay `running` longer;
  `deploygroups` may list two groups `running`. No wire field is added.
- The rollout engine is touched: `make load` is compared with
  `bench/load-history.csv` (a degradation above 20% at the same parameters is
  a regression). The extra per-tick work is one indexed `UPDATE` per running
  rollout.
