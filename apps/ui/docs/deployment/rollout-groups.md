# Deploy groups

> The waves a rollout is divided into, and the rules that move it from one to the next.

## Deploy groups {#deploy-groups}

A group is a slice of the rollout's devices. Groups are built when the rollout is created, and every device the rollout took belongs to exactly one — the slices never overlap.

The table on a rollout's page lists each group with its state, how many devices it holds, and its two thresholds. A group opens a page listing what it did to each of its devices.

## What a group does {#what-a-group-does}

Nothing, until it is activated. Its jobs exist from the moment the rollout starts, but they are [scheduled](/help/glossary#scheduled): not served to devices, not visible to them.

Activating a group, for each of its devices:

1. Supersedes whatever else that device had outstanding — see [superseding](/help/deployment/assignment#superseding).
2. Puts the job into [wait_for_confirmation](/help/glossary#wait-for-confirmation) if the confirmation flow applies and this group has not waived it, otherwise into [running](/help/glossary#running).
3. Sets the device's assigned distribution set and moves it to [pending](/help/glossary#pending).

The devices find out at their next poll.

## Thresholds {#thresholds}

Both are percentages of the group's own device count, not of the whole rollout.

Success threshold :: How much of the group must finish before the next group is activated. 100 means everybody; 90 means the rollout carries on with a tenth of the wave still outstanding.
Error threshold :: How much of the group may fail before the rollout stops. Reaching it puts the group into [error](/help/glossary#error) and [pauses](/help/glossary#paused) the whole rollout.

Devices that have [downloaded](/help/glossary#downloaded) count towards success as well as those that have finished, which is what makes a download-only rollout able to complete at all.

A group with no devices is finished immediately. Otherwise a query that matched nothing would wedge the rollout forever.

## Choosing the numbers {#choosing-thresholds}

A success threshold of 100 means one permanently offline device stops the rollout. That is usually not what you want: 90 or 95 lets the wave complete and leaves the stragglers to be dealt with afterwards, from the [pending update](/help/inventory/targets#pending-update) list.

An error threshold that is too generous defeats the purpose. If you would abandon the rollout after five failures in a hundred, the threshold is 5.

## Group states {#group-states}

[creating](/help/glossary#creating) :: Being built.
[ready](/help/glossary#ready) :: Built; its devices are known.
[scheduled](/help/glossary#scheduled) :: Its jobs exist and are invisible to their devices.
[running](/help/glossary#running) :: Activated; its devices can see their jobs.
[finished](/help/glossary#finished) :: Met its success threshold.
[error](/help/glossary#error) :: Met its error threshold, and paused the rollout.

## Per-group filters {#per-group-filters}

A group can be given a query of its own through the API. **The built-in strategy ignores it.** It is accepted and stored and has no effect on which devices land in which group — see [sharp edges](/help/reference/sharp-edges#per-group-filters).

## Success and error actions {#group-actions}

Groups also record a "success action" and an "error action". These are constants: on success the rollout always continues to the next group, on error it always pauses. They are shown because the API returns them, not because they can be changed.
