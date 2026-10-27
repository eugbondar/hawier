# Design

## Context

The engine asks the strategy for a `RolloutVerdict` over the RUNNING groups
and applies it. The verdict's `finished` list was documented as "groups that
have met their success condition", but the engine set every group on it to
FINISHED. With nothing RUNNING it then started the next SCHEDULED group, and
with nothing SCHEDULED either it finished the rollout. So the success
threshold did two jobs: it started the next group *and* closed the current
one, and the last group's threshold closed the rollout.

### Divergence (found by this change)

`rollout-engine`, requirement "Evaluation and advance", said "groups judged
finished become FINISHED", and `first-party-extensions` said the classic
module judges a group "finished" when its finished share reaches the success
threshold. Read together, these SHALLs put a rollout into FINISHED while its
actions were still RUNNING. hawkBit 1.0, which the classic strategy is promised
to reproduce, does not do this. From `JpaRolloutExecutor` and
`StartNextGroupRolloutGroupSuccessAction` in hawkBit 1.0.0:

- for each RUNNING group: error condition → error action (pause); otherwise
  success condition → success action, which with `NEXTGROUP` starts the
  SCHEDULED child group and leaves this group as it is;
- then, independently, `isRolloutGroupComplete`: no action of the group in a
  status outside {ERROR, FINISHED, CANCELED} (plus DOWNLOADED for a
  DOWNLOAD_ONLY rollout) → the group becomes FINISHED;
- with no group RUNNING, `triggerNextGroup` starts the next one;
- the rollout is FINISHED when no group is RUNNING or SCHEDULED.

This was a defect in the SHALLs, not a decision. It is fixed here, through
this change, and not written into the main spec as it was.

### A second divergence, not fixed here

The host counts DOWNLOADED actions as finished for the success threshold of
every rollout (`running_group_stats`). hawkBit counts DOWNLOADED only for a
DOWNLOAD_ONLY rollout and FINISHED otherwise, so in hawier a forced group
whose devices have downloaded but not installed can meet its threshold early.
It changes when the next group starts, not when anything finishes, and gets
its own change.

## Goals / Non-Goals

**Goals:**
- "Start the next group" and "this group is finished" are two separate
  transitions, with hawkBit 1.0's rules for each.
- No rollout is FINISHED while an action of one of its non-ERROR groups is
  still active.

**Non-Goals:**
- Writing `sp_rollout_group.parent` (hawkBit's chain). hawier orders groups
  by id, as it already does for "next scheduled group".
- The DOWNLOADED counting above; error threshold arithmetic; per-group target
  filters (API_NOTES §7).

## Decisions

**Completion belongs to the host, not to the strategy.** Whether every action
of a group is in a terminal status is a fact about rows the host owns; a
strategy that could withhold it could keep a settled group RUNNING forever.
The verdict stays a judgement (`succeeded`, `errored`, `pause`), and the
engine marks complete groups FINISHED with one `UPDATE ... WHERE NOT EXISTS`
over `sp_action` on the claimed rollout, served by
`sp_idx_action_rollout_group_status` (tenant, rollout_group, status).
Alternative considered: send "active actions" in `GroupStats` and let the
module decide — rejected, it gives a module a transition the host must own.

**Rename the verdict field to `succeeded`, keep `finished` as a serde
alias.** The old name is what led the engine astray; a module built against
the old `hawier-api` still serializes `finished` and still works with the new
meaning. Alternative: keep the name and fix only the engine — cheaper, but the
next reader of the struct is told the wrong thing by its name.

**The successor is the next group by id.** For a succeeded group G the engine
takes the first group of the rollout with an id greater than G; if it is
SCHEDULED it is activated, otherwise nothing happens. This is hawkBit's
"start my SCHEDULED child", given that hawier's groups are created in order.
It is idempotent: on the next tick G succeeds again and its successor is no
longer SCHEDULED. When G has no successor and the rollout is dynamic, the
engine tries to grow a dynamic group, which is what hawkBit's pre-created
dynamic group gets from G's `NEXTGROUP`.

**Order inside the claimed transaction:** errored groups → ERROR and pause
(return); succeeded groups → start successors / grow; complete RUNNING groups
→ FINISHED; then the existing state machine (nothing RUNNING → next
SCHEDULED, nothing SCHEDULED → finish or grow). A group errored in this
verdict is not also treated as succeeded. So when every action of group 1
finishes, group 2 is RUNNING and group 1 FINISHED in the same tick, as the
"Success advances" scenario requires.

**A settled group below its success threshold finishes.** hawkBit does it, and
otherwise a group with failures under the error threshold stays RUNNING with
nothing left to wait for, and the rollout never moves on.

## Risks / Trade-offs

- [Rollouts that used to reach FINISHED stay RUNNING while a device never
  polls] → That is the intended behaviour. The manual says so, and that a
  threshold below 100 lets the next group start, not the rollout end.
- [Resume after an error pause, while an earlier group is still RUNNING,
  starts the next group only when that earlier group settles or meets its
  threshold again] → hawkBit behaves the same (its "nothing running →
  trigger next" fallback). The error-paused group itself is ERROR and never
  succeeds.
- [Dynamic rollouts with a succeeded but unsettled last group run the
  growth query every tick] → Before, the same query ran every tick once the
  last group was FINISHED; the cost is unchanged.
- [Engine tick cost] → One extra indexed `UPDATE` per RUNNING rollout per
  tick; `make load` is compared with `bench/load-history.csv`.

## Migration Plan

No schema change. Rollouts already marked FINISHED with active actions stay
as they are; groups already FINISHED are not reopened. Rollback is reverting
the commit; the serde alias makes the extension modules interchangeable
either way.
