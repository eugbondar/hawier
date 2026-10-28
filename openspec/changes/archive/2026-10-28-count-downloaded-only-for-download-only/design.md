# Design

## Context

The engine builds `GroupStats` for the strategy from the actions of each
RUNNING group, counting FINISHED and DOWNLOADED together as `finished`. The
strategy never sees the action type, so the distinction can only be made on
the host side. Group completion already distinguishes the two (DOWNLOADED is
terminal only for download-only rollouts, `finish-rollout-groups-on-completion`);
the success count did not.

### Divergence (recorded by the previous change, fixed here)

hawkBit 1.0, `ThresholdRolloutGroupSuccessCondition.eval`: the counted status
is `DOWNLOADED` when the rollout's action type is DOWNLOAD_ONLY, `FINISHED`
otherwise. hawier counted both for every rollout. No spec SHALL said either
way ("finished share"), so this is a code defect and the delta spells the
rule out.

## Goals / Non-Goals

**Goals:**
- For forced, soft and time-forced rollouts only FINISHED counts.
- For download-only rollouts DOWNLOADED counts.

**Non-Goals:**
- Changing what the strategy receives (`GroupStats` keeps its shape).
- `totalTargetsPerStatus`, which already shows DOWNLOADED as running.

## Decisions

**FINISHED still counts in a download-only rollout.** hawkBit counts only
DOWNLOADED there, so a download-only action a device closes as FINISHED is
not a success in hawkBit. hawier keeps counting it: a device that installed
has certainly downloaded, and not counting it would only delay the next group
until the group settles (FINISHED is terminal, so the group finishes and the
"nothing running" fallback starts the next one anyway). This is a deliberate
superset of hawkBit's rule, stated in the delta spec.

**Decide in the host's count query.** The action type is read with the
groups (one join to `sp_rollout`) and applied while folding the per-status
counts. Alternative: send the action type in `RolloutStats` and let modules
decide — rejected, every strategy would have to repeat the same rule, and a
module that forgot it would bring the defect back.

## Risks / Trade-offs

- [A soft rollout with devices that defer installation stops advancing where
  it used to advance] → That is the fix: the old advance was on devices that
  had not installed. `triggerNextGroup` remains for an operator who wants it.
