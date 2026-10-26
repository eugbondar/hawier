# The retention rule form

> Scheduled deletion. Start it in dry run.

## The form {#the-form}

Creates a rule that fires on a schedule and deletes artifacts matching its filter, older than its minimum age.

> Leave **dry run** on until you have seen a full cycle's result. Deletion is not reversible.

## Name {#name}

What the rule is called. Name it after what it removes — `drop-dev-builds-90d`.

## Schedule (cron) {#cron}

When the rule fires, as a repeating expression. Validated when you save, so an expression that would never fire is refused.

Off-peak is the usual choice: a rule that deletes while a fleet is downloading competes with it.

## Filter (RSQL) {#filter}

A [query](/help/reference/rsql#the-query-language) narrowing which artifacts the rule may consider — by filename, size, creation time, or the module they belong to.

An empty filter means **every** artifact is a candidate, bounded only by the minimum age. Almost never what you want.

```
filename==*-dev.bin
module.name==nightly-agent
size=gt=1073741824
```

## Minimum age (ms) {#min-age-ms}

How old an artifact must be before the rule may touch it, in milliseconds.

This is the floor under everything else: however wrong the filter turns out to be, nothing recent goes. Thirty days is 2592000000.

## Dry run {#dry-run}

On, the rule runs on schedule and records what it **would** have deleted, deleting nothing. Off, it deletes.

Every new rule should spend at least one cycle here first.
