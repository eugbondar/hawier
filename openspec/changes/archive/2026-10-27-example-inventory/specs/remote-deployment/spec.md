# Spec Delta

## MODIFIED Requirements

### Requirement: Deploy only through make and ansible
A host SHALL be described once, with address, ssh user and `hawier_node_id`, either in the tracked `deploy/ansible/inventory.yml` or in `deploy/ansible/inventory.local.yml`, which git ignores and make reads in addition when it exists. The tracked inventory SHALL ship one ready entry, `server`, whose address is a documentation placeholder, so the simplest install is setting that address and running `make deploy-server`. `make deploy-<host>` SHALL be the only supported way to ship. It SHALL:
1. Refuse an unknown host, listing the known ones.
2. Refuse a dirty working tree unless `ALLOW_DIRTY=1`, because the version would not describe what is shipped. Changes to the tracked inventory SHALL NOT count: it is not part of the images.
3. Run the preflight.
4. Build release images tagged `localhost/hawier:<version>-{server,ui}`, where version is `vYY.MM.DD-<sha>`.
5. Run the play.

The play SHALL refuse a version that was not produced by make. `make deploy-list` SHALL list the hosts from both inventory files. Deploy logs SHALL NOT be tracked by git.

#### Scenario: Dirty tree
- **WHEN** `make deploy-prod` runs with uncommitted changes
- **THEN** nothing is built and no host is contacted

#### Scenario: Only the inventory was edited
- **WHEN** the only uncommitted change is a new `ansible_host` in `deploy/ansible/inventory.yml`
- **THEN** the dirty check passes and the deploy proceeds to preflight

#### Scenario: Private host
- **WHEN** a host is described only in `deploy/ansible/inventory.local.yml`
- **THEN** `make deploy-list` lists it and `make deploy-<host>` deploys it

### Requirement: Preflight before build
Every check that can refuse a host SHALL run before any image is built, and SHALL fail with a sentence about the host rather than a traceback. The checks SHALL be:
- that the host's address is not a documentation placeholder (RFC 2606 `example.com`, `example.net`, `example.org` and their subdomains; RFC 5737 networks), checked before contacting the host, naming the inventory file to edit
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

#### Scenario: Unedited example host
- **WHEN** `make deploy-server` runs while `server` still points at `hawier.example.com`
- **THEN** preflight fails naming `deploy/ansible/inventory.yml`, no ssh connection is made and no build has started
