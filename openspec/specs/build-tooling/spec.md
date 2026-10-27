# build-tooling Specification

## Purpose

The developer and operator interface to the repository. Make is the single entry
point. Containers are built with whichever engine is present. Build profiles
are explicit, the toolchain is pinned, and the build cache is collected
automatically.

## Requirements

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

### Requirement: Container engine is discovered, never hardcoded
Every engine invocation, in both Makefiles and in the deploy role, SHALL use the engine discovered at run time: podman if present, otherwise docker. `ENGINE=<name>` on the command line and `hawier_engine` in the inventory SHALL override the choice. Only the CLI subset common to both engines SHALL be used. Images from a registry SHALL be referenced by their fully qualified name, such as `docker.io/library/...`, including in `FROM`.

#### Scenario: Both engines installed
- **WHEN** podman and docker are both installed and `ENGINE` is unset
- **THEN** `make docker` builds with podman

#### Scenario: Override
- **WHEN** `ENGINE=docker make up` is run
- **THEN** the stand is brought up with docker compose

### Requirement: Build profiles
Locally built images SHALL default to the `dev` profile. `make deploy-<host>` SHALL always build with `CARGO_PROFILE=release`. `CARGO_PROFILE=release make scale` SHALL build the stand with the release profile. The `dev` profile SHALL keep only line tables of debug information and SHALL keep debug assertions. The `release` profile SHALL use thin LTO and strip debug information.

#### Scenario: Default stand is debug
- **WHEN** `make scale` runs without an override
- **THEN** the running node reports `node.profile: debug`

### Requirement: Pinned toolchain
The Rust toolchain SHALL be pinned to one version in `rust-toolchain.toml`, with rustfmt, clippy and the wasm32-wasip1 target. The Dockerfile's builder image tag SHALL pin the same version. The two pins SHALL be changed together.

#### Scenario: Container and host agree
- **WHEN** the toolchain version in `rust-toolchain.toml` is read
- **THEN** the Dockerfile builder image carries the same version

### Requirement: Build cache garbage collection
`make gc` SHALL delete unreachable cargo artifacts under `target/debug`. It SHALL keep:
- every artifact linked into `target/debug/`
- the newest generation of each unit
- the newest incremental sessions per crate

It SHALL drop units not built for `GC_MAX_AGE` days (default 30). It SHALL never touch `target/release` or `target/wasm32-wasip1`. It SHALL refuse to run while cargo holds the build lock.

`make gc-dry` SHALL report what would be removed and remove nothing. Building targets SHALL first run an automatic check. The check SHALL collect only when `target/debug/deps` holds more than `GC_TRIP_FILES` files (default 6000), or when the filesystem has less than `GC_TRIP_FREE_GB` free (default 20).

#### Scenario: Superseded hash removed
- **WHEN** a unit was rebuilt with a new hash two days after the old one
- **THEN** `make gc` removes the old hash's files, fingerprint and build directory

#### Scenario: Dry run
- **WHEN** `make gc-dry` runs
- **THEN** nothing is deleted and the largest removal candidates are listed

#### Scenario: Concurrent build
- **WHEN** `make gc` runs while cargo is building
- **THEN** it refuses with a message and deletes nothing

### Requirement: Integration tests are one target per crate
Integration tests SHALL live in one target per crate, `apps/server/tests/it/` and `apps/ui/tests/it/`. Each SHALL have a `main.rs` that lists its modules. They SHALL be marked ignored and run only through `make it` or `make ui-audit`.

#### Scenario: Area run
- **WHEN** `cargo test -p hawier-server --test it -- --ignored ddi::` is run against a database
- **THEN** only the DDI integration tests run

### Requirement: The repository is English-only
`make lint` SHALL fail when any text file in the working tree that git tracks or would track (not ignored) contains a character from the Cyrillic block (U+0400–U+04FF), and SHALL print the offending lines. Specs, documentation, code comments and test data are all covered; input that has to be non-ASCII is written with escapes in code. Binary files (images, archives) hold no text and SHALL be skipped: their bytes can match the Cyrillic block by chance.

#### Scenario: Cyrillic in a comment
- **WHEN** a tracked source file gains a comment with Cyrillic text
- **THEN** `make lint` fails and names the file and line

#### Scenario: Clean tree
- **WHEN** no tracked file contains Cyrillic
- **THEN** this check passes and `make lint` continues with its other checks

#### Scenario: A committed screenshot
- **WHEN** a PNG under `docs/assets/` contains a byte sequence that decodes as a Cyrillic character
- **THEN** the check ignores the file

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
- **THEN** the target fails at once rather than at the end of its bounded
  wait, saying the node exited, and prints the tail of the node's log
