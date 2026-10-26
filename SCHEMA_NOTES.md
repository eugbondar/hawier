# SCHEMA_NOTES — narrow places in the DB schema

> **Status: temporary compatibility (development stage).**
> hawier currently uses a DB schema that is byte-for-byte compatible with hawkBit 1.0.0
> (Flyway baseline `B1_20_0` + `V1_20_1` + `V1_20_2`, history in the table
> `schema_version`). This is a deliberate **temporary** decision, so that during
> development stock hawkBit can work against the same database and so that the UI/clients
> can move over without a data migration.
>
> Everything listed below is an inherited "narrow place" that will be
> fixed in the pre-production revisions of hawier's own schema.
> **Rule for agents:** every newly found narrow place must be marked in
> the code with a `// NARROW:` comment (or `-- NARROW:` in SQL) and added as an item
> to this file. See CLAUDE.md.

| # | Narrow place | Why it is left | Fix plan (pre-production) |
|---|---|---|---|
| 1 | `tenant VARCHAR(40)` is denormalized onto every table, with no FK to `sp_tenant`; join/metadata tables have no tenant at all | compatibility with JPA `@Multitenant(SINGLE_TABLE)` | FK `tenant_id BIGINT` → `sp_tenant`, or schema-per-tenant; add tenant to the join tables |
| 2 | All timestamps are `BIGINT` epoch millis (`created_at`, `last_target_query`, …), while `SP_LOCK` uses `TIMESTAMP` | hawkBit's JPA converters | `timestamptz` + `now()` defaults; BRIN indexes on time |
| 3 | `optlock_revision BIGINT` (JPA optimistic locking) on every table | stock hawkBit expects the column; hawier writes 1/increment | remove; concurrency via `UPDATE ... WHERE` and versions where they are really needed |
| 4 | Nullable BOOLEAN (`active`, `deleted`, `valid`, `is_dynamic`, `confirmation_required`…) — three-valued logic that the code reads as two-valued | hawkBit's DDL is weaker than the contract of its entity | `NOT NULL DEFAULT` everywhere |
| 5 | `sp_action_status_messages` has no PK and no ordering column; in hawkBit it is marked `TODO - messages is not used yet` | JPA `@ElementCollection List<String>` | surrogate PK + `seq INT`, or move the messages into `sp_action_status.detail` JSONB |
| 6 | Enums as "raw" INTEGERs with gaps (RolloutStatus skips 7,8) and without CHECK constraints; in `sp_rollout_group` some columns use a converter, some an ordinal | exact hawkBit code maps (see `libs/api/src/codes.rs`) | PG enum types or lookup tables + CHECK |
| 7 | JPA join tables (`sp_ds_sm`, `sp_ds_tag`, `sp_target_target_tag`, `sp_ds_type_element`, `sp_target_type_ds_type`, `sp_rollout_target_group`) without surrogate keys, audit, and tenant | JPA style | proper tables with an audit block where history matters |
| 8 | Asymmetric `ON DELETE`: `sp_action.target` cascades and destroys the action history, while `sp_action.distribution_set`/`rollout` block deletion | copied from hawkBit | policy: history tables are not cascaded, but soft-deleted/archived |
| 9 | `sp_rollout_group.parent` is a self-reference without an FK (index only): cycles and orphans are possible | hawkBit lives with it | FK + CHECK against cycles (recursive CTE in a trigger or at the application level) |
| 10 | Three nearly identical metadata tables; `sp_sm_metadata` has its PK in reverse order `(meta_key, sm)` and an extra `target_visible` column | 1:1 with hawkBit | a single table `entity_metadata(entity_kind, entity_id, key, value)` or a JSONB column |
| 11 | Redundant/badly named indexes: `sp_idx_target_attributes_target_id` duplicates a prefix of the PK; `uk_target_name` sits on `sp_target_type`; the index `fk_action_status_messages_action_status` is named like a constraint | 1:1 with hawkBit | index review against real query plans |
| 12 | `access_control_context TEXT` in `sp_rollout` and `sp_target_filter_query` — a serialized Java SecurityContext; for hawier it is opaque, we write NULL and ignore it | stock hawkBit needs the column | remove; the initiator's context goes into proper columns |
| 13 | `sp_target.target_group VARCHAR(256)` — a free-text path hierarchy, the third overlapping grouping mechanism (tags, types, groups) | new hawkBit mechanism | a dedicated groups table with a parent FK, or consolidation with tags |
| 14 | `SP_LOCK` — a Spring Integration table in upper case among lowercase `sp_*`; hawier does not use it | needed by stock hawkBit (JdbcLockRegistry) | remove together with byte-compatibility; leader election via advisory locks |
| 15 | Artifact hashes are three hex columns (`VARCHAR(32)/VARCHAR(40)/CHAR(64)`), with no UNIQUE `(tenant, sha1_hash)` — deduplication only at the application level | 1:1 with hawkBit | `bytea` + UNIQUE (tenant, sha256); keep sha1 only for DDI compatibility |
| 16 | `locked BOOLEAN DEFAULT TRUE` on DS/SM with nullable `deleted`/`valid` — asymmetric defaults | 1:1 with hawkBit | `NOT NULL DEFAULT FALSE` + consistent defaults |
| 17 | No partitioning for the unboundedly growing `sp_action_status` / `sp_action_status_messages` | hawkBit cleans them with a job | range partitions by time + retention policy |
| 18 | `hawier_session.secret` stores the user's password (needed to replay HTTP Basic from the UI into mgmt) | inherited from the hawier-ui↔hawkBit pairing | server-side session tokens; do not store the password at all |
| 19 | `hawier_session` is the only table with two keys: a numeric `id` (as everywhere in hawier) and a random `token` that lives in the cookie. The split is forced: behind the token stands the password from #18, so it has to be unguessable, while a snowflake is guessable | deliberate decision (numbers everywhere except sessions) | together with #18: a signed server-side token with a TTL, the separate column goes away |
| 19a | Target groups are enumerated by `GROUP BY target_group` over the whole `sp_target`: the index `sp_idx_target_group (tenant, target_group)` from the baseline covers it, but the count is still row by row. Fine for a console list, not for the hot path | consequence of #13 (a group is a free-text column, not a table) | together with #13: a groups table with a parent FK, the count is an ordinary aggregate over the FK |
| 20 | The status repeat counter lives in `sp_action_status.optlock_revision`. The semantics match (how many times the row was changed), but the name says something else, and any future code that decides to "normalize optlock" will silently wipe the counter. See `actions::repeat_of_last_status` | together with #3: a column cannot be added to `sp_*` without breaking byte-compatibility | together with #3: column `repeats INT NOT NULL DEFAULT 0` + `last_reported_at`, `optlock_revision` is removed |
| 21 | `sp_target_attributes.attribute_value VARCHAR(128)` — attribute values are always text, and RSQL compares them as text: `Field::KeyValueSub` binds `Value::Text` even for `=lt=/=le=/=gt=/=ge=` (`apps/server/src/rsql.rs`). A numeric attribute is compared lexicographically, and **there is no error — the result is silently wrong**: `attribute.bucket=le=50000` will let `'9'` through and drop `'10000'` | 1:1 with hawkBit (`@ElementCollection Map<String,String>`) | typed values (`value_num`/`value_text` or JSONB with a type) and numeric predicates in RSQL; until then keep numeric dimensions in a `hawier_*` sidecar — see EXTENSIONS.md §4.7 |
| 22 | `sp_rollout_group`: **partially fixed.** `success_condition_exp`/`error_condition_exp` are now written from the strategy's plan, and `target_filter` is passed to it in `GroupDefinition` — the classic strategy still ignores it (deliberately, that is its semantics; see API_NOTES §7). The columns `success_action`/`error_action` are still written as constants and not read: the transitions belong to the engine, not to the group | the engine implements the transitions itself | remove `*_action` together with the schema revision: their value affects nothing |
| 23 | The device liveness flag (`hawier_target_poll_state`) is computed row by row: `sp_target` has no index on `last_target_query` (the baseline indexes `name`, `controller_id`, `created_at`, `last_modified_at`, `target_group`), and adding an index to `sp_*` is forbidden — so filtering and sorting by `pollState` is a scan of the tenant's targets. On top of that, every row examined costs two reads of `sp_tenant_configuration`: a STABLE function correlated on `t.tenant` is not hoisted out of the scan by the planner. Acceptable for a console list, not for the hot path | the polling window lives in the tenant config, not in a target column, and the predicate must be one for all consumers (UI/UX rule 6): `Field::Column.sql` is a `&'static str`, into which a value from Rust cannot be substituted | together with the schema revision: an index `sp_target (tenant, last_target_query)` and a denormalized column `overdue_at`, updated on polling. Until then — a sidecar `hawier_target_liveness(tenant, target, overdue_at)` written by DDI; but first measure `make load` and compare with `bench/load-history.csv`: an extra write on every poll is the hot path |

## How compatibility is verified

- `apps/server/src/db/flyway.rs` — a replica of the Flyway algorithm (CRC32 checksum
  line by line, parsing of migration names); hawier writes the same rows into
  `schema_version` that Flyway would have written.
- Canary: stock `hawkbit/hawkbit-update-server` must start on a
  database migrated by hawier, without `flyway repair` (a manual step — see the
  README; an automated test compares the `schema_version` rows).
