# Spec Delta

## Purpose

hawier's database contract. This stage of development keeps a schema that is
byte-compatible with hawkBit 1.0.0, with Flyway-compatible history and a
separate hawier-owned track. It also covers startup behavior towards the
database.

## ADDED Requirements

### Requirement: hawkBit schema applied verbatim
The server SHALL apply hawkBit's PostgreSQL migrations unchanged and in order: baseline `B1_20_0`, then `V1_20_1`, then `V1_20_2`.

For each migration it SHALL write a `schema_version` row exactly as Flyway would, in the same transaction as the migration:
- installed rank, version, description, type and script
- checksum: Flyway's CRC32 over the lines, excluding terminators, with any BOM stripped
- installer
- success flag

A stock hawkBit 1.0.0 SHALL be able to start on a database migrated by hawier without `flyway repair`.

#### Scenario: Fresh database
- **WHEN** the server starts on an empty database
- **THEN** `schema_version` holds three successful rows whose checksums equal Flyway's

#### Scenario: Idempotent
- **WHEN** the server starts again on the migrated database
- **THEN** no migration is reapplied

### Requirement: Failed history blocks startup
A `schema_version` row with `success = false` SHALL stop startup with a message to repair the database first. Migrations SHALL be serialized across concurrently starting nodes.

#### Scenario: Two nodes start together
- **WHEN** two nodes start against the same empty database
- **THEN** each migration is applied exactly once

### Requirement: hawier-owned track
hawier's own tables SHALL carry the prefix `hawier_`. Their migrations SHALL be recorded in `hawier_schema_version`, separate from `schema_version`. During development the track SHALL be a single file, `0001_hawier.sql`, edited in place. hawier SHALL NOT alter `sp_*` tables. The hawier tables SHALL be:
- `hawier_saved_view`
- `hawier_action_log`
- `hawier_incident`
- `hawier_search_query`
- `hawier_session`
- `hawier_event_outbox`
- `hawier_webhook`
- `hawier_webhook_delivery`
- `hawier_retention_rule`
- `hawier_rollout_ext`
- `hawier_role`
- `hawier_grant`

#### Scenario: Separate history
- **WHEN** the server has migrated a fresh database
- **THEN** `hawier_schema_version` records `0001_hawier` and `schema_version` contains no hawier row

### Requirement: Waiting for the database
At startup the server SHALL retry the database connection for up to `HAWIER_DB_WAIT_SECS` seconds (default 60) before failing. The failure message SHALL name that variable.

#### Scenario: Postgres comes up late
- **WHEN** PostgreSQL becomes reachable 10 seconds after the node starts
- **THEN** the node waits, connects and starts normally

### Requirement: Shared state only in the database
Nodes SHALL keep no durable state of their own. Targets, actions, rollouts, configuration and artifact metadata SHALL live in the shared PostgreSQL database. Artifact bytes SHALL live in the storage the storage extension locates.

#### Scenario: Restarted node
- **WHEN** a node is restarted
- **THEN** it serves the same data as before without any local recovery
