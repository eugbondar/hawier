# Action log

> What operators did in this console.

## The action log {#the-action-log}

One row per console task: who did it, what they did it to, when, and whether it worked. Recorded in two phases, so a task that started and never finished is visible as such rather than missing.

It records what was done through **this console**. Changes made directly through the API are not here — those leave their trace in the API's own records.

## Reading a row {#reading}

Username :: Who was signed in.
Resource and id :: What was acted on.
Action :: What was attempted.
Status :: Whether it succeeded.
Error :: What went wrong, when it did.

A failed row usually carries an [incident number](/help/admin/incidents#incident-numbers), and that leads to the request itself.

## Latest {#latest}

Everything, newest first.

## Failed {#failed}

Tasks that did not succeed.

## Changes {#changes}

Tasks that modified something, filtering out the reads.
