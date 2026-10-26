# ARCHITECTURE — hawier

hawier is a rewrite of Eclipse hawkBit in Rust: a device update server
with hawkBit-compatible Mgmt (`/rest/v1`) and DDI (`/{tenant}/controller/v1`)
APIs on top of PostgreSQL. AMQP/DMF is deliberately not supported.

## Monorepo

```
apps/server   crate hawier-server → binary `hawier`     (server: Mgmt + DDI + /ui/v1 + engines)
apps/ui       crate hawier-ui     → binary `hawier-ui`  (server-side rendered console)
apps/client   crate hawier-client → binary `hawier-device` (device simulator / load client)
apps/ext-fs               wasm `hawier_ext_fs`               (storage in a directory)
apps/ext-s3               wasm `hawier_ext_s3`               (storage in S3, presigned URLs)
apps/ext-webhook          wasm `hawier_ext_webhook`          (event delivery)
apps/ext-retention        wasm `hawier_ext_retention`        (artifact retention policy)
apps/ext-rollout-classic  wasm `hawier_ext_rollout_classic`  (rollout: hawkBit percentages and thresholds)
apps/ext-rollout-bucket   wasm `hawier_ext_rollout_bucket`   (rollout: canary waves)
apps/ext-console          wasm `hawier_ext_console`          (console tabs and views)
apps/ext-authz            wasm `hawier_ext_authz`            (permissions: roles and grants)
libs/api      crate hawier-api    (shared serde DTOs and enum codes; the only library)
```

Integration tests: one target per crate — `apps/server/tests/it/` and
`apps/ui/tests/it/` (`make it`); each test function gets its own DB
and an in-process server on an ephemeral port.

## Server module map

```
config      HAWIER_* env; node role (control|sub)
db          two sqlx pools (mgmt/ui + a dedicated ddi), migrations:
            hawkbit track (verbatim SQL + Flyway rows in schema_version)
            + hawier track (hawier_* tables in hawier_schema_version)
auth        Mgmt Basic (static users), DDI TargetToken/GatewayToken,
            /ui/v1 Bearer; constant-time comparisons
rsql        FIQL parser → parameterized SQL (per-resource field maps)
artifact    spool + hashing and descriptor-based IO from the storage extension
ext         extension host: registry + loading and calling wasm modules
ddi         15 DDI routes (+ `.MD5SUM` on the artifact) + feedback state machine
mgmt        all /rest/v1 resource groups (json! responses = wire contract)
ui_state    /ui/v1 for hawier-ui: views, action log (write), searches,
            sessions, node version, console extension surfaces
deploy_logic DS→target assignment (sp_action creation, supersede, confirmation)
rollout     rollout engine + auto-assign (control node only)
housekeeping / retention / outbox  control-node background loops: cleanup
            of actions and sessions, retention rules, event delivery
authz / incident  per-request permission classification; incident numbers and log
```

## Console module map (`apps/ui`)

```
templates/  all markup (askama): base.html + pages + macros/panels
view        typed page view models and the shared cell vocabulary (Cell)
routes      routes, building models from upstream JSON, form handling
resources   resource declarations: columns, system-views, forms, bulk operations
hawkbit     /rest/v1 client (Basic replay on behalf of the user)
ui_api      internal /ui/v1 client (views, logs, searches, sessions)
html        small pure helpers: URL encoding, reading JSON paths, time
```

Rule: HTML lives only in `templates/`; Rust supplies data. The only
exception is `view::icon` (an svg shape).

## Distributed topology

```
                    ┌─────────────┐
   operators ─────► │ hawier-ui   │
                    └──────┬──────┘
                           │ /rest/v1 + /ui/v1
                    ┌──────▼──────┐         ┌──────────────┐
                    │ control ×1  │ ──────► │  PostgreSQL  │
                    │ mgmt+ddi+   │         └──────▲───────┘
                    │ engines     │                │
                    └─────────────┘         ┌──────┴───────┐
   devices ───────────────────────────────► │  sub ×N      │
   (DDI poll/download/feedback)             │  ddi+ro-mgmt │
                                            └──────────────┘
                    artifact storage — an extension on every node
```

- **One database for all nodes — there is no inter-node synchronization by
  construction.** hawier nodes are stateless: all state (targets, actions,
  rollouts, attributes, artifact metadata) lives in the shared PostgreSQL,
  artifact binaries in shared storage (volume/NFS/S3). A dead node simply
  reads the current state from the database after a restart; it has nothing
  to lose. The production path for HA of the database itself is standard PG
  (streaming replication / Patroni); nodes switch to the new primary.
  Physically independent DBs per node (multi-master with conflict
  resolution) are deliberately out of scope at this stage.
