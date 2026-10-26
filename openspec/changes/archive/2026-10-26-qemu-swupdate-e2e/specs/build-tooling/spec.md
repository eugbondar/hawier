# Spec Delta

## MODIFIED Requirements

### Requirement: Make is the only control surface
The root Makefile SHALL provide these targets:
- checks and builds: `all`, `fmt`, `lint`, `test`, `build`, `ext`, `ext-release`
- local runs: `run`, `run-ui`, `device`
- measurement: `load`, `load-poll`
- integration tests: `it`, `it-keep`, `ui-audit`
- device end-to-end: `e2e-image`, `e2e`
- cache: `gc`, `gc-dry`
- console state: `reset-ui-state`
- images and stand: `docker`, `up`, `scale`, `down`, `logs`, `ps`, `clean`
- deployment: `deploy-list`, `deploy-<host>`, `logs-<host>`, `creds-<host>`, `rollback-<host>`

`lint` SHALL run `cargo fmt --check` and clippy with `-D warnings` over the workspace. It SHALL also run clippy explicitly over both integration-test targets. It SHALL also run `shellcheck` over the end-to-end harness scripts. `it` SHALL bring up PostgreSQL from the stand compose file, wait for it to be ready, run both integration targets, and stop it.

`all` and `build` SHALL NOT depend on `e2e-image`: the device image is built only when asked for.

#### Scenario: Lint covers integration tests
- **WHEN** a clippy warning is introduced in `apps/server/tests/it/`
- **THEN** `make lint` fails

#### Scenario: Integration run
- **WHEN** `make it` is run on a machine with a container engine
- **THEN** PostgreSQL is started, both integration suites run, and PostgreSQL is stopped afterwards

#### Scenario: Plain build does not touch Buildroot
- **WHEN** `make build` runs on a fresh checkout
- **THEN** nothing is written under `target/e2e/`
