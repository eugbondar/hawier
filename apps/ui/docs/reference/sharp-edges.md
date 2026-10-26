# Sharp edges

> Behaviour that is correct, deliberate, and surprising. Read this before it surprises you.

## A device may refuse to be cancelled {#cancel-rejected}

Cancelling asks the device to stop. It can answer no, and then it carries on and finishes.

There is no way to stop a device from here. [Force quit](#force-quit) closes the record; it does not reach the device.

## Force quit does not reach the device {#force-quit}

It closes the action on the server without waiting for anything. The device is told nothing at all — no cancellation, no notice — so if it already has the files it may install them anyway, and then report against an action that no longer exists.

Use it to clear a stuck record when you know the device is gone. Do not use it to stop an installation.

## A denied confirmation waits forever {#denied-confirmation}

Denying a confirmation is not a decision the system acts on. The action stays at [wait_for_confirmation](/help/glossary#wait-for-confirmation) indefinitely — it does not fail, does not expire, and does not go away.

Cancel it if you want it gone.

## An offline installation is taken on trust {#offline-declaration}

A device can state what it has installed without any action having existed. The server records it and marks the device in sync with whatever set it named. Nothing verifies it.

A device that lies, or whose agent has a bug, is believed. If your fleet reports in sync with something you never sent, this is why.

## Soft has no deadline {#soft-has-no-deadline}

A [soft](/help/deployment/action-types#soft) action lets the device install at a convenient moment, and nothing ever decides that too long has passed. A device that never finds a convenient moment never installs, and the action stays active indefinitely.

If it has to land eventually, use [time-forced](/help/deployment/action-types#timeforced).

## Download only stops at downloaded {#download-only-stops}

A [download-only](/help/deployment/action-types#downloadonly) action closes when the files arrive. The device is never told to install, and [downloaded](/help/glossary#downloaded) is the end state rather than a step.

Choosing it for a rollout means the rollout will complete having installed nothing.

## Per-group filters are ignored {#per-group-filters}

A rollout group can be given a query of its own through the API. It is accepted, stored and shown — and the built-in division ignores it entirely. Which devices land in which group is decided by the [strategy](/help/deployment/rollout-strategies#strategies), not by per-group queries.

## Group success and error actions are fixed {#group-actions}

Groups display a "success action" and an "error action". They are constants: on success the rollout always advances, on error it always pauses. They cannot be changed, and they are shown only because the interface reports what the server returns.

## Attribute comparisons are textual {#numeric-attributes}

`attribute.*` and `metadata.*` values are always compared as text, whatever they look like.

`attribute.bucket=le=50000` keeps `9` and drops `10000`, because as text `'10000'` sorts before `'9'`. There is no warning and no error — you get a wrong answer that looks like a right one.

Pad numeric attributes to a fixed width at the device if you need to compare them, or compare them exactly.

## A rollout is never in error {#no-error-rollout}

Reaching an error threshold [pauses](/help/glossary#paused) a rollout. There is no error state for a rollout, so filtering rollouts by error finds nothing — look for paused ones, and for groups in [error](/help/glossary#error).

## Autoclose skips the handshake {#autoclose}

With [automatic closing](/help/admin/settings#repository-actions-autoclose-enabled) on, a superseded action is closed immediately as [canceled](/help/glossary#canceled) without asking the device. The record is tidy; the device was never told, and may still install what it had already downloaded.

## A tenant with no roles allows everything {#no-roles}

Authorisation is opt-in. Until the first [role](/help/admin/roles#unconfigured) exists, every signed-in user can do everything, and nothing in the interface says so.

## Missing extensions refuse rather than improvise {#missing-extensions}

No storage extension means artifacts answer 503. No rollout strategy means rollouts sit in [creating](/help/glossary#creating) forever. No retention extension means rules delete nothing. No delivery extension means events pile up unsent.

All four look like breakage and are deliberate: see [extensions](/help/admin/extensions#no-extension-no-fallback).
