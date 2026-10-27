# Spec Delta

## MODIFIED Requirements

### Requirement: Make is the only control surface
The root Makefile SHALL provide these targets:
- checks and builds: `all`, `fmt`, `lint`, `test`, `build`, `ext`, `ext-release`
- local runs: `run`, `run-ui`, `device`
- measurement: `load`, `load-poll`
- integration tests: `it`, `it-keep`, `ui-audit`
- device end-to-end: `e2e-image`, `e2e`
- documentation: `screenshots`
- cache: `gc`, `gc-dry`
- console state: `reset-ui-state`
- images and stand: `docker`, `up`, `scale`, `down`, `logs`, `ps`, `clean`
- deployment: `deploy-list`, `deploy-<host>`, `logs-<host>`, `creds-<host>`, `rollback-<host>`

`lint` SHALL run `cargo fmt --check` and clippy with `-D warnings` over the workspace. It SHALL also run clippy explicitly over both integration-test targets. It SHALL also run `shellcheck` over the end-to-end harness scripts and the screenshot harness script. It SHALL also check that relative links and images in `README.md` and `docs/**` resolve (spec `repository-docs`). `it` SHALL bring up PostgreSQL from the stand compose file, wait for it to be ready, run both integration targets, and stop it.

`all` and `build` SHALL NOT depend on `e2e-image` or `screenshots`: the device image and the screenshots are built only when asked for.

#### Scenario: Lint covers integration tests
- **WHEN** a clippy warning is introduced in `apps/server/tests/it/`
- **THEN** `make lint` fails

#### Scenario: Integration run
- **WHEN** `make it` is run on a machine with a container engine
- **THEN** PostgreSQL is started, both integration suites run, and PostgreSQL is stopped afterwards

#### Scenario: Plain build does not touch Buildroot
- **WHEN** `make build` runs on a fresh checkout
- **THEN** nothing is written under `target/e2e/`

#### Scenario: Plain build does not take screenshots
- **WHEN** `make all` runs on a fresh checkout
- **THEN** no browser image is pulled and nothing under `docs/assets/screenshots/` changes

#### Scenario: Dead link in the README
- **WHEN** `README.md` links to `docs/dev/nope.md`, which does not exist
- **THEN** `make lint` fails and names the file, the line and the target

## ADDED Requirements

### Requirement: Console screenshots
`make screenshots` SHALL write PNG screenshots of the console into
`docs/assets/screenshots/`, one file per page from a fixed list kept in one
place in the harness. The list SHALL include at least the overview page, the
target list, one target's page with its action history, and the page of a
rollout that is in progress.

The target SHALL run against a stand of its own: a dev node and a dev console
on loopback ports distinct from the compose stand and from the end-to-end
harness, over a database of its own created in the stand's PostgreSQL and
dropped afterwards. It SHALL seed that stand with demo data through the Mgmt
API and the device simulator only, so that the captured lists are not empty
and one rollout is in progress. It SHALL sign in to the console with the
stand's credentials, capture every listed page at one fixed window size, and
on exit — success, failure or interrupt — stop the node and the console it
started and drop its database.

The browser SHALL run in a container started through the discovered engine
from an image named by its full registry name and an exact version. The
container SHALL reach only the loopback stand.

The target SHALL fail, naming the page, when a page answers with an error
status, when a page shows a console render error, or when a written file is
larger than 600 KB; it SHALL then leave the previous files in
`docs/assets/screenshots/` unchanged. Every wait in the harness (database,
node, console, rollout progress) SHALL give up after a bounded time and say
what it was waiting for.

#### Scenario: Refresh after a console change
- **WHEN** `make screenshots` runs on a machine with a container engine and a
  free compose PostgreSQL
- **THEN** every listed page is written to `docs/assets/screenshots/`, and
  afterwards no node, console or browser container from the run is left and
  its database is gone

#### Scenario: A page fails to render
- **WHEN** one of the listed pages answers 500
- **THEN** `make screenshots` fails naming that page, and the files in
  `docs/assets/screenshots/` are the ones from the previous run

#### Scenario: The compose stand is running
- **WHEN** `make up` has the compose stand on its usual ports
- **THEN** `make screenshots` runs beside it without touching the stand's
  database or its data

#### Scenario: Node never comes up
- **WHEN** the harness node exits at startup
- **THEN** the target fails after its bounded wait, saying it was waiting for
  the node, and prints the tail of the node's log