- **How devices find nodes**: the hawkBit DDI protocol has no discovery —
  a device knows the base URL of the central node/LB and follows `_links`
  from there. A documented API shortcoming: the protocol cannot delegate
  *polling* itself to geo-distributed sub nodes, and with a fleet of tens of
  millions of devices polling is the main load. The plan for splitting it
  off (a `pollingBase` link in the controller-base response) and the
  doctrine "the client must follow all links and never build URLs itself" are in
  [API_NOTES.md](API_NOTES.md). The future role of sub nodes in rollouts
  (the center controls — subs execute and report back) is recorded there too.
- **control** (one node): full Mgmt, DDI, `/ui/v1`, rollout engine,
  auto-assign, housekeeping. `HAWIER_ROLE=control` (the default) — a single
  node is a complete installation.
- **sub** (N nodes): DDI + artifact delivery + read-only Mgmt (writes →
  405) + `/ui/v1`. No background loops.
- **Control node failure**: the DDI path consists of pure request-scoped DB
  transactions, with no singletons, so poll/download/feedback keep working
  through sub nodes (verified by an integration test). Only the *advancement*
  of rollouts (next groups) and auto-assign pause — a deliberate tradeoff;
  once the control node returns, the engine resumes where it stopped
  (all transitions are idempotent, the rollout claim goes through
  `FOR UPDATE SKIP LOCKED`).

## hawkBit compatibility (temporary, development stage)

- The DB schema = hawkBit 1.0.0: Flyway baseline `B1_20_0` + `V1_20_1` +
  `V1_20_2`, applied verbatim; history is written to `schema_version`
  as Flyway-compatible rows (a replica of the checksum algorithm —
  line-by-line CRC32, `apps/server/src/db/flyway.rs`).
- Compatibility was confirmed by a one-off check (2026-08-21): stock
  hawkBit started on a database migrated by hawier — Flyway validation
  without `repair`. Running alongside hawkBit is not supported as a
  scenario; migration determinism is pinned by a test of the Flyway rows.
- Own tables — only `hawier_*` (twelve: views, action log, incidents,
  searches, sessions, outbox, webhooks and their deliveries, retention
  rules, rollout strategy parameters, roles, grants), track
  `hawier_schema_version`.
- Narrow places of the inherited schema are catalogued in SCHEMA_NOTES.md and
  marked `// NARROW:` in code; they are fixed in pre-production revisions.
- The on-disk artifact layout follows hawkBit
  (`{dir}/{TENANT}/{sha1[-4:-2]}/{sha1[-2:]}/{sha1}`) — a directory shared
  with hawkBit is possible for the duration of a migration.

## The 10 Gbit path

1. **Streaming everywhere**: download is `tokio::fs` → `ReaderStream` (256 KiB)
   → `Body::from_stream`; a file is never buffered whole; no serde on the
   binary path. ETag=sha1, `If-Match`/`If-None-Match`, single-range 206 —
   fleets resume downloads instead of redownloading.
2. **Pool isolation**: a dedicated `ddi_pool` — Mgmt/UI bursts do not starve
   devices.
3. **Horizontal scaling is the main answer**: N sub nodes behind an LB; a shared
   volume now, S3 later.
4. **S3 mode** (the `apps/ext-s3` extension): the extension hands out a
   presigned URL with `redirect_ok`, the host answers the device with a `302` — hawier
   leaves the byte path entirely, keeping authorization, ETag and
   recording of the `DOWNLOAD` status. Optionally — `X-Accel-Redirect` behind nginx to
   serve from local disk.

Measurements (`make load`, a single release binary, loopback, simulator devices
verifying sha1+md5 on the same machine — so the measurement is conservative,
client-side hashing eats the same cores):

| Fleet | Artifact | Time | Speed |
|---|---|---|---|
| 200 | 4 MB | 4.1 s | 49 updates/s, ~1.6 Gbit/s payload |
| 1000 | 8 MB | 8.3 s | 121 updates/s, ~7.7 Gbit/s payload |
| 2000 | 8 MB | 24.5 s | 82 updates/s, ~5.2 Gbit/s payload |

The time includes both the granularity of the rollout engine tick (2 s) and the
status poll (2 s). 10 Gbit in total is reached with 2+ sub nodes; the per-node
measurement is the base.

Polling (`make load-poll`, 2000 devices @100ms, one release node):
before hot-path optimizations — 7.7k polls/s at avg 287 ms; after
(tenant-config cache, lazy last_target_query, a single LATERAL query) —
**15.5k polls/s at avg 125 ms** (~2×). History — bench/poll-history.csv.

## The extension mechanism

