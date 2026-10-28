# Tasks

## 1. Engine

- [x] 1.1 In `apps/server/src/rollout.rs`, count DOWNLOADED towards a group's finished count only when the rollout is download-only; verify `make test` and the integration tests below

## 2. Tests

- [x] 2.1 Add an integration test: a soft two-group rollout whose group-1 device defers installation (reports `downloaded`) keeps group 2 SCHEDULED, and advances once the device installs; verify with `cargo test -p hawier-server --test it -- --ignored rollout::`
- [x] 2.2 Add an integration test: a download-only two-group rollout advances when its group-1 device reports `downloaded`; verify as in 2.1

## 3. Manual

- [x] 3.1 Update `apps/ui/docs/deployment/rollout-groups.md` (what counts towards success); verify `make ui-audit`

## 4. Verification and commit

- [x] 4.1 `make lint`, `make test`, `make it` green
- [x] 4.2 `make load` against a release node, compared with `bench/load-history.csv` at the same parameters (no degradation above 20%)
- [x] 4.3 Archive the change (syncing its delta into `openspec/specs/`) and commit the slice on master
