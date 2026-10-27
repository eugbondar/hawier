# Configuration

Both binaries are configured through the environment. Tenant settings —
polling time, token authentication, assignment semantics, maintenance
windows — are not environment: they live in the database and are edited on
the console's Settings page or through `/rest/v1/system/configs` (specs
[`tenant-configuration`](../../openspec/specs/tenant-configuration/spec.md),
[`deployment-assignment`](../../openspec/specs/deployment-assignment/spec.md),
[`ddi-api`](../../openspec/specs/ddi-api/spec.md)).

## Server (`hawier`)

| Variable | Default | Meaning |
|---|---|---|
| `HAWIER_BIND_ADDR` | `0.0.0.0:8080` | server address |
| `HAWIER_DATABASE_URL` | `postgres://hawkbit:hawkbit@localhost:5432/hawkbit` | PostgreSQL |
| `HAWIER_ROLE` | `control` | `control` (everything) or `sub` (DDI + read-only Mgmt, no background engines) — spec [`node-roles`](../../openspec/specs/node-roles/spec.md) |
| `HAWIER_ARTIFACT_DIR` | `./artifacts` | the root inside which the node accepts paths from the storage extension; see the note below |
| `HAWIER_MGMT_USERS` | `admin:admin:DEFAULT` | `user:pass:tenant,...` |
| `HAWIER_INTERNAL_TOKEN` | `dev-internal-token` | Bearer for `/ui/v1`; the console must have the same one |
| `HAWIER_DDI_TARGETTOKEN_ENABLED` | `true` | default of the tenant setting `authentication.targettoken.enabled` |
| `HAWIER_DDI_GATEWAYTOKEN_ENABLED` | `false` | default of `authentication.gatewaytoken.enabled` |
| `HAWIER_DDI_ANONYMOUS` | `false` | anonymous DDI (development only) |
| `HAWIER_DB_WAIT_SECS` | `60` | how long to wait for the database at startup; after a reboot Postgres may come up later than the node |
| `HAWIER_MGMT_POOL_SIZE` / `HAWIER_DDI_POOL_SIZE` | 10 / 20 | database pools; DDI has its own |
| `HAWIER_MAX_ARTIFACT_SIZE` | `1073741824` (1 GiB) | request body limit for artifact uploads |
| `HAWIER_MAX_CONNECTIONS` | `4096` | requests handled concurrently; the excess waits in a queue |
| `HAWIER_TENANT_CACHE_TTL_MS` | `5000` | tenant config cache on the DDI path; other nodes see a change within the TTL (`0` — no cache) |
| `HAWIER_POLL_TOUCH_INTERVAL_MS` | `30000` | how often a poll rewrites the device's last-poll time |
| `HAWIER_HOUSEKEEPING_INTERVAL_MS` | `60000` | tick of the housekeeping loop (control only): action cleanup per `action.cleanup.auto.*` and expired console sessions |
| `HAWIER_NODE_ID` | hash of hostname | node number (0..1023) inside incident numbers — spec [`incidents`](../../openspec/specs/incidents/spec.md) |
| `HAWIER_EXT_STORAGE` | — | path to the storage `.wasm`; empty — artifacts are answered with 503 |
| `HAWIER_EXT_EVENT_SINK` | — | path to the event-delivery `.wasm`; empty — events are not delivered (a control node currently marks outbox rows delivered anyway) |
| `HAWIER_EXT_RETENTION` | — | path to the retention-policy `.wasm`; empty — rules do not fire |
| `HAWIER_EXT_ROLLOUT` | — | path to the rollout-strategy `.wasm`; empty — a rollout is not split into groups |
| `HAWIER_EXT_CONSOLE` | — | path to the `.wasm` with console tabs; empty — only the built-in tree |
| `HAWIER_EXT_AUTHZ` | — | path to the permission-policy `.wasm`; empty — everything authenticated is allowed |
| `HAWIER_EXT_DEADLINE_MS` | `2000` | deadline of a single extension call |
| `HAWIER_EXT_PARAMS_FILE` | — | extension parameters file (`KEY=value`), editable from the console; empty — editing is off, only the environment counts |
| `HAWIER_EXT_*` | — | everything else with this prefix is passed to the modules, and nothing else is |

**Artifact root.** A descriptor path from the storage extension is accepted
only if it is absolute and lies under `HAWIER_ARTIFACT_DIR`, and the root
itself is not canonicalized. Set it absolute and equal to
`HAWIER_EXT_FS_ROOT`: with the relative default, every download served
through `ext-fs` answers 500.

## Extension parameters

`HAWIER_EXT_*` values reach the modules from two places: the environment and
the file named by `HAWIER_EXT_PARAMS_FILE`, which the console's Settings page
edits. The environment **wins** over a line of the same name in the file —
this is how a deployment pins a value that must match the machine. Module
paths (`HAWIER_EXT_STORAGE` and its neighbours), `HAWIER_EXT_DEADLINE_MS` and
`HAWIER_EXT_PARAMS_FILE` itself cannot be set from the file: choosing a
module is a trust decision, not a parameter. Changes take effect on **node
restart**. Contract: spec
[`extension-host`](../../openspec/specs/extension-host/spec.md).

## Console (`hawier-ui`)

| Variable | Default | Meaning |
|---|---|---|
| `HAWKBIT_MGMT_URL` | `http://localhost:8080` | where the console sends Mgmt requests |
| `HAWIER_API_URL` | the Mgmt URL | the node's `/ui/v1` state API |
| `HAWIER_INTERNAL_TOKEN` | `dev-internal-token` | Bearer for `/ui/v1`, equal to the node's |
| `HAWIER_BIND_ADDR` | `0.0.0.0:8088` | console address |
| `HAWIER_STATIC_DIR` | `static` | stylesheet and script |
| `HAWIER_DEV_MODE` | off | `1`/`true` — answer a clear 404 for assets not yet built |
