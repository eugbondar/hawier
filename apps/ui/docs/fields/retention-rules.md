# Retention rule fields

## scope {#scope}

What the rule may delete. Only **artifact** is implemented — a rule cannot delete devices, actions or distribution sets.

## cron {#cron}

When the rule fires, as a repeating schedule expression. Validated when saved.

## filter {#filter}

The [query](/help/reference/rsql#the-query-language) narrowing what the rule may consider — filename, size, age, or the module a file belongs to. Empty means every artifact is a candidate.

## minAgeMs {#min-age-ms}

How old something must be before the rule may touch it, in milliseconds. The floor under everything else.

## dryRun {#dry-run}

On, the rule reports what it **would** delete and deletes nothing. Every new rule should spend a cycle here first — deletion is not reversible.

## active {#active}

Whether the rule runs at all. Off keeps its settings and fires never.

## lastRunAt {#last-run-at}

When it last fired.

## lastResult {#last-result}

What happened on that run — including, for a dry run, what it would have removed.
