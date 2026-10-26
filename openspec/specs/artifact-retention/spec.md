# artifact-retention Specification

## Purpose

Operator-defined retention rules that periodically remove artifacts. The
retention extension decides which offered candidates to delete, and the host
guarantees safety: it deletes only what it offered, reports dry runs, and
removes content only after its last reference is gone.

## Requirements

### Requirement: Retention rule resource
`/rest/v1/retentionrules` SHALL offer create, read, update, delete and list. A rule SHALL have:
- `name`
- `scope` (only `artifact`)
- `cron` (the Quartz-style syntax shared with maintenance windows, evaluated in UTC)
- `filter` (RSQL over `filename`, `size`, `createdat`, `module.name`, `module.version`)
- `minAgeMs`
- `params` (JSON passed to the policy)
- `dryRun` (default true)
- `active` (default true)
- `lastRunAt`
- `lastResult`

On write, an unknown scope, an unparsable cron or an unknown filter field SHALL get 400.

#### Scenario: Safe default
- **WHEN** a rule is created through the API without `dryRun`
- **THEN** it is stored with `dryRun: true`

#### Scenario: Bad filter refused
- **WHEN** a rule's filter names an unknown field
- **THEN** the server answers 400 and stores nothing

### Requirement: Rule evaluation
On housekeeping, an active rule SHALL run when its cron has fired since its last run. A run SHALL:
1. Collect up to 5000 candidates of the tenant that are older than `minAgeMs` and match the filter.
2. Flag each candidate for whether it is in a distribution set and whether it was ever assigned.
3. Ask the retention extension which candidates to delete.

The host SHALL delete only ids it offered. A plan naming any other id SHALL fail the run, and nothing SHALL be deleted. Each run SHALL record `lastRunAt` and `lastResult`. A failing rule SHALL NOT prevent other rules from running.

#### Scenario: Keep newest, delete the rest
- **WHEN** an armed rule with the default policy runs over five unassigned dev builds of one module
- **THEN** the two oldest are deleted and three remain

#### Scenario: Filter narrows the offer
- **WHEN** a rule's filter matches only one module
- **THEN** artifacts of other modules are never offered or deleted

### Requirement: Dry run
A dry-run rule SHALL delete nothing. It SHALL record `dry run: would delete N of M candidates (B bytes) — <reason>` as its last result.

#### Scenario: Dry-run report
- **WHEN** a dry-run rule would delete 3 artifacts
- **THEN** nothing is deleted and the last result says `would delete 3`

### Requirement: Deletion is announced and content is reference-counted
Each deleted artifact SHALL emit `artifact.deleted` with `deletedBy: retention:<rule>` and `blobRemoved`. Content SHALL be removed from storage only when no artifact of the tenant references its sha1 any longer.

#### Scenario: Shared content survives
- **WHEN** retention deletes one of two artifacts that share content
- **THEN** the event reports `blobRemoved: false` and the other artifact still downloads
