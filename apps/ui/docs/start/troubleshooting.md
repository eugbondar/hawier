# When something is not happening

> The five questions worth asking before anything else.

## The device is not picking up its action {#not-picking-up}

An action sits at [pending](/help/glossary#pending) or never reaches [retrieved](/help/glossary#retrieved). In order of likelihood:

1. **It has not polled yet.** Nothing is pushed to devices. Check **last request** on the device page against the tenant's [polling interval](/help/admin/settings#polling-time). If the device is [overdue](/help/glossary#overdue), it is not talking to this server at all.
2. **Its group has not started.** An action created by a rollout is [scheduled](/help/glossary#scheduled) until its group is activated, and a scheduled action is invisible to the device on purpose.
3. **It is waiting for confirmation.** With the confirmation flow on, an action sits at [wait_for_confirmation](/help/glossary#wait-for-confirmation) until the device or an operator confirms it.
4. **A maintenance window is closed.** The device may download outside the window but is told not to install. See [maintenance windows](/help/deployment/maintenance-windows#maintenance-windows).
5. **Another action is in front of it.** Unless multiple assignments are enabled, a device works one action at a time and the new one supersedes the old — which needs the old one to be cancelled first, and cancelling needs the device to acknowledge.

## The rollout is stuck in creating {#stuck-in-creating}

A rollout in [creating](/help/glossary#creating) is being divided into groups. If it stays there, this node has no rollout strategy available and cannot divide anything. That is deliberate — the server refuses rather than inventing a division. See [rollout strategies](/help/deployment/rollout-strategies#no-strategy).

## The distribution set will not attach or assign {#set-will-not-assign}

- **It is not complete.** Its type demands a module of some kind that it does not have. Open the set; **complete** is on its page, and its type lists which module types are mandatory.
- **It is invalidated.** An invalidated set cannot start anything new. See [invalidation](/help/repository/locking#invalidation).
- **It is locked.** Locked sets can still be assigned; they cannot be **changed**. See [locking](/help/repository/locking#implicit-lock).

## Downloads answer 503 {#downloads-503}

Artifact storage is provided by an extension. If no storage extension is available on the node, the server answers 503 rather than serving something it cannot verify. Nothing you can change in the console fixes this — it is a deployment question. See [extensions](/help/admin/extensions#no-extension-no-fallback).

## A list turned into a red message {#red-callout}

The query in the filter panel asked for a field the server does not know, or asked for it in a way that field does not support. The message names the field and the position. The page frame stays put — clear the query, or fix it against [the query reference](/help/reference/rsql#the-query-language).

## What the error itself said {#error-categories}

Every failure falls into one of a dozen categories, and the category narrows the cause considerably — see [errors and limits](/help/reference/errors#categories).

## Something failed and left a number {#incident-numbers}

Errors carry a decimal number. Paste it into [Incidents](/incidents) and you get the request that failed, when, on which node, and what it said. See [incidents](/help/admin/incidents#incident-numbers).
