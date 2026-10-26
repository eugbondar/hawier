# Actions

> One job for one device. Created for you; watched by you; cancellable by you.

## Actions {#actions}

An action is a single instruction to a single device: install this distribution set. It is created by an assignment, by a rollout, or by an auto-assignment rule — never written by hand.

It lives from creation until the device reports that it finished or failed, or until it is cancelled.

## State {#state}

An action has two levels of state, and they answer different questions.

Status :: The coarse one: [pending](/help/glossary#pending) while it is still active, [finished](/help/glossary#finished) once it is closed. Whether it succeeded is not in this word.
Detail status :: The precise one — [scheduled](/help/glossary#scheduled), [retrieved](/help/glossary#retrieved), [download](/help/glossary#download), [downloaded](/help/glossary#downloaded), [running](/help/glossary#running), [wait_for_confirmation](/help/glossary#wait-for-confirmation), [canceling](/help/glossary#canceling), [cancel_rejected](/help/glossary#cancel-rejected), [canceled](/help/glossary#canceled), [finished](/help/glossary#finished) or [error](/help/glossary#error).

When a list column and a card disagree, check which of the two you are reading. A **closed and failed** action is `finished` by status and `error` by detail status.

## Type {#type}

[update](/help/glossary#update) — install something — or [cancel](/help/glossary#cancel), which is the job of stopping one.

## Status history {#status-history}

Every report the device has sent about this action, in order, with the exact time and whatever the device had to say. When an update fails, this is where the reason is: the messages come from the device, not from this server.

## Repeated reports {#repeated-reports}

A device that reports the same thing twice does not produce two rows. The row shows a count and the time it was last repeated instead.

That is not cosmetic: an action may hold only a bounded number of status rows, and a device stuck in a loop would otherwise exhaust it and start failing its own reports.

## Cancelling {#cancelling}

**Cancel** asks the device to stop. The action goes to [canceling](/help/glossary#canceling) and waits for the device to acknowledge, which it may [refuse](/help/glossary#cancel-rejected).

**Force quit** closes the action on the server without waiting. The device is told nothing: if it already has the files, it may still install them. Use it to clear a stuck record, not to stop a device — see [sharp edges](/help/reference/sharp-edges#force-quit).

## Active {#active}

Actions still in flight. What the fleet is doing right now.

## All {#all}

Every action ever created, newest first.

## Failed {#failed}

Actions the device reported as failed. Open one and read its status history.

## Finished {#finished}

Actions that completed successfully.

## From rollouts {#from-rollouts}

Actions created by a rollout rather than by a direct assignment.
