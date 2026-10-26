-- hawier's own tables and functions (hawier_* prefix — never collides with
-- hawkBit's sp_*).
--
-- One file on purpose: we are still in development, so there is no earlier
-- hawier to stay compatible with. Breaking changes are made by editing this
-- file and recreating the database, not by piling ALTERs on top. (The
-- hawkBit track is the opposite — it must stay byte-exact for Flyway.)
--
-- Every id here is the decimal snowflake from apps/server/src/incident.rs:
-- 41 bits ms since 2026-01-01 | 10 bits node | 12 bits sequence. Short
-- enough to double-click, sortable by time, unique across nodes.

-- Saved list views of the console (per tenant + user + resource).
CREATE TABLE hawier_saved_view (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    username VARCHAR(64) NOT NULL,
    resource VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    q TEXT NOT NULL DEFAULT '',
    columns TEXT NOT NULL DEFAULT '',
    sort VARCHAR(128) NOT NULL DEFAULT '',
    page_size INTEGER NOT NULL DEFAULT 25,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX hawier_idx_saved_view_scope
    ON hawier_saved_view (tenant, username, resource, position);

-- What an operator did in the console: one two-phase row per action.
CREATE TABLE hawier_action_log (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    username VARCHAR(64) NOT NULL,
    resource VARCHAR(64) NOT NULL,
    resource_id VARCHAR(256) NOT NULL DEFAULT '',
    action VARCHAR(64) NOT NULL,
    status VARCHAR(16) NOT NULL,
    request_summary TEXT NOT NULL DEFAULT '',
    response_summary TEXT NOT NULL DEFAULT '',
    error TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

CREATE INDEX hawier_idx_action_log_scope
    ON hawier_action_log (tenant, created_at DESC);

-- One row per failing request, keyed by the number the caller was given.
CREATE TABLE hawier_incident (
    id BIGINT PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant VARCHAR(40),
    username VARCHAR(64),
    method VARCHAR(8) NOT NULL,
    path VARCHAR(512) NOT NULL,
    status INTEGER NOT NULL,
    error_code VARCHAR(128) NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    -- Where the failing request came from, so the dashboard can separate
    -- devices and API clients from our own console noise:
    --   ddi | mgmt | console | ui
    kind VARCHAR(16) NOT NULL DEFAULT 'mgmt'
);

CREATE INDEX hawier_idx_incident_time ON hawier_incident (occurred_at DESC);
CREATE INDEX hawier_idx_incident_kind ON hawier_incident (kind, id DESC);

-- Recent global-search queries, per tenant + user, offered back in the
-- search box. One row per distinct query: repeating it bumps the row.
CREATE TABLE hawier_search_query (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    username VARCHAR(64) NOT NULL,
    q TEXT NOT NULL,
    uses INTEGER NOT NULL DEFAULT 1,
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX hawier_idx_search_query_unique
    ON hawier_search_query (tenant, username, q);
CREATE INDEX hawier_idx_search_query_recent
    ON hawier_search_query (tenant, username, last_used_at DESC);

-- Console sessions. The id is an ordinary number like everywhere else, but
-- the cookie carries `token`: whoever holds it can read `secret` below, so
-- it must be unguessable — a snowflake would not be.
--
-- NARROW: `secret` is the user's mgmt password, kept so the console can
-- replay HTTP Basic upstream (inherited hawkBit-era design). Pre-production
-- hawier replaces this with server-issued tokens; see SCHEMA_NOTES.md #18.
CREATE TABLE hawier_session (
    id BIGINT PRIMARY KEY,
    token VARCHAR(64) NOT NULL,
    username VARCHAR(64) NOT NULL,
    tenant VARCHAR(40) NOT NULL,
    secret TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX hawier_idx_session_token ON hawier_session (token);
CREATE INDEX hawier_idx_session_expiry ON hawier_session (expires_at);

-- Domain events waiting to be delivered outwards (transactional outbox).
--
-- A row is written in the SAME transaction as the change it describes, so
-- an event cannot exist for a rollout that rolled back, and a committed
-- change cannot silently fail to notify. Delivery is a separate concern:
-- the control node drains this table, claiming rows with FOR UPDATE SKIP
-- LOCKED exactly like the rollout engine claims rollouts.
--
-- Sub nodes write here too (they serve DDI) and never drain: while the
-- control node is down, events simply queue.
CREATE TABLE hawier_event_outbox (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    kind VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL,
    -- pending → delivered, or dead once the attempts are exhausted.
    state VARCHAR(16) NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The drainer's only query: due pending rows, oldest first.
CREATE INDEX hawier_idx_event_outbox_due
    ON hawier_event_outbox (next_attempt_at, id) WHERE state = 'pending';

-- Who wants to be told. An ordinary console resource (UI/UX rule 11).
CREATE TABLE hawier_webhook (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    name VARCHAR(128) NOT NULL,
    url TEXT NOT NULL,
    -- Shared secret for the HMAC signature header. Never rendered back.
    secret TEXT NOT NULL DEFAULT '',
    -- Comma-separated event kinds, or '*' for all.
    event_mask TEXT NOT NULL DEFAULT '*',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX hawier_idx_webhook_tenant ON hawier_webhook (tenant, active);

-- What actually happened on the wire, so a silent integration is visible.
CREATE TABLE hawier_webhook_delivery (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    webhook BIGINT NOT NULL REFERENCES hawier_webhook (id) ON DELETE CASCADE,
    event BIGINT NOT NULL,
    kind VARCHAR(64) NOT NULL,
    -- HTTP status, or 0 when the request never got an answer.
    status INTEGER NOT NULL DEFAULT 0,
    error TEXT NOT NULL DEFAULT '',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    delivered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX hawier_idx_webhook_delivery_recent
    ON hawier_webhook_delivery (tenant, delivered_at DESC);

-- Automatic deletion rules, evaluated on a schedule.
--
-- The rule narrows candidates (RSQL + a minimum age); the retention
-- extension decides which of them may actually go. Neither the SQL nor the
-- extension deletes anything: the host does, and only ids the extension was
-- offered in the first place.
--
-- `dry_run` defaults to TRUE on purpose. A rule that deletes artifacts is
-- the kind of automation that should be watched saying what it would do
-- before it is allowed to do it.
CREATE TABLE hawier_retention_rule (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    name VARCHAR(128) NOT NULL,
    -- `artifact` today; a column so another scope needs no new table.
    scope VARCHAR(32) NOT NULL DEFAULT 'artifact',
    -- Narrows what the extension is asked about at all.
    filter_rsql TEXT NOT NULL DEFAULT '',
    -- Nothing younger than this is even a candidate.
    min_age_ms BIGINT NOT NULL DEFAULT 0,
    -- Quartz-style cron, the same parser maintenance windows use.
    cron VARCHAR(128) NOT NULL,
    -- Rule-specific knobs, passed to the extension verbatim.
    params JSONB NOT NULL DEFAULT '{}'::jsonb,
    dry_run BOOLEAN NOT NULL DEFAULT TRUE,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    last_run_at TIMESTAMPTZ,
    -- What the last run did, in words, for the operator.
    last_result TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX hawier_idx_retention_due ON hawier_retention_rule (tenant, active);

-- Which strategy divides a rollout, and how it is configured.
--
-- A sidecar rather than columns on `sp_rollout`: that table is hawkBit's and
-- must stay byte-compatible. A rollout with no row here uses whichever
-- strategy the node was started with, which keeps every existing rollout
-- working unchanged.
CREATE TABLE hawier_rollout_ext (
    rollout BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    strategy VARCHAR(64) NOT NULL,
    params JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Roles and who holds them.
--
-- hawkBit has no user table (users come from configuration), so this grants
-- by username rather than by a foreign key. Enforcement is opt-in per
-- tenant: with no roles defined, everyone keeps the full permission set, so
-- switching hawier on does not lock an existing deployment out of itself.
CREATE TABLE hawier_role (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    name VARCHAR(64) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- Comma-separated permission names; `ROLE_ALL` means everything.
    permissions TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX hawier_idx_role_name ON hawier_role (tenant, name);

CREATE TABLE hawier_grant (
    id BIGINT PRIMARY KEY,
    tenant VARCHAR(40) NOT NULL,
    username VARCHAR(64) NOT NULL,
    role BIGINT NOT NULL REFERENCES hawier_role (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX hawier_idx_grant_unique ON hawier_grant (tenant, username, role);
CREATE INDEX hawier_idx_grant_user ON hawier_grant (tenant, username);

-- ---------------------------------------------------------------------------
-- Device liveness: one definition, in SQL, because three consumers ask.
-- ---------------------------------------------------------------------------
--
-- The console asks the same question three ways: which rows the Targets list
-- keeps (`q=pollState==...`), what the `pollState` column says in each row,
-- and what the target card shows in `pollStatus`. A predicate written once in
-- SQL and once in Rust is a predicate that will disagree with itself on some
-- row -- different clocks (the node's SystemTime against the database's
-- now()), different config snapshots (a cached tenant value against a live
-- read), different parsers. The symptom would be a green badge inside the
-- "Not Responding" list, and nothing could catch it. That is exactly the
-- failure UI/UX rule 6 exists to prevent, so the predicate lives here, once,
-- and Rust only calls it.
--
-- Reading sp_tenant_configuration is a read: the ban is on altering hawkBit's
-- tables, and nothing here does.
--
-- Dropped by bare name before each CREATE so the file stays re-appliable
-- after `make reset-ui-state`: CREATE OR REPLACE refuses to change a return
-- type, and a DROP carrying an argument list would leave an old overload
-- behind under a changed signature.

DROP FUNCTION IF EXISTS hawier_target_poll_state;
DROP FUNCTION IF EXISTS hawier_poll_window_millis;
DROP FUNCTION IF EXISTS hawier_polling_millis;
DROP FUNCTION IF EXISTS hawier_tenant_millis;
DROP FUNCTION IF EXISTS hawier_hhmmss_millis;

-- `HH:MM:SS` -> milliseconds; NULL when the value is not that shape.
--
-- NULL rather than an exception, and a regex rather than `::INTERVAL`:
-- sp_tenant_configuration is free text, and PUT /rest/v1/system/configs
-- checks only that the value is a scalar, not that it is a duration. So
-- `pollingTime = '5 minutes'` is a value an operator can really store, and a
-- cast would turn every Targets list into a 500 until somebody found it. A
-- typo in a settings field must not take the fleet view down; the caller
-- falls back to the documented default instead.
CREATE FUNCTION hawier_hhmmss_millis(p_value TEXT) RETURNS BIGINT
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $hawier$
    SELECT CASE WHEN btrim(p_value) ~ '^[0-9]{1,4}:[0-9]{1,2}:[0-9]{1,2}$' THEN
        ((split_part(btrim(p_value), ':', 1)::BIGINT * 60
        + split_part(btrim(p_value), ':', 2)::BIGINT) * 60
        + split_part(btrim(p_value), ':', 3)::BIGINT) * 1000
    END;
$hawier$;

-- One tenant-configuration duration in millis, with its default.
--
-- p_default is the same `HH:MM:SS` literal tenant::default_value returns, not
-- a precomputed number: the settings page shows that string, and this
-- predicate must not be able to disagree with the page about what "unset"
-- means.
--
-- ORDER BY/LIMIT rather than a bare scalar subquery: uk_tenant_configuration
-- is case-sensitive on tenant, so a stock hawkBit writing 'default' beside
-- our 'DEFAULT' would make an unguarded subquery raise "more than one row
-- returned" -- a 500 on every target list, from a row we did not write.
CREATE FUNCTION hawier_tenant_millis(p_tenant TEXT, p_key TEXT, p_default TEXT)
RETURNS BIGINT LANGUAGE sql STABLE PARALLEL SAFE AS $hawier$
    SELECT COALESCE(
        (SELECT hawier_hhmmss_millis(c.conf_value)
           FROM sp_tenant_configuration c
          WHERE UPPER(c.tenant) = UPPER(p_tenant) AND c.conf_key = p_key
          ORDER BY c.id LIMIT 1),
        hawier_hhmmss_millis(p_default));
$hawier$;

-- `pollingTime` alone: what the device is told to sleep, and therefore when
-- the next request is expected (hawkBit's nextExpectedRequestAt).
CREATE FUNCTION hawier_polling_millis(p_tenant TEXT) RETURNS BIGINT
LANGUAGE sql STABLE PARALLEL SAFE AS $hawier$
    SELECT hawier_tenant_millis(p_tenant, 'pollingTime', '00:05:00');
$hawier$;

-- How long a device may be silent before it is late: hawkBit's own formula,
-- pollingTime + pollingOverdueTime. Both default to 00:05:00, so an
-- unconfigured tenant calls a device overdue after ten minutes of silence.
-- pollingOverdueTime had been declared, defaulted and served over
-- /rest/v1/system/configs while nothing read it; this is what reads it.
CREATE FUNCTION hawier_poll_window_millis(p_tenant TEXT) RETURNS BIGINT
LANGUAGE sql STABLE PARALLEL SAFE AS $hawier$
    SELECT hawier_polling_millis(p_tenant)
         + hawier_tenant_millis(p_tenant, 'pollingOverdueTime', '00:05:00');
$hawier$;

-- The single definition of the state the console filters, sorts, lists and
-- shows on the card. Total by construction -- no row is NULL -- so
-- `pollState!=responding` is the exact complement of `pollState==responding`
-- and the two views' totals add up to the unfiltered total (UI/UX rule 6).
-- The NULL arm is first so a never-polled row never pays for the lookup.
--
-- NARROW: evaluated per row. sp_target has no index on last_target_query and
-- we may not add one to an sp_* table, so filtering or sorting on this scans
-- the tenant's targets; on top of that every scanned row costs two lookups in
-- sp_tenant_configuration, because a STABLE function correlated on t.tenant
-- cannot be hoisted out of the scan. Acceptable for a console list, wrong for
-- a hot path. See SCHEMA_NOTES.md #23.
CREATE FUNCTION hawier_target_poll_state(p_last_target_query BIGINT, p_tenant TEXT)
RETURNS TEXT LANGUAGE sql STABLE PARALLEL SAFE AS $hawier$
    SELECT CASE
        WHEN p_last_target_query IS NULL THEN 'never'
        WHEN (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT
             > p_last_target_query + hawier_poll_window_millis(p_tenant)
            THEN 'overdue'
        ELSE 'responding'
    END;
$hawier$;
