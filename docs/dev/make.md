# make targets

Make is the only control surface (spec
[`build-tooling`](../../openspec/specs/build-tooling/spec.md)). You need
`cargo`, a container engine — **podman or docker**, whichever is found,
podman first; `ENGINE=docker make ...` overrides — and `shellcheck` for
`make lint`.

## Development loop

```bash
make up          # postgres + hawier-control + hawier-ui (compose)
```

Without containers, except Postgres:

```bash
make it-keep     # bring up postgres and run the integration tests
make run         # server locally (control node) on :8080
make run-ui      # console locally on :8088
```

Before a commit: `make lint`, and `make ui-audit` when `apps/ui` or
`static/assets` changed.

## All targets

| Target | Action |
|---|---|
| `make all` | check-tools + lint + test + build |
| `make check-tools` | whether `cargo`, a container engine and `shellcheck` are present |
| `make fmt` | `cargo fmt --all` |
| `make lint` | English-only check, `cargo fmt --check`, `clippy -D warnings` (including both `tests/it` targets), `shellcheck` over the e2e and screenshot harnesses, relative links in `README.md` and `docs/` |
| `make test` | unit tests (no DB) |
| `make it` / `make it-keep` | integration tests against a real Postgres (compose); `-keep` leaves Postgres running |
| `make ui-audit` | console UI/UX audit only (quick run) |
| `make reset-ui-state` | recreate the `hawier_*` tables after editing `0001_hawier.sql` |
| `make run` / `make run-ui` | server / console locally |
| `make device ARGS="..."` | device simulator ([simulator.md](simulator.md)) |
| `make ext` / `make ext-release` | build the wasm extension modules (debug / release) |
| `make load` / `make load-poll` | update / polling load run ([load-testing.md](load-testing.md)) |
| `make e2e-image` | reference device image (Buildroot, SquashFS A/B, signed `.swu`) in `target/e2e/out` |
| `make e2e` / `E2E_ONLY=<scenario> make e2e` | hawier against SWUpdate/suricatta in QEMU ([e2e.md](e2e.md)) |
| `make screenshots` | regenerate the README screenshots in `docs/assets/screenshots/` (see below) |
| `make gc` / `make gc-dry` | remove unreachable artifacts from `target/debug` / show what would go |
| `make build` / `make docker` | release build / images (with whichever engine is found) |
| `make up` / `make scale` / `make down` / `make logs` / `make ps` | compose stand; `scale` adds a sub node on `:8081` |
| `make clean` | `cargo clean` + remove images — also wipes `target/release/hawier-device` and the wasm modules; you usually want `make gc` |
| `CARGO_PROFILE=release make scale` | the stand with an optimized build — for measurements and anything that leaves the machine |
| `make deploy-<host>` / `make deploy-list` | deploy to a host from `deploy/ansible/inventory.yml` / list known hosts ([deployment.md](deployment.md)) |
| `make logs-<host>` | container states, restarts and the log tail of each service on the host |
| `make creds-<host>` | the console address and how to sign in |
| `make rollback-<host> VERSION_OVERRIDE=v…` | bring up a version whose image is already on the host |

## Images and the network

`make docker`, `make e2e-image` and `make screenshots` are the only targets
that reach the network, and only from this machine. If your engine's client
configuration sends builds through a proxy on loopback, a build container
reaches it only with host networking, passed in `BUILD_ARGS`. An assignment
on the command line replaces the default, which carries the build stamp —
repeat its three arguments, or `/settings` shows the wrong version:

```bash
make docker BUILD_ARGS="--network=host --build-arg GIT_SHA=$(git rev-parse --short=9 HEAD) --build-arg BUILD_DATE=$(git log -1 --format=%cd --date=format:%y.%m.%d) --build-arg CARGO_PROFILE=dev"
```

## Screenshots

`make screenshots` brings up a node and a console of its own on
`127.0.0.1:18180` / `:18188` over a database `hawier_shots`, seeds demo data
through Mgmt and the simulator, and captures the pages listed in
`scripts/screenshots.sh` with a pinned Playwright image. It runs beside
`make up`. The first run pulls the image and installs the pinned Python
packages into `target/screenshots-py`; later runs need no network. The
committed pictures change only when every page rendered; on failure the
logs are in `target/screenshots/logs`. Rerun it after a visible console
change and commit the result — the pictures are never edited by hand.
