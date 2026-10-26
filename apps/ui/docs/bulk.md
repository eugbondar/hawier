# Bulk operations

> Doing one thing to many rows at once.

## Bulk operations {#bulk-operations}

Select rows in a list — see [selecting rows](/help/console/lists#selection) — then right-click over the selection, or use the counter in the toolbar. The operations offered depend on the list.

Everything here applies to every selected row. Operations that destroy something ask first and tell you how many rows are involved; read that number before confirming, because a selection can be larger than the page you are looking at.

## Assign DS {#assign-ds}

Gives every selected device the chosen distribution set. One [action](/help/deployment/actions#actions) per device, and each supersedes whatever that device had outstanding.

This is the blunt instrument: everything starts at once, with no waves and no thresholds. For more than a handful of devices, use a [rollout](/help/deployment/rollouts#rollouts) instead.

## Add tag {#tag}

Adds a [tag](/help/inventory/target-tags#tags) to every selected row. Rows that already carry it are unchanged. Nothing is installed or cancelled — it is a label.

## Remove tag {#untag}

Removes a tag from every selected row. The tag itself still exists.

## Set group {#set-group}

Puts every selected device into one [group path](/help/inventory/target-groups#groups). A device is in one group at a time, so this replaces whatever it had.

The field accepts a path that does not exist yet — typing a new one creates it by using it.

## Clear group {#clear-group}

Removes the group from every selected device, leaving them ungrouped.

## Cancel actions {#cancel-actions}

Asks every selected device to stop what it is doing. Each outstanding action goes to [canceling](/help/glossary#canceling) and waits for its device to acknowledge, which a device may [refuse](/help/glossary#cancel-rejected).

This does not undo anything already installed.

## Invalidate {#invalidate}

Marks every selected distribution set unusable: no new assignment, no new rollout. Devices that already have it keep it — see [invalidation](/help/repository/locking#invalidation).

Use it when a release turns out to be bad. It is the fastest way to stop a bad set spreading further.

## Start {#start}

Starts every selected [ready](/help/glossary#ready) rollout. Each creates its jobs and activates its first group.

## Pause {#pause}

Pauses every selected running rollout. Devices already working carry on; no further group is activated.

## Resume {#resume}

Resumes every selected paused rollout, from where it stopped. If one was paused by an error threshold, resuming carries it into the next wave with those failures already counted.

## Stop {#stop}

Cancels every outstanding job in the selected rollouts and ends them. Terminal — a stopped rollout can be [retried](/help/deployment/rollout-lifecycle#retry), not resumed.

## Delete {#delete}

Removes the selected rows. Objects that something still refers to are marked deleted and hidden rather than removed, so the things pointing at them keep working.

Deleting is not an undo. Deleting a rollout does not recall what it installed; deleting a tag does not change the devices that carried it.
