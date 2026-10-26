# hawier

A Rust implementation of a device update server, compatible with Eclipse hawkBit
(Mgmt + DDI API, PostgreSQL), plus a web console and a device simulator.
One monorepo, as few moving parts as possible.

> **Compatibility (temporary, development stage):** the DB schema is
> byte-for-byte identical to hawkBit 1.0.0, and the Mgmt/DDI API follows its
> wire contracts, so that existing devices, tooling and stock hawkBit work with
> the same database during migration. The narrow places of the inherited schema
> are collected in [SCHEMA_NOTES.md](SCHEMA_NOTES.md) and will be fixed in the
> pre-production revisions of hawier's own schema. Details are in
> [ARCHITECTURE.md](ARCHITECTURE.md).

## Components

| What | Where | Binary | Port |
|---|---|---|---|
| Server (Mgmt `/rest/v1`, DDI `/{tenant}/controller/v1`, `/ui/v1`) | `apps/server` | `hawier` | 8080 |
| Web console (server-side rendered) | `apps/ui` | `hawier-ui` | 8088 |
| Device simulator / load client | `apps/client` | `hawier-device` | — |
| Shared DTOs and enum codes | `libs/api` | — | — |

AMQP/DMF is not supported (deliberately).

## Quick start

You need `cargo`, a container engine (**podman or docker** — whichever is
found, podman first; `ENGINE=docker make ...` overrides) and GNU Make as the
control centre.

```bash
make up          # postgres + hawier-control + hawier-ui (compose)
```

- Console: http://localhost:8088 (admin / admin, tenant DEFAULT)
- Mgmt API: http://localhost:8080/rest/v1 (HTTP Basic admin:admin)
- Simulator: `make device ARGS="--controller-id dev-1 --gateway-token <key>"`

Development without containers (except Postgres):

```bash
make it-keep     # bring up postgres and run the integration tests
make run         # server locally (control node) on :8080
make run-ui      # console locally on :8088
```

## Make targets

| Target | Action |
|---|---|
| `make all` | check-tools + lint + test + build |
| `make check-tools` | whether `cargo`, a container engine and `shellcheck` are present |
| `make fmt` | `cargo fmt --all` |
| `make lint` | `cargo fmt --check` + `clippy -D warnings` (including both `tests/it` targets) + `shellcheck` over `e2e/` |
| `make test` | unit tests (no DB) |
| `make it` / `make it-keep` | integration tests against a real Postgres (compose) |
| `make ui-audit` | console UI/UX audit only (quick run) |
| `make reset-ui-state` | recreate `hawier_*` after editing `0001_hawier.sql` |
| `make run` / `make run-ui` | server / console locally |
| `make device ARGS="..."` | device simulator |
| `make ext` / `make ext-release` | build the wasm extension modules (debug / release) |
| `make load` / `make load-poll` | update / polling load run (see below) |
| `make e2e-image` | reference device image (Buildroot, SquashFS A/B, signed `.swu`) in `target/e2e/out` |
| `make e2e` / `E2E_ONLY=<scenario> make e2e` | hawier against SWUpdate/suricatta in QEMU, hardened profile (see below) |
| `make gc` / `make gc-dry` | remove unreachable artifacts from `target/debug` / show what would go |
| `make build` / `make docker` | release build / images (with whichever engine is found) |
| `make up` / `make down` / `make logs` / `make ps` | compose stack |
| `make clean` | `cargo clean` + remove images (also wipes `target/release/hawier-device` and the wasm modules — you usually want `make gc`) |
| `CARGO_PROFILE=release make scale` | the same, but with an optimized build — for measurements and anything that leaves the machine |
| `make deploy-<host>` / `make deploy-list` | deploy the stack to a host from `deploy/ansible/inventory.yml` / list known hosts |
| `make logs-<host>` | what the stack on the host is doing: container states, restarts, the log tail of each service |
| `make creds-<host>` | the console address and how to sign in |
| `make rollback-<host> VERSION_OVERRIDE=v…` | bring up a version on the host whose image is already there |

## Device simulator

