# Retention rules

> Scheduled, automatic deletion — of artifacts, on a schedule you set, with a dry run.

## Retention rules {#retention-rules}

A rule is a schedule, a filter narrowing what it may consider, and a minimum age. When it fires, everything matching is a candidate, and the rule decides what actually goes.

The only scope implemented is **artifacts**. A rule cannot delete devices, actions or distribution sets.

## Dry run {#dry-run}

With dry run on, the rule runs on schedule and records what it **would** have deleted, without deleting anything.

Leave every new rule in dry run for a full cycle and read the result before arming it. Deletion is not reversible.

## The schedule {#cron}

A repeating expression saying when the rule fires. It is validated when you save, so an expression that would never fire is refused.

## The filter {#filter}

A query narrowing which artifacts the rule may consider — by filename, size, age, or the module they belong to. It is checked when you save.

An empty filter means every artifact is a candidate, subject only to the minimum age. That is rarely what you want.

## Minimum age {#min-age}

How old an artifact must be before the rule may touch it, in milliseconds. A floor under everything else: however wrong the filter is, nothing recent goes.

## Active {#active}

Whether the rule runs. An inactive rule keeps its settings and fires never.

## Nothing happens {#no-extension}

Deciding what to keep is done by an [extension](/help/admin/extensions#extensions). Without one, rules fire and delete nothing. The last result on each rule says what happened.

## All {#all}

Every retention rule.

## Armed {#armed}

Rules that are active and **not** in dry run — the ones that actually delete. The list worth reviewing.

## Dry run rules {#dry-run-rules}

Rules that are only reporting.

## Paused {#paused}

Rules that are switched off.
