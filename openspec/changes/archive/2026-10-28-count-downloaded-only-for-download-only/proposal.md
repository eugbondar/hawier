# Proposal

## Why

The engine counts every DOWNLOADED action as a success when it gives the
strategy a group's numbers, whatever the rollout's action type. For a forced,
soft or time-forced rollout DOWNLOADED is an intermediate state: the device
has the bytes and has not installed them. A group whose devices only
downloaded can therefore meet its success threshold and start the next group
before a single device has installed. hawkBit 1.0
(`ThresholdRolloutGroupSuccessCondition`) counts DOWNLOADED only for a
DOWNLOAD_ONLY rollout and FINISHED otherwise. The divergence was recorded by
`finish-rollout-groups-on-completion` and is fixed here.

## What Changes

- The finished count a strategy judges a group by is its FINISHED actions,
  plus its DOWNLOADED actions only when the rollout is download-only.
- A soft rollout whose devices defer installation (report `downloaded` and
  wait) no longer advances on those devices; it advances once they install,
  or when an operator triggers the next group.
- A download-only rollout still advances on DOWNLOADED, as before.
- The manual stops saying that downloaded devices count towards success for
  every rollout.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities
- `rollout-engine`: "Evaluation and advance" states what counts as finished
  in the per-group counts given to the strategy.

## Impact

- `apps/server/src/rollout.rs` (per-group counts for the strategy).
- Integration tests in `apps/server/tests/it/rollout.rs`.
- Manual: `apps/ui/docs/deployment/rollout-groups.md`.
- Observable through Mgmt only as timing: a non-download-only rollout's next
  group starts later when devices stop at `downloaded`. No wire change.
- The rollout engine is touched: `make load` is compared with
  `bench/load-history.csv` (a degradation above 20% at the same parameters is
  a regression). The count query reads one more column per rollout.