In full — in [EXTENSIONS.md](EXTENSIONS.md); here only what affects the
big picture. An extension is a **wasm module** that a node loads and
calls. There is no other transport, and no in-process tier either:
a decision that belongs to an extension is made by the extension, or it is
not made and the host answers with a declared refusal. Choosing wasm rather than
a separate process is about authority: a module has none at all, apart from
what is granted by imports (we give clocks and `HAWIER_EXT_*`), whereas a native
program has the same authority as its user. First-party extensions live
right here — `apps/ext-fs`, `apps/ext-s3`, `apps/ext-webhook` — and
the integration tests load exactly them.

The invariant from which everything else follows: **decisions and descriptors
cross the extension boundary, never bytes and never handles.** An extension
does not hold a `PgConnection` and does not stand inside `FOR UPDATE SKIP LOCKED` — the host
loads a snapshot, calls with a deadline, and applies the decision in its own transaction.
Host duties are not extension points: moving bytes,
transactions and locks, the rollout engine state machine, secret issuance.
The telling case is s3: the extension hands out a presigned link, the host does
a 302, and the bytes do not go through hawier at all.

Each point declares an execution site — `control` (background loop),
`request` (mgmt/UI path) or `ddi` (device path, **including sub nodes**).
Storage lives on `ddi`, so the module is mandatory on every node
that serves devices; there is no fallback, and a node without it
honestly answers `503` with `Retry-After`.

Webhooks, cron retention, remote-shell, oauth/permissions, custom console
tabs, storage and rollout logic are expressed through this mechanism —
including the classic one. Hence the hardened profile (§6 of
[API_NOTES.md](API_NOTES.md)): it is a launch profile, a node without the
classic rollout extension.

## Decisions and tradeoffs

- **No ORM**: the legacy schema (nullable booleans, int enums with gaps,
  epoch-millis) maps onto explicit sqlx queries better than onto any ORM.
- **Mgmt responses as `serde_json::json!`**: the wire format is the contract;
  typing 70 DTOs for the sake of the types themselves is not a simplification.
- **Storage is an extension, not a backend enum**: only the spool
  with hashing and descriptor-based IO remains in the core; where bytes are stored is decided by a wasm module
  (`ext-fs`, `ext-s3`). There is no built-in backend, so a node without the module
  answers `503` rather than "somehow managing on its own".
- **One shared crate `hawier-api`**: the server, client and UI all need the DTOs;
  we add no more libraries (the "no micro-libraries" rule).
- **Test client = production simulator**: `hawier-device --count N` is both
  the integration driver for tests and the load tool.

## Specs

What the system does — normatively, with verifiable scenarios — lives in
`openspec/specs/`; this file explains why it is built this way. Change
behavior through an OpenSpec change (`/opsx:propose`).

| Area | Specs |
|---|---|
| Nodes and devices | [node-roles](openspec/specs/node-roles/spec.md), [ddi-api](openspec/specs/ddi-api/spec.md), [artifact-delivery](openspec/specs/artifact-delivery/spec.md), [device-simulator](openspec/specs/device-simulator/spec.md) |
| Management | [mgmt-api](openspec/specs/mgmt-api/spec.md), [rsql-query](openspec/specs/rsql-query/spec.md), [access-control](openspec/specs/access-control/spec.md), [incidents](openspec/specs/incidents/spec.md), [tenant-configuration](openspec/specs/tenant-configuration/spec.md), [ui-state-api](openspec/specs/ui-state-api/spec.md) |
| Deployments | [deployment-assignment](openspec/specs/deployment-assignment/spec.md), [rollout-engine](openspec/specs/rollout-engine/spec.md) |
| Background loops | [action-housekeeping](openspec/specs/action-housekeeping/spec.md), [artifact-retention](openspec/specs/artifact-retention/spec.md), [event-delivery](openspec/specs/event-delivery/spec.md) |
| Extensions | [extension-host](openspec/specs/extension-host/spec.md), [first-party-extensions](openspec/specs/first-party-extensions/spec.md) |
| Data and build | [database-schema](openspec/specs/database-schema/spec.md), [build-identity](openspec/specs/build-identity/spec.md) |
| Console | [console-shell](openspec/specs/console-shell/spec.md), [console-lists](openspec/specs/console-lists/spec.md), [console-objects](openspec/specs/console-objects/spec.md), [console-bulk-actions](openspec/specs/console-bulk-actions/spec.md), [console-search](openspec/specs/console-search/spec.md), [console-settings](openspec/specs/console-settings/spec.md), [console-help](openspec/specs/console-help/spec.md), [console-visual-language](openspec/specs/console-visual-language/spec.md) |
| Tooling | [build-tooling](openspec/specs/build-tooling/spec.md), [load-benchmark](openspec/specs/load-benchmark/spec.md), [remote-deployment](openspec/specs/remote-deployment/spec.md), [tls-proxy](openspec/specs/tls-proxy/spec.md) |
