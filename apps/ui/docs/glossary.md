# Status glossary

> Every status word the console prints, and what it means where you saw it.

## Status values {#status-values}

A status word has no entry of its own here yet. Colour is a hint, not a meaning: green is a healthy or completed state, amber is waiting or in progress, red is a failure, grey is anything the console has no opinion about.

Note that **running** is green and **pending** is amber: work in progress is normal, work that has not been picked up yet is worth a look.

## finished {#finished}

Done, successfully.

On an **action**, the device reported that it installed the update. On a **rollout** or a **deploy group**, every device it was responsible for reached a final state. Meeting a success threshold only starts the next group; it does not finish anything.

## error {#error}

Something failed.

On an **action**, the device reported failure. On a **target**, the last update attempt failed and the device is waiting for someone to decide what happens next. On a **deploy group**, the share of failed devices reached the group's error threshold, which also pauses the rollout that owns it.

A rollout itself is never in error: hitting an error threshold **pauses** it. See [rollout lifecycle](/help/deployment/rollout-lifecycle#paused).

## warning {#warning}

Reserved. The server never writes this status; if you ever see it, it came from data written by something else.

## running {#running}

Work is under way.

On an **action**, the device has picked the job up and is working on it. On a **rollout**, groups are being started and advanced. On a **deploy group**, its actions are live and devices can see them.

## canceled {#canceled}

The action was cancelled and the device acknowledged it — or the server closed it immediately, if automatic closing of superseded actions is switched on.

Spelled `canceled` on the wire, `cancelled` in some places in the interface. Same thing.

## canceling {#canceling}

A cancellation has been requested and the server is waiting for the device to acknowledge it. The device still has the job; it has been told to drop it.

A device may refuse — see [cancel rejected](#cancel-rejected).

## cancel_rejected {#cancel-rejected}

The device was asked to cancel and **refused**. The action is still running and the device is still installing.

This is a real state, not a glitch: a device that has passed the point of no return is allowed to say so. If the action has to stop regardless, force-quit closes it on the server — but the device is told nothing and will carry on. See [sharp edges](/help/reference/sharp-edges#force-quit).

## retrieved {#retrieved}

The device fetched the job description exactly once. It knows what it is supposed to install; it has not said anything about doing it yet.

## download {#download}

The device reported that it started downloading the artifacts.

## downloaded {#downloaded}

The device has all the artifacts.

For an ordinary update this is a step on the way. For a **download-only** action this is the end: the action closes here and the device is never told to install. See [action types](/help/deployment/action-types#downloadonly).

## scheduled {#scheduled}

Created, but not live.

A rollout creates every action up front, including for groups that have not started. An action in this state is invisible to the device: it is not served, and the device is not told about it. It becomes live when its deploy group is activated.

## wait_for_confirmation {#wait-for-confirmation}

The action exists and is assigned, but the device must confirm before it may install anything.

Only happens when the confirmation flow is switched on for the tenant. The device confirms through its own channel, or someone activates auto-confirmation for that device. A **denied** confirmation leaves the action here indefinitely — see [confirmation](/help/deployment/confirmation#denial).

## pending {#pending}

Waiting.

On a **target**, an update has been assigned and the device has not finished it — either it has not polled yet, or it is working. On an **action** list, `pending` is the coarse status meaning "still active", the opposite of `finished`.

## in_sync {#in-sync}

The device is running exactly what it was assigned. Nothing is outstanding.

## registered {#registered}

The device introduced itself and the server created a record for it, but nothing has ever been assigned to it. A fleet that is all `registered` is a fleet that has been plugged in and not yet given anything to do.

## unknown {#unknown}

The device exists as a record and has never reported anything. Usually a target created through the console or the API that has not polled yet.

## responding {#responding}

The device is calling in on schedule. Nothing to do.

## overdue {#overdue}

The device has not polled within the window the tenant expects. It is not necessarily broken — it may be off, offline, or on a slower schedule than the tenant default. See [poll state](/help/fields/targets#poll-state).

## creating {#creating}

Being built.

A **rollout** in this state is still counting the devices its filter matches and dividing them into groups. If it stays here, the node has no rollout strategy extension and cannot divide anything — see [rollout strategies](/help/deployment/rollout-strategies#no-strategy).

## ready {#ready}

Built and waiting for a human.

A **rollout** in this state has its groups; nothing has been sent to any device. It starts when someone starts it.

## starting {#starting}

The rollout was started and the server is creating the jobs and activating the first group. It moves on by itself.

## paused {#paused}

Stopped in place, and resumable.

Either someone paused it, or a deploy group reached its error threshold and paused the rollout to stop the damage spreading to the next group. Devices already working keep working; no new group is activated. **Resume** picks it up where it stopped.

## stopping {#stopping}

A stop was requested and the server is cancelling the outstanding jobs.

## stopped {#stopped}

Terminal. Every outstanding job was cancelled. A stopped rollout cannot be resumed, but it can be retried, which creates a new rollout aimed at the devices that failed.

## waiting_for_approval {#waiting-for-approval}

The rollout is built and needs a second person to approve it before it can start. Only happens when rollout approval is switched on for the tenant.

## approval_denied {#approval-denied}

Someone declined the rollout. Terminal — it can be deleted, not started.

## deleting {#deleting}

The rollout is being removed and the server is closing the jobs it created.

## deleted {#deleted}

Removed. Kept as a record because its actions still refer to it, and hidden from the default lists.

## active {#active}

Switched on and in effect. Used for webhooks, retention rules and auto-confirmation.

## inactive {#inactive}

Switched off. The object still exists and keeps its settings; it just does nothing.

## started {#started}

A console task began and has not reported its outcome yet. A row stuck here is a task that never finished — see [the action log](/help/admin/action-log#the-action-log).

## ok {#ok}

Succeeded. Used for console tasks and webhook deliveries.

## success {#success}

Succeeded — the word the device uses when it reports a finished installation.

## failed {#failed}

Did not succeed. Used for console tasks, webhook deliveries and retention runs.

## update {#update}

The kind of an action: install a distribution set on a device. The other kind is [cancel](#cancel).

## cancel {#cancel}

The kind of an action: stop an update that is already running on a device.

## forced {#forced}

An [action type](/help/deployment/action-types#forced): install now, no discretion for the device.

## soft {#soft}

An [action type](/help/deployment/action-types#soft): the device installs when convenient. It has no deadline, so a device may never install at all.

## time forced {#time-forced}

An [action type](/help/deployment/action-types#timeforced): soft until a moment you named, forced after it.

## download only {#download-only}

An [action type](/help/deployment/action-types#downloadonly): download the files and stop. The action closes at [downloaded](#downloaded) and nothing is installed.

## cancelled {#cancelled}

The same state as [canceled](#canceled). The console spells it with two Ls in places where it is prose and with one where it echoes the wire.