```bash
# one device, one cycle (poll → download → verify sha1/md5 → feedback)
hawier-device --url http://localhost:8080 --controller-id dev-1 --token <sec_token> --once

# a fleet of 500 devices via gateway token
hawier-device --count 500 --controller-id fleet --gateway-token <key> --poll-override 5s

# fault injection
hawier-device --fail-download ...      # closes updates with failure
hawier-device --deny-confirmation ...  # rejects the confirmation flow
```

The client builds only the initial poll URL itself; everything else comes from
the `_links` in server responses (the doctrine and the geo-polling roadmap are
in [API_NOTES.md](API_NOTES.md)).

## Load test

`make load` is a real e2e run against a running hawier (`make up` or
`make run`): it creates an artifact, a rollout to a fleet of simulators, waits
for FINISHED on all of them and prints updates/s and Gbit/s. Parameters:

| Variable | Default | Meaning |
|---|---|---|
| `URL` | `http://localhost:8080` | the node Mgmt goes through |
| `DDI_URL` | `= URL` | where the fleet connects (e.g. a sub node on `:8081`) |
| `COUNT` | `200` (at least 2) | fleet size |
| `SIZE_MB` | `4` | artifact size |
| `TIMEOUT_S` | `600` | how long to wait for FINISHED on all |
| `USER_PASS` | `admin:admin` | Mgmt account |
| `GATEWAY_KEY` | `load-gateway-key` | gateway key the fleet registers with |
| `DEVICE_BIN` | `target/release/hawier-device` | the simulator (built automatically by default) |
| `DEVICE_LOG` | `/dev/null` | where to write the fleet log |
| `HISTORY` | `bench/load-history.csv` | where to append the row |
| `PROFILE_CHECK` | `1` | `0` — don't check that the nodes are release (the measurement then doesn't count for history) |
| `KEEP` | `0` | `1` — don't delete what was created on the server |

`make load-poll` takes `URL`, `DDI_URL`, `COUNT` (2000), `INTERVAL` (`1s`),
`DURATION_S` (60), `USER_PASS`, `GATEWAY_KEY`, `HISTORY`
(`bench/poll-history.csv`) and `KEEP`. The behaviour of both targets is the spec
[`load-benchmark`](openspec/specs/load-benchmark/spec.md).

```bash
COUNT=1000 SIZE_MB=8 make load
```

