# Roll out an update to a fleet

> Doing the same thing to a thousand devices, in waves, with a way to stop.

## Why not just assign it {#why-not-just-assign}

You can select every device in a list and assign a distribution set to all of them at once. For a handful of devices that is the right tool. For a fleet it is not: everything starts at the same moment, and if the update is bad you find out when the whole fleet is broken.

A **rollout** does the same work in waves and watches the result of each wave before starting the next.

## Step by step {#step-by-step}

1. **Have a complete distribution set.** A rollout will not start from an incomplete or invalidated set.
2. **Decide who.** A rollout aims at a [query over devices](/help/reference/rsql#the-query-language) — for example `updateStatus==in_sync;attribute.hw==rev2`. Try the query on the [targets](/targets) list first: the number of rows it matches is the number of devices the rollout will take.
3. **Create the rollout.** [Rollouts](/rollouts) then **New**. Name it, pick the set, paste the query, and choose how many groups to divide it into. Every field is explained in [the rollout form](/help/forms/rollout#the-form).
4. **Set the thresholds.** **Success threshold** is the share of a group that must finish before the next group starts. **Error threshold** is the share that may fail before the whole rollout pauses itself. See [thresholds](/help/deployment/rollout-groups#thresholds).
5. **Check the groups.** The rollout is built in the background; when it reaches [ready](/help/glossary#ready), its page lists the groups and how many devices each one took. Nothing has been sent yet.
6. **Start it.** The first group's devices get the job at their next poll. Every other group's jobs exist but are invisible to their devices until their group is activated.
7. **Watch.** The progress bars on the rollout page count devices by state. Groups activate one after another as each meets its success threshold.

## Choosing the number of groups {#choosing-groups}

The first group is your canary. Ten groups over a thousand devices means the first hundred devices carry the risk; a hundred groups means the first ten do, and the rollout takes ten times as many waves to finish.

A common shape is a small first wave and larger ones after it. The console's create form divides the fleet evenly; uneven waves are available through the API.

## When something goes wrong {#when-it-goes-wrong}

If a group's failures reach the error threshold, that group goes to [error](/help/glossary#error) and the rollout [pauses](/help/glossary#paused) itself. Devices already working carry on; no new group starts.

At that point you decide: **Resume** to continue anyway, **Stop** to cancel everything outstanding, or fix the distribution set and start a new rollout. **Retry** on a finished or stopped rollout builds a new one aimed only at the devices that failed.

> Pausing does not recall an update. Devices that already installed it keep it. To move a fleet back, roll out the previous distribution set as a new rollout.
