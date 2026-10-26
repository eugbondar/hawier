# Rollouts

> The same distribution set to many devices, in waves, with rules about when to continue and when to stop.

## Rollouts {#rollouts}

A rollout takes a distribution set, a query that picks the devices, and a number of waves. It divides the matching devices into [deploy groups](/help/deployment/rollout-groups#deploy-groups), gives the first group its jobs, and then advances group by group as each one meets its success threshold.

If failures reach the error threshold, the rollout stops itself rather than carrying the problem into the next wave. That is the whole point of using one instead of assigning to everybody at once.

Creating a rollout sends nothing. It is built, it waits at [ready](/help/glossary#ready), and a person starts it.

## Anatomy {#anatomy}

Distribution set :: What every device in the rollout will be told to install. Must be complete and valid.
Target filter :: A query over devices. Whoever matches it when the rollout is built is who the rollout takes.
Groups :: How many waves to divide those devices into.
Thresholds :: How much of a wave must succeed before the next one starts, and how much may fail before everything pauses.
Action type :: How hard each device is pushed — see [action types](/help/deployment/action-types#action-types).

## Progress {#progress}

The bars on a rollout's page count its devices by state: not started, running, finished, error, cancelled. They add up to the rollout's total, and the total was fixed when the rollout was built.

A device that matched the filter after the rollout was created is **not** in it — unless the rollout is [dynamic](/help/deployment/dynamic-rollouts#dynamic-rollouts).

## Retrying {#retrying}

**Retry** on a finished or stopped rollout creates a new one aimed at exactly the devices that failed, in a single group. The original is left as it was, as the record of what happened.

## Deleting {#deleting}

A rollout that never started is removed. One that created actions is marked deleted and disappears from the lists, because its actions still refer to it.

## Latest {#latest}

Every rollout, newest first.

## Running {#running}

Rollouts currently advancing through their groups.

## Needs attention {#needs-attention}

Rollouts that stopped moving on their own: paused, or with a group in error. The list to check after starting something.

## Paused {#paused}

Rollouts stopped in place. Either somebody paused it, or an error threshold did — see [paused](/help/glossary#paused).

## Finished {#finished}

Rollouts that completed every group.

## Stopped {#stopped}

Rollouts that were stopped and cancelled their outstanding jobs.

## Draft ready {#draft-ready}

Rollouts that are built but have never been started. Nothing has been sent to any device in these.

## Recently changed {#recently-changed}

Rollouts touched most recently, whether by the engine or by a person.