Each run appends a row to `bench/load-history.csv` (the history lives in the
repository) and cleans up the fleet it created (`KEEP=1` — don't clean up).
How to read the history and what to beware of in it — `bench/NOTES.md`.
**Regression rule:** run `make load` before a release and
after changes to the hot path (DDI, artifact, rollout engine) and compare
with the history; a drop in updates/s or MB/s of more than ~20% with the same
parameters is a regression, to be dealt with before merging. Single-node
measurements are in [ARCHITECTURE.md](ARCHITECTURE.md).

## Run on a real client (e2e)

`make e2e` checks hawier not with its own simulator but with SWUpdate/suricatta —
the client that real fleets run on. Devices from the reference image
(Buildroot, SquashFS, A/B) are brought up in QEMU, the node runs in the hardened
profile, and six scenarios are run: a full A/B cycle, refusal on signature,
refusal on the device's hwrevision, the server's refusal of an incompatible
assignment, a rollout over a mixed fleet, and rollback of a slot that failed to
boot. Behaviour is defined by the specs
[`device-e2e`](openspec/specs/device-e2e/spec.md) and
[`reference-device-image`](openspec/specs/reference-device-image/spec.md).

The host needs a container engine (the Buildroot build runs in a builder
container), `qemu-system-x86_64` and `qemu-img`, `curl`, `python3`. KVM is used
if `/dev/kvm` is readable and writable; otherwise — software emulation with
deadlines ×6 (`E2E_ACCEL=tcg` forces it). The first image build downloads
sources through the same proxy as the other builds; a repeat build downloads
nothing and rebuilds nothing.

```bash
make e2e-image
```

```bash
make e2e
```

```bash
E2E_ONLY=fallback make e2e
```

Scenarios: `incompatible-assignment`, `update`, `bad-signature`,
`wrong-revision`, `rollout`, `fallback`. The result of each (PASS/FAIL and
duration) is in `target/e2e/report.txt`; serial logs of the devices, the node
log and the action history of each target are in `target/e2e/logs/`. The node
under test is a dev binary on `127.0.0.1:18080` with a separate database
`hawier_e2e`; after the run no QEMU, no node and no running PostgreSQL is left.

A device by hand (e.g. to figure out the image): a copy of the disk on top of
the reference, revision and identity via fw_cfg, as the harness does. Without
an identity SWUpdate does not start and says so on the console; login is root
with no password on `ttyS0` (test image):

```bash
qemu-img create -f qcow2 -F raw -b "$PWD/target/e2e/out/disk.img" /tmp/dev.qcow2
```

```bash
qemu-system-x86_64 -accel kvm -m 192 -nographic -drive file=/tmp/dev.qcow2,if=virtio,format=qcow2 -netdev user,id=n0,restrict=on -device virtio-net-pci,netdev=n0 -fw_cfg name=opt/org.hawier/hwrevision,string=rev1
```

**Type compatibility is now enforced.** Assigning a set to a target whose type
does not declare the set's type compatible is answered with `400
target.type.incompatible` for the whole request; rollouts and auto-assignment
skip such targets. This is how hawkBit 1.0 behaves; previously hawier stored the
relation but did not check it (API_NOTES §9.10). With the tenant key
`target.type.required` a target without a type gets nothing — part of the
hardened profile.

### Budget

The image is also built on a free GitHub runner, so it has a budget;
`make e2e-image` fails, naming the exceeded quantity.

| Quantity | Now | Limit |
|---|---|---|
| slot (`slot-v1.squashfs`) | 6.5 MiB | 8 MiB |
| each `.swu` | 6.5 MiB | 8 MiB |
| `disk.img` | 53 MiB | 64 MiB |
| build tree (`br-output` + `dl` + `ccache`) | 3.9 GiB | 4 GiB |
| rebuild with warm `dl/` and `ccache` (here, 32 cores) | 5 min 21 s | — |
| cold build on a 4-vCPU runner | to be measured on the first CI run | 30 min |
| the run itself, 6 scenarios | ~1.5 min (KVM), ~2.5 min (TCG) | — |

## Environment variables (server)

| Variable | Default | Meaning |
|---|---|---|
| `HAWIER_BIND_ADDR` | `0.0.0.0:8080` | server address |
| `HAWIER_DATABASE_URL` | `postgres://hawkbit:hawkbit@localhost:5432/hawkbit` | PostgreSQL |
| `HAWIER_ROLE` | `control` | `control` (everything) or `sub` (DDI + read-only Mgmt, no background engines) |
| `HAWIER_ARTIFACT_DIR` | `./artifacts` | the root inside which the host accepts paths from the storage extension. **Set it absolute and equal to `HAWIER_EXT_FS_ROOT`**: a descriptor path is accepted only if it is absolute and lies under this root, so with the relative default any serving through `ext-fs` is a 500 (a discrepancy, see design.md `baseline-existing-capabilities` item 20) |
| `HAWIER_MGMT_USERS` | `admin:admin:DEFAULT` | `user:pass:tenant,...` |
| `HAWIER_INTERNAL_TOKEN` | `dev-internal-token` | Bearer for `/ui/v1` (the console has the same one) |
| `HAWIER_DDI_TARGETTOKEN_ENABLED` | `true` | default of the tenant setting `authentication.targettoken.enabled` |
| `HAWIER_DDI_GATEWAYTOKEN_ENABLED` | `false` | default of `authentication.gatewaytoken.enabled` |
| `HAWIER_DDI_ANONYMOUS` | `false` | anonymous DDI (dev only) |
| `HAWIER_DB_WAIT_SECS` | `60` | how long to wait for the database at startup: on a single host Postgres comes up alongside, and after a reboot the order is not guaranteed — the node waits instead of exiting |
| `HAWIER_MGMT_POOL_SIZE` / `HAWIER_DDI_POOL_SIZE` | 10 / 20 | DB pools (DDI is isolated) |
| `HAWIER_MAX_ARTIFACT_SIZE` | `1073741824` (1 GiB) | request body limit for artifact uploads |
| `HAWIER_MAX_CONNECTIONS` | `4096` | how many requests the node handles concurrently; the excess wait in a queue |
| `HAWIER_TENANT_CACHE_TTL_MS` | `5000` | tenant config cache on the DDI path; other nodes see a change no later than after the TTL (`0` — no cache) |
| `HAWIER_POLL_TOUCH_INTERVAL_MS` | `30000` | how often a poll rewrites the device's last-poll time |
| `HAWIER_HOUSEKEEPING_INTERVAL_MS` | `60000` | tick of the housekeeping loop (control only): cleanup of actions per `action.cleanup.auto.*` + deletion of expired UI sessions |
| `HAWIER_NODE_ID` | hash of hostname | node number (0..1023) inside incident numbers |
| `HAWIER_EXT_STORAGE` | — | path to the storage `.wasm`; empty = artifacts are served as 503 |
| `HAWIER_EXT_EVENT_SINK` | — | path to the event-delivery `.wasm`; empty = events are not delivered (on a control node outbox rows are currently marked delivered — a discrepancy, item 8 of the catalogue) |
| `HAWIER_EXT_RETENTION` | — | path to the retention-policy `.wasm`; empty = rules do not fire |
| `HAWIER_EXT_ROLLOUT` | — | path to the rollout-strategy `.wasm`; empty = a rollout is not split into groups |
| `HAWIER_EXT_CONSOLE` | — | path to the `.wasm` with console tabs; empty = only the built-in tree |
| `HAWIER_EXT_AUTHZ` | — | path to the permission-policy `.wasm`; empty = everything authenticated is allowed |
| `HAWIER_EXT_DEADLINE_MS` | `2000` | deadline of a single extension call |
| `HAWIER_EXT_PARAMS_FILE` | — | extension parameters file (`KEY=value`), editable from the console; empty = editing is off, only the environment works |
| `HAWIER_EXT_*` | — | everything else with this prefix is passed to the module (and only that). The environment **wins** over a same-named file line: this is how a deployment pins a value that must match the machine |

Module paths (`HAWIER_EXT_STORAGE` and neighbours), `HAWIER_EXT_DEADLINE_MS` and
`HAWIER_EXT_PARAMS_FILE` itself cannot be set from the file: choosing a module
is a trust decision, not a parameter. Changes take effect on **node restart**.
The contract is the spec [`extension-host`](openspec/specs/extension-host/spec.md).

Every request gets an **incident number** — a decimal snowflake (milliseconds
since 2026-01-01 + node number + counter) in `X-Request-Id`; on error the same
number arrives as the `"incident"` field in the JSON body, is logged to
`hawier_incident` and opens directly in the UI: `/incidents/{number}`
(spec [`incidents`](openspec/specs/incidents/spec.md)).

Assignment semantics are as in hawkBit, controlled by tenant configs
(edited in the UI on the Settings page): `multi.assignments.enabled`
(several active actions, DDI picks by weight DESC, id ASC),
`repository.actions.autoclose.enabled` (supersede closes the old action
immediately as CANCELED, without the cancel handshake), `implicit.lock.enabled`
(an assignment/rollout locks the DS and its modules against changes).
Maintenance windows (`maintenanceWindow`: Quartz cron + duration `HH:mm:ss` +
offset `±hh:mm`) are fully supported: DDI returns
`available`/`unavailable`, outside the window `update: skip` (the device
downloads but does not install), the sleep before the window is shortened per
`maintenanceWindowPollCount`. All keys and their effect are in
[`tenant-configuration`](openspec/specs/tenant-configuration/spec.md),
[`deployment-assignment`](openspec/specs/deployment-assignment/spec.md),
[`ddi-api`](openspec/specs/ddi-api/spec.md).

Console: `HAWKBIT_MGMT_URL` (where to go for Mgmt), `HAWIER_API_URL`
(the `/ui/v1` state API, defaults to the Mgmt URL), `HAWIER_INTERNAL_TOKEN`,
`HAWIER_BIND_ADDR` (`:8088`), `HAWIER_STATIC_DIR` (`static`), `HAWIER_DEV_MODE`
(`1`/`true` — answer with a clear 404 for assets not yet built).

## hawkBit DB compatibility

hawier applies the original hawkBit migrations (baseline `B1_20_0` +
`V1_20_1` + `V1_20_2`) verbatim and writes a Flyway-compatible history to
`schema_version` (verified by running stock hawkBit against a hawier database —
validation passed without `flyway repair`; simultaneous operation with hawkBit
is not supported as a working scenario and is not needed). hawier's own data
(views, action log, console sessions and searches, incidents, outbox and
webhooks, retention rules, roles and grants, rollout strategy parameters)
lives in twelve `hawier_*` tables and does not overlap with `sp_*` — spec
[`database-schema`](openspec/specs/database-schema/spec.md).

## Scaling

One control node (Mgmt + rollout engine) + N sub nodes (DDI + artifact
serving). If the control node goes down, devices keep updating through the sub
nodes. Demonstration:

```bash
make scale                                                  # + sub node on :8081
podman compose -f deploy/docker-compose.yml stop hawier-control   # or docker compose — whichever engine you have
hawier-device --url http://localhost:8081 --count 50 --controller-id fleet --gateway-token ...
```

Node roles and what survives the death of the control node — spec
[`node-roles`](openspec/specs/node-roles/spec.md).

The 10 Gbit strategy (streaming, pool isolation, S3 presigned redirect) is in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Deployment

The simplest install — one edit, one command:

1. In [`deploy/ansible/inventory.yml`](deploy/ansible/inventory.yml), set
   `ansible_host` of the `server` entry to your machine (and `ansible_user`,
   if you do not log in as root).
2. `make deploy-server`

When it finishes, the console is at `http://<your host>:8088`, devices talk to
`http://<your host>:8080`, and `make creds-server` prints the login (now or any
time later). Before real devices, turn on TLS: see the commented examples in
the same file and [TLS](#tls) below.

**This** machine needs `ansible-core` (`pipx install ansible-core`) and
internet access for the build. The target needs ssh and three things, and the
role installs none of them (it says so and stops — this takes a second, before
any build):

| On the host | Why |
|---|---|
| `python3` | ansible itself runs on it; without it even fact gathering doesn't start |
| podman or docker **with compose** | `podman compose` exists since podman 4.9, otherwise `podman-compose` is needed; with docker it is the v2 plugin, compose v1 will not read this file |
| ~4 GB of free space for images | `hawier_min_free_mb` |

The target machine needs no internet at all.

A host is described once in the inventory — address, ssh port, user and node
number; the file has commented examples for TLS, a small machine and a host
behind your own proxy. Hosts you would rather keep out of git go to
`deploy/ansible/inventory.local.yml` (ignored, same format); make reads both.
Editing the inventory does not count as a dirty tree for a deploy: it is not
part of the images.

```bash
make deploy-list          # which hosts are described
make deploy-<host>        # build, ship, bring up, verify
```

What happens: preflight rejects the host before the build; release images are
built (`CARGO_PROFILE=release` forced); only what is missing there is streamed
to the host via `save | zstd | ssh | load` (exact image names are compared — ids
differ between podman and docker), including the base `postgres` — **the target
machine downloads nothing**; a self-contained compose file is rendered; the play
waits for health, recognizing a crash loop; the deployment ends by checking
`node.build`/`node.profile`/`node.role` on the node and the build on the
console. The full behaviour, with failure conditions, is in the specs
[`remote-deployment`](openspec/specs/remote-deployment/spec.md) and
[`build-tooling`](openspec/specs/build-tooling/spec.md).

The host gets `/opt/hawier` with `docker-compose.yml` (values written as
literals, file 0600, no `.env`) and `secrets/`. The Postgres password,
`HAWIER_INTERNAL_TOKEN` and `HAWIER_MGMT_USERS` are generated **once** on the
first deploy and only read afterwards: `POSTGRES_PASSWORD` takes effect only at
volume initialization. The credentials are printed on the run where they were
born, and to ask for them later — `make creds-<host>`. To change the console
password:

```bash
make deploy-<host> ANSIBLE_ARGS='-e hawier_mgmt_users=admin:NEW:DEFAULT'
```

This way it ends up neither in the inventory nor in git. The Postgres password
is not changed this way: the volume keeps the one it was created with — the role
will refuse and suggest `ALTER USER`.

The cause of a failure is printed by make — as plain text that can be read and
forwarded. When a deploy fails, the report is printed immediately; separately
and at any time — `make logs-<host>`, with no deploy, no version and not a
single command on the host (`DIAG_LINES=300` if you need more lines). It is also
saved to `hawier-<host>.log`. Everything in it is asked **of the engine**, not
of compose:

```
=== state ===
postgres        running     exit=0    restarts=0    healthy
hawier-control  restarting  exit=1    restarts=7    unhealthy
hawier-ui       running     exit=0    restarts=0    starting

=== log: hawier-control (last 5) ===
Error: loading the wasm extension /nope.wasm
...
```

Wait timings — `hawier_compose_timeout`, `hawier_health_retries`,
`hawier_health_delay`. The image tag equals the stamp on `/settings` by
construction, so `<engine> image ls` on the host reads as a deploy journal (the
last `hawier_keep_images` are kept), and a rollback is
`make rollback-<host> VERSION_OVERRIDE=v25.09.20-abc123def` (the image is already
there: no build, no transfer).

The engine is chosen on each side separately (podman if present, otherwise
docker), and the sides may differ. `ENGINE=docker make deploy-<host>` or
`hawier_engine` in the inventory — if the machine has both and you need the
second. The deploy always names its own images
`localhost/hawier:<version>-{server,ui}` — a name both engines read the same.

### TLS

`hawier_tls: true` in the inventory puts **Caddy** in front of the stack: it
obtains a Let's Encrypt certificate via ACME and renews it itself. The node and
the console move to loopback. A single name (`hawier_console_host` =
`hawier_ddi_host`) is split by paths, two different ones — by two site blocks;
the latter is better, because the node builds `_links` from the `Host` the
device saw. **No DNS records need to be added** if the name already points at
the host: preflight checks this. Certificates live in the `caddy-data` volume —
losing the volume means re-issuing under Let's Encrypt limits: never `down -v`.
Headers, routes and config application — spec [`tls-proxy`](openspec/specs/tls-proxy/spec.md).

What to know before the first production host:

- **Behind a TLS proxy, `X-Forwarded-Proto: https` and a preserved `Host` are
  mandatory.** The server builds all `_links` from them, which the fleet follows
  for artifacts; without them a thousand devices are handed `http://` links. The
  first hop of `X-Forwarded-For` is accepted by the server as the device address
  **without a list of trusted proxies**, so the proxy must overwrite XFF, and
  8080 must not be exposed — hence the default `hawier_publish_server: 127.0.0.1:8080`.
- **A small host must be warned about this.** The defaults assume a machine
  with headroom: 128 MB of shared buffers and 100 connections for postgres plus
  30 pools for the node. On 400 MB of RAM this is the difference between a
  working stack and one the OOM killer takes down. Knobs for the host:
  `hawier_pg_shared_buffers`, `hawier_pg_max_connections`,
  `hawier_mgmt_pool_size`, `hawier_ddi_pool_size` — see the commented `tiny`
  example in the inventory.
- **Rootless podman deserves separate attention.** It does not raise the hard
  descriptor limit above the user's, and the node asks for 65536 (`ulimit -Hn`
  of the deploy user must be at least that); and `restart: unless-stopped`
  covers a crash with it but not a reboot — you need `systemctl --user enable
  podman-restart`. The docker daemon restarts containers by itself.
- **The Postgres port is not published.** On the local test stand `5432:5432` is
  a development convenience; on a host it must not be so.
- **The hawier migration track is one file, edited in place** (see "hawkBit DB compatibility"):
  a version with a modified `0001_hawier.sql` may require manual
  intervention, and `pg_dump` before it is up to the operator.
- Secrets are only as private as the engine is private: whoever can
  `inspect` a container reads its environment. Overrides in the inventory —
  only via `ansible-vault`.
- The deploy user needs passwordless sudo — or the unprivileged mode as a whole:
  `ansible_become: false`, `hawier_become_engine: false` and
  `hawier_owner`/`hawier_group` set to them. `--ask-become-pass` does not work
  through the streaming ssh pipe, and the stream and compose must look at the
  same image store: loading as root and looking as the user is "no image"
  right after a successful load. Preflight checks this.
- `deploy/docker-compose.yml` (test stand) and
  `deploy/ansible/roles/stack/templates/docker-compose.yml.j2` (deploy)
  describe one stack — editing one requires a look at the other.

## Extensions

Everything that can be moved out of the core has been: storage, event
delivery, retention, rollout, console tabs and permissions are **wasm modules**
that hawier loads and calls. There are no in-process implementations, so a
"reference" and a "third-party" implementation of a point are one and the same.
The module has no file system, no sockets and no node secrets: it sees only the
`HAWIER_EXT_*` variables, and the network only to hosts declared in the
manifest, and only through the host's hands.

| Extension | Where | Module | What it decides |
|---|---|---|---|
| Storage (files) | `apps/ext-fs` | `hawier_ext_fs.wasm` | the artifact path in the tree |
| Storage (S3) | `apps/ext-s3` | `hawier_ext_s3.wasm` | presigned URL; holds the keys |
| Event delivery | `apps/ext-webhook` | `hawier_ext_webhook.wasm` | whom to send what to, the signature |
| Retention policy | `apps/ext-retention` | `hawier_ext_retention.wasm` | which artifacts may be deleted |
| Rollout (classic) | `apps/ext-rollout-classic` | `hawier_ext_rollout_classic.wasm` | hawkBit percentages and thresholds |
| Rollout (canary) | `apps/ext-rollout-bucket` | `hawier_ext_rollout_bucket.wasm` | random waves over stable buckets |
| Console tabs | `apps/ext-console` | `hawier_ext_console.wasm` | which nodes and views are in the tree |
| Permissions | `apps/ext-authz` | `hawier_ext_authz.wasm` | whether this identity may perform this action |

```bash
make ext        # build the modules (target wasm32-wasip1)
HAWIER_EXT_STORAGE=$PWD/target/wasm32-wasip1/debug/hawier_ext_fs.wasm \
HAWIER_EXT_FS_ROOT=$PWD/artifacts HAWIER_ARTIFACT_DIR=$PWD/artifacts make run
```

The contracts are the specs [`extension-host`](openspec/specs/extension-host/spec.md) and
[`first-party-extensions`](openspec/specs/first-party-extensions/spec.md);
why it is so and what comes next — [EXTENSIONS.md](EXTENSIONS.md). Without a
storage extension the node serves polling and deployments, but serves artifacts
as `503` — there is deliberately no fallback; and a wrong module path keeps the
node from coming up at all.

## Documents

| File | What it is about |
|---|---|
| [`openspec/specs/`](openspec/specs/) | **what the system does**: the contract of each capability; change via an OpenSpec change (`/opsx:propose`) |
| [ARCHITECTURE.md](ARCHITECTURE.md) | module map, topology, the 10 Gbit path, trade-offs |
| [EXTENSIONS.md](EXTENSIONS.md) | the extension mechanism: why it is so, narrow places, roadmap |
| [SCHEMA_NOTES.md](SCHEMA_NOTES.md) | narrow places of the inherited DB schema and the plan to cut them out |
| [API_NOTES.md](API_NOTES.md) | API backlog: limitations of the inherited API, the `_links` doctrine, hardened |
| `bench/NOTES.md` | how to read the load-run history and what in it not to trust |
