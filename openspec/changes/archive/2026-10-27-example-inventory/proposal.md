# Proposal

## Why

The repository is going public, and its inventory named the maintainer's own
machines: a LAN address and a public host, with a deploy log of that host
committed next to them. A public repository must not carry them, and a new
user should not have to learn the inventory format to try the system: the
simplest install has to be "set your server's address in one place, run one
make target".

Two things stood in the way. The inventory held only real hosts, so a new user
had to write an entry from scratch. And `make deploy-<host>` refuses a dirty
working tree, so editing the tracked inventory — the one edit the user is
asked to make — would have refused the deploy.

## What Changes

- The tracked inventory ships one ready entry, `server`, pointing at a
  placeholder (`hawier.example.com`), plus commented examples (TLS, a small
  machine, behind a proxy). The user sets `ansible_host` and runs
  `make deploy-server`.
- Preflight refuses a host whose address is a documentation placeholder
  (RFC 2606 `example.*` names, RFC 5737 networks), with a sentence naming the
  file to edit, before contacting the host.
- Private hosts go to `deploy/ansible/inventory.local.yml`, ignored by git and
  read by make in addition to the tracked inventory.
- The dirty-tree check ignores the tracked inventory: it is not part of the
  images (`.dockerignore` drops `deploy/`), so editing it does not make the
  version describe something else.
- Deploy logs (`*.log.ansible`) are ignored by git; the committed one is
  removed.

## Capabilities

### Modified Capabilities
- `remote-deployment`: where hosts are described, what the dirty check covers,
  and a preflight refusal for placeholder hosts.

## Impact

- `deploy/ansible/inventory.yml`, `deploy/ansible/roles/preflight/tasks/main.yml`,
  `Makefile` (deploy section), `.gitignore`, README "Deployment".
- No change to what is built or shipped.
