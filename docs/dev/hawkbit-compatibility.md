# hawkBit compatibility

For the development stage, hawier's database schema is byte-for-byte the
schema of hawkBit 1.0.0, and its Mgmt and DDI APIs follow hawkBit's wire
contracts, so existing devices, tooling and stock hawkBit work with the same
database during a migration. This is a temporary decision: the narrow places
of the inherited schema are collected in
[SCHEMA_NOTES.md](../../SCHEMA_NOTES.md) and will be cut out in the
pre-production revisions of hawier's own schema. Contract: spec
[`database-schema`](../../openspec/specs/database-schema/spec.md); the
reasoning is in [ARCHITECTURE.md](../../ARCHITECTURE.md).

## The hawkBit migration track

hawier applies the original hawkBit migrations — baseline `B1_20_0`, then
`V1_20_1` and `V1_20_2` — verbatim, and writes a Flyway-compatible history
to `schema_version`. This was verified by running stock hawkBit against a
hawier database: Flyway validation passed without `flyway repair`. Running
hawier and hawkBit on one database at the same time is not a supported
scenario and is not needed.

## hawier's own tables

hawier's own data — views, the action log, console sessions and searches,
incidents, the outbox and webhooks, retention rules, roles and grants,
rollout strategy parameters — lives in `hawier_*` tables and never overlaps
with hawkBit's `sp_*` tables, which hawier never alters. The `hawier_*`
track is one migration file, `apps/server/migrations/hawier/0001_hawier.sql`,
edited in place while hawier is in development; after editing it, run
`make reset-ui-state`.
