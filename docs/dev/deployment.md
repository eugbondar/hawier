# Deployment

The full behaviour, with failure conditions, is in the specs
[`remote-deployment`](../../openspec/specs/remote-deployment/spec.md),
[`build-tooling`](../../openspec/specs/build-tooling/spec.md) and
[`tls-proxy`](../../openspec/specs/tls-proxy/spec.md).

## The simplest install

1. In [`deploy/ansible/inventory.yml`](../../deploy/ansible/inventory.yml),
   set `ansible_host` of the `server` entry to your machine (and
   `ansible_user`, if you do not log in as root).
2. `make deploy-server`

When it finishes, the console is at `http://<your host>:8088`, devices talk
to `http://<your host>:8080`, and `make creds-server` prints the login (now
or any time later). Before real devices, turn on [TLS](#tls).

## Requirements

**This** machine needs `ansible-core` (`pipx install ansible-core`) and
internet access for the build. The target needs ssh and three things, which
the role does not install — it names what is missing and stops, in about a
second, before any build:

| On the host | Why |
|---|---|
| `python3` | ansible itself runs on it |
| podman or docker **with compose** | `podman compose` exists since podman 4.9, otherwise `podman-compose` is needed; with docker it is the v2 plugin — compose v1 will not read the file |
| ~4 GB of free space for images | `hawier_min_free_mb` |

The target machine needs no internet at all.

## Hosts

A host is described once in the inventory — address, ssh port, user and
node number; the file has commented examples for TLS, a small machine and a
host behind your own proxy. Hosts you would rather keep out of git go to
`deploy/ansible/inventory.local.yml` (ignored, same format); make reads
both. Editing the inventory does not make the tree dirty for a deploy: it is
not part of the images.

```bash
make deploy-list          # which hosts are described
make deploy-<host>        # build, ship, bring up, verify
```

## What a deploy does

Preflight rejects an unfit host before the build. Release images are built
(`CARGO_PROFILE=release` is forced). Only what the host lacks is streamed to
it via `save | zstd | ssh | load` — exact image names are compared, since
ids differ between podman and docker — including the base `postgres`: **the
target machine downloads nothing**. A self-contained compose file is
rendered; the play waits for health and recognizes a crash loop; the deploy
ends by checking `node.build`, `node.profile` and `node.role` on the node
and the build on the console.

The host gets `/opt/hawier` with `docker-compose.yml` (values written as
literals, file 0600, no `.env`) and `secrets/`.

## Secrets

The Postgres password, `HAWIER_INTERNAL_TOKEN` and `HAWIER_MGMT_USERS` are
generated **once**, on the first deploy, and only read afterwards. The
credentials are printed on the run where they were born; later, ask with
`make creds-<host>`. To change the console password:

```bash
make deploy-<host> ANSIBLE_ARGS='-e hawier_mgmt_users=admin:NEW:DEFAULT'
```

This way it ends up neither in the inventory nor in git. The Postgres
password cannot be changed this way: `POSTGRES_PASSWORD` takes effect only
when the volume is initialized, so the role refuses and suggests
`ALTER USER`.

## When it fails

The cause is printed by make as plain text that can be read and forwarded:
immediately when a deploy fails, and at any time with `make logs-<host>` —
no deploy, no version, not a single command typed on the host
(`DIAG_LINES=300` for more lines). The report is also saved to
`hawier-<host>.log`. Everything in it is asked of the container engine, not
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

Wait timings: `hawier_compose_timeout`, `hawier_health_retries`,
`hawier_health_delay`.

## Versions and rollback

The image tag equals the build stamp on `/settings` by construction, so
`<engine> image ls` on the host reads as a deploy journal (the last
`hawier_keep_images` are kept). A rollback brings up a version whose image is
already there — no build, no transfer:

```bash
make rollback-<host> VERSION_OVERRIDE=v25.09.20-abc123def
```

## Engines

The engine is chosen on each side separately (podman if present, otherwise
docker), and the sides may differ. `ENGINE=docker make deploy-<host>` or
`hawier_engine` in the inventory picks the second one on a machine that has
both. The deploy always names its images `localhost/hawier:<version>-{server,ui}`,
a name both engines read the same way.

## TLS

`hawier_tls: true` in the inventory puts **Caddy** in front of the stack: it
obtains a Let's Encrypt certificate via ACME and renews it itself. The node
and the console move to loopback. A single name (`hawier_console_host` =
`hawier_ddi_host`) is split by paths, two different names by two site
blocks; the latter is better, because the node builds `_links` from the
`Host` the device saw. **No DNS records need to be added** if the name
already points at the host: preflight checks this. Certificates live in the
`caddy-data` volume — losing it means re-issuing under Let's Encrypt
limits: never `down -v`.

## Before the first production host

- **Behind a TLS proxy, `X-Forwarded-Proto: https` and a preserved `Host`
  are mandatory.** The server builds all `_links` from them, and the fleet
  follows those links for artifacts; without them a thousand devices are
  handed `http://` links. The first hop of `X-Forwarded-For` is taken as the
  device address **without a list of trusted proxies**, so the proxy must
  overwrite XFF and 8080 must not be exposed — hence the default
  `hawier_publish_server: 127.0.0.1:8080`.
- **A small host needs smaller numbers.** The defaults assume headroom: 128
  MB of shared buffers and 100 connections for Postgres plus 30 pool
  connections for the node. On 400 MB of RAM that is the difference between
  a working stack and one the OOM killer takes down. Knobs:
  `hawier_pg_shared_buffers`, `hawier_pg_max_connections`,
  `hawier_mgmt_pool_size`, `hawier_ddi_pool_size` — see the commented
  `tiny` example in the inventory.
- **Rootless podman deserves separate attention.** It does not raise the
  hard descriptor limit above the user's, and the node asks for 65536
  (`ulimit -Hn` of the deploy user must be at least that); and
  `restart: unless-stopped` covers a crash but not a reboot — run
  `systemctl --user enable podman-restart`. The docker daemon restarts
  containers by itself.
- **The Postgres port is not published.** On the local stand `5432:5432` is
  a development convenience; on a host it must not be.
- **The hawier migration track is one file, edited in place**
  ([hawkbit-compatibility.md](hawkbit-compatibility.md)): a version with a
  modified `0001_hawier.sql` may need manual intervention, and a `pg_dump`
  before it is up to the operator.
- Secrets are only as private as the engine: whoever can `inspect` a
  container reads its environment. Overrides in the inventory — only via
  `ansible-vault`.
- The deploy user needs passwordless sudo — or the unprivileged mode as a
  whole: `ansible_become: false`, `hawier_become_engine: false` and
  `hawier_owner`/`hawier_group` set to that user. `--ask-become-pass` does
  not work through the streaming ssh pipe, and the stream and compose must
  look at the same image store: loading as root and looking as the user is
  "no such image" right after a successful load. Preflight checks this.
- `deploy/docker-compose.yml` (the local stand) and
  `deploy/ansible/roles/stack/templates/docker-compose.yml.j2` (the deploy)
  describe one stack — editing one means looking at the other.

## Scaling

One control node (Mgmt and the rollout engine) plus N sub nodes (DDI and
artifact serving). If the control node goes down, devices keep updating
through the sub nodes. Locally:

```bash
make scale                                                       # + sub node on :8081
podman compose -f deploy/docker-compose.yml stop hawier-control  # or docker compose
hawier-device --url http://localhost:8081 --count 50 --controller-id fleet --gateway-token ...
```

Node roles and what survives the death of the control node: spec
[`node-roles`](../../openspec/specs/node-roles/spec.md). The 10 Gbit
strategy (streaming, pool isolation, S3 presigned redirect) is in
[ARCHITECTURE.md](../../ARCHITECTURE.md).
