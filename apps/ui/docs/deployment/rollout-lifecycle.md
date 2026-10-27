# Rollout lifecycle

> Which state a rollout is in, which button it will accept, and why the others are refused.

## The path {#the-path}

1. **Creating.** The rollout is counting the devices its query matches and dividing them into groups. Nothing has been sent.
2. **Ready** — or **waiting for approval**, if the tenant requires a second pair of eyes.
3. **Starting.** Jobs are created for every group. The first group is activated; the rest are [scheduled](/help/glossary#scheduled) and invisible to their devices.
4. **Running.** Groups advance one after another: a group that meets its success threshold activates the next one, and keeps running itself until its last device reports.
5. **Finished**, or **paused**, or **stopped**.

## Start {#start}

Accepted only from [ready](/help/glossary#ready). Creates every job the rollout will ever create, activates the first group, and moves the rollout to running.

Jobs for later groups exist from this moment but are invisible to their devices until their group is activated.

## Pause {#pause}

Accepted only while running. Devices already working carry on; no further group is activated.

Pausing does not recall anything. A device that has the files may still install them.

## Resume {#resume}

Accepted only from [paused](/help/glossary#paused). Picks the rollout up where it stopped.

If it was paused by an error threshold, resuming carries on into the next group **with the failures still counted**. Resume when you understand why they failed, not to see whether it happens again.

## Stop {#stop}

Accepted while running or paused. Cancels every outstanding job and ends the rollout. Terminal — a stopped rollout cannot be resumed, only [retried](#retry).

## Trigger next group {#trigger-next-group}

Accepted while running. Activates the next scheduled group immediately, without waiting for the current one to meet its success threshold.

This is the override. It is how you get a slow wave moving when you have decided the outstanding devices are simply offline — and it is also how you push a bad update into the next wave, so be sure.

## Approve {#approve}

Accepted only from [waiting_for_approval](/help/glossary#waiting-for-approval), and only when the tenant requires approval. Records who approved it and any remark, and moves the rollout to ready.

## Deny {#deny}

The other answer. Terminal: a denied rollout can be deleted, not started. The remark is kept as the reason.

## Retry {#retry}

Accepted from finished, stopped or paused. Creates a **new** rollout aimed at exactly the devices that failed, as a single group. The original is untouched.

## Paused {#paused}

A rollout pauses for one of two reasons: somebody pressed pause, or a group's failures reached its error threshold. There is no separate "error" state for a rollout — an error threshold pauses it, and the group that tripped it is the one showing [error](/help/glossary#error).

## Why a button was refused {#conflicts}

Every transition above is guarded. Asking for one the rollout cannot make answers with a conflict naming the state it is in and the state the operation needs. The console only offers the buttons the current state accepts, so this normally appears when two people are working on the same rollout at once.
