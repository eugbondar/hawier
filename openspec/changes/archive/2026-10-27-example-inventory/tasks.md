# Tasks

## 1. Inventory

- [x] 1.1 Ship one ready `server` entry with a placeholder address and commented examples in `deploy/ansible/inventory.yml`; move private hosts to the ignored `inventory.local.yml`.
- [x] 1.2 make reads both inventory files; the dirty check excludes the tracked inventory.
- [x] 1.3 Preflight refuses documentation-placeholder addresses before contacting the host.
- [x] 1.4 Ignore `*.log.ansible` and remove the tracked deploy log.
- [x] 1.5 README "Deployment" starts with the one-edit, one-command install.

## 2. Verify

- [x] 2.1 `make deploy-list` lists hosts from both files; `make deploy-server` with the placeholder fails in preflight with the sentence, before any build; an edited inventory passes the dirty check.
- [x] 2.2 `make lint`, `openspec validate --strict`.
