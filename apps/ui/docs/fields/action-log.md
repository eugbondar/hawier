# Action log fields

## resource {#resource}

Which kind of object the task acted on.

## resourceId {#resource-id}

Which object. Paste it into that resource's list to find the row.

## action {#action}

What was attempted.

## status {#status}

Whether it succeeded. Recorded in two phases, so a task that started and never finished is visible as such rather than missing.

## error {#error}

What went wrong, when something did. A failed row usually carries an [incident number](/help/admin/incidents#incident-numbers), which leads to the request itself.

## requestSummary {#request-summary}

A short form of what was sent.

## responseSummary {#response-summary}

A short form of what came back.

## finishedAt {#finished-at}

When the task completed. Empty on a task that started and never finished.
