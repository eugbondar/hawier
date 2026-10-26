# Spec Delta

## Purpose

Shipping the stack to a host listed in `deploy/ansible/inventory.yml` with
`make deploy-<host>`. The host is refused early, images are streamed over ssh,
a self-contained compose file is rendered, and secrets are generated once. The
deploy waits for health while detecting crash loops, reports failures as plain
text, verifies what arrived, and supports rollback and credential lookup.

## ADDED Requirements

### Requirement: Deploy only through make and ansible
A host SHALL be described once in the inventory, with address, ssh user and `hawier_node_id`. `make deploy-<host>` SHALL be the only supported way to ship. It SHALL:
1. Refuse an unknown host, listing the known ones.
2. Refuse a dirty working tree unless `ALLOW_DIRTY=1`, because the version would not describe what is shipped.
3. Run the preflight.
4. Build release images tagged `localhost/hawier:<version>-{server,ui}`, where version is `vYY.MM.DD-<sha>`.
5. Run the play.

The play SHALL refuse a version that was not produced by make. `make deploy-list` SHALL list the hosts.

#### Scenario: Dirty tree
- **WHEN** `make deploy-prod` runs with uncommitted changes
- **THEN** nothing is built and no host is contacted

### Requirement: Preflight before build
Every check that can refuse a host SHALL run before any image is built, and SHALL fail with a sentence about the host rather than a traceback. The checks SHALL be:
- a Python interpreter, checked without needing anything on the host
- a container engine (podman, then docker) that answers as the play user
- compose v2 (`<engine> compose` or `<engine>-compose`), with an engine-specific hint
- at least `hawier_min_free_mb` (4096) free in the image store
- that images will be loaded into the same store compose will read
- with TLS enabled, that every public name resolves to an address of the host

The role SHALL install nothing and SHALL gather no facts.

#### Scenario: No compose
- **WHEN** the host's engine has no compose v2
- **THEN** preflight fails naming what to install, and no build has started

#### Scenario: Name points elsewhere
- **WHEN** TLS is on and the console name resolves to another machine
- **THEN** preflight fails naming the name, its address and the host's addresses

### Requirement: The target downloads nothing
Every image the host runs SHALL arrive from the deploy machine. Base images such as postgres and caddy SHALL be pulled on the deploy machine. Images SHALL be streamed as `save | zstd (or gzip) | ssh | load`, with `pipefail`, without a registry and without temporary files. Only references the host lacks SHALL be sent. After loading, every reference compose needs SHALL be present, or the deploy SHALL fail before compose runs. Every service in the deployed compose file SHALL have `pull_policy: never`.

#### Scenario: Redeploy of the same version
- **WHEN** the same version is deployed twice
- **THEN** the second run sends no image data

#### Scenario: Stream broken mid-transfer
- **WHEN** `save` dies during the transfer
- **THEN** the task fails instead of reporting success

### Requirement: Self-contained compose file
The deployed compose file SHALL contain literal values with no `${...}` interpolation, and SHALL have no `.env` beside it. It SHALL be written with mode 0600 and without logging its content. The project name SHALL be pinned, so that volume names are stable. PostgreSQL SHALL NOT be published by default. The node SHALL be published on loopback by default.

#### Scenario: Password cannot arrive empty
- **WHEN** the stack is deployed with podman-compose
- **THEN** the PostgreSQL password is a literal in the file and the database initialises with it

### Requirement: Secrets generated once
The PostgreSQL password, the internal token and the console users SHALL be generated on the host on the first deploy, as 32 alphanumeric characters each. They SHALL be stored in `<dir>/secrets/` with mode 0600 and never regenerated. `hawier_mgmt_users` and `hawier_internal_token` given to the play SHALL replace the stored values. A given `hawier_postgres_password` that differs from the stored one SHALL fail the deploy with instructions to change it in the database. Credentials SHALL be printed on the run that created them, and `make creds-<host>` SHALL print them later.

#### Scenario: Second deploy
- **WHEN** a host is deployed a second time
- **THEN** no secret file changes

#### Scenario: New console password
- **WHEN** `make deploy-prod ANSIBLE_ARGS='-e hawier_mgmt_users=admin:NEW:DEFAULT'` is run
- **THEN** the console accepts the new password after the deploy

### Requirement: Bounded waits that detect crash loops
Every compose call SHALL be wrapped in a timeout (`hawier_compose_timeout`, 600 s). Compose SHALL only be asked to bring the stack up. Container state SHALL be read from the engine.

Health SHALL be awaited by the play. Each attempt SHALL check both readiness and looping in the same loop:
- ready means `/health` answers `ok` inside the container
- looping means the container is restarting, exited or gone, or its restart count grew

A looping container SHALL fail the deploy at once. When no container existed before the deploy, a failed deploy SHALL stop the stack. A previously running stack SHALL be left alone.

#### Scenario: Crash on start
- **WHEN** the control node exits with code 1 on startup
- **THEN** the deploy fails on the first attempt that sees it, not after all retries

#### Scenario: Failed first bring-up
- **WHEN** a host's first deploy fails
- **THEN** the stack is stopped so the host is not left in a restart loop

### Requirement: Plain-text diagnostics
On a failed deploy, and on `make logs-<host>` at any time, make SHALL print a plain-text report and save it to `hawier-<host>.log`. The report SHALL be built only from engine queries. It SHALL contain:
- host, time, engine and project
- every container with its status and image
- per service: state, exit code, restart count and health
- the last `DIAG_LINES` log lines of each service

#### Scenario: Read without deploying
- **WHEN** `make logs-prod` is run
- **THEN** the report is printed without building, without a version and without running commands by hand on the host

### Requirement: Verification of what arrived
A deploy SHALL succeed only if all of these hold:
- the node reports `node.build` equal to the shipped version, `node.profile` = `release`, `node.role` = `control`, and a writable parameters file
- the console's `/api/health` reports the same build
- with TLS, the public HTTPS endpoints answer with valid certificates

After success, older image versions beyond `hawier_keep_images` (3) SHALL be pruned, never the current one.

#### Scenario: Console not recreated
- **WHEN** the console container still runs the previous build after the deploy
- **THEN** the deploy fails saying the console was not recreated

### Requirement: Rollback
`make rollback-<host> VERSION_OVERRIDE=<version>` SHALL re-render and bring up the stack on an image already present on the host, with no build and no transfer. It SHALL then verify that version.

#### Scenario: Roll back
- **WHEN** a host still holds the image of the previous version and rollback names it
- **THEN** the stack runs that version and verification expects it

### Requirement: Stand and deploy describe one stack
The local stand compose file and the deploy template SHALL describe the same services, extension configuration and volumes. They are allowed to differ only in these ways:
- the stand builds from source, and the template uses shipped images
- the stand has fixed development credentials, and the template has generated secrets
- the stand has a sub-node `scale` profile on port 8081
- the template has the optional TLS proxy
- the template has health checks

The extension parameters volume SHALL be written only by the node, never by ansible.

#### Scenario: Scale profile
- **WHEN** `make scale` is run
- **THEN** a sub node serves DDI on port 8081 from the same image as the control node
