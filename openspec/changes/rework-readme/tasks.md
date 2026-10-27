# Tasks

## 1. Screenshot harness (commit 1)

- [x] 1.1 Pick an exact `mcr.microsoft.com/playwright/python:v<x.y.z>-noble` tag at least two weeks old and pull it with the discovered engine; verify `$(ENGINE) image ls` lists it by that full name
- [x] 1.2 Write `scripts/screenshots.py`: sign in through `/login` (username, password, tenant), visit each `name path` argument at 1440×900 / scale 1, fail on non-2xx or `template render failed`, write `<name>.png` to the mounted output dir; verify by running it against the compose stand (`make up`) for `/` only
- [x] 1.3 Write `scripts/screenshots.sh` stand setup and teardown: compose PostgreSQL, database `hawier_shots`, node on `127.0.0.1:18180` with `ext-fs` + `ext-rollout-classic`, console on `127.0.0.1:18188`, bounded waits that also detect a dead child and print its log tail, `trap` that kills both, drops the database and stops PostgreSQL only if it started it; verify with `kill` of the node mid-wait that the run fails naming the node and leaves no process or database behind (`pgrep hawier`, `psql -l`)
- [x] 1.4 Add seeding to `scripts/screenshots.sh` through Mgmt and `hawier-device` only: configs, module + artifact, DS, types, tags, ~40 simulators plus a few `--fail-download`, one finished rollout, one in-progress rollout whose group includes never-polling targets; verify by opening the harness console in the browser pane before capture that lists are filled and the second rollout stays RUNNING
- [x] 1.5 Add the page list (overview, target list, a target with action history, the in-progress rollout, a distribution set) as one array, run the capture container (`--network=host`, `--user`, script read-only, output under `target/screenshots/`), enforce the 600 KB cap and promote into `docs/assets/screenshots/` only when every page succeeded; verify that forcing one path to a 404 fails the run and leaves the committed PNGs unchanged
- [x] 1.6 Add the `screenshots` target to the root `Makefile` (depends on `ext`, builds server/ui/client, not a prerequisite of `all`/`build`) and add `scripts/screenshots.sh` to the `shellcheck` line of `lint`; verify `make lint` passes and `make -n all` does not mention `screenshots`
- [x] 1.7 Run `make screenshots` with the compose stand up beside it; open each PNG and check it shows filled lists, no stray credentials and a sensible crop; verify the stand's own data is untouched and no node/console/container from the run remains
- [ ] 1.8 `make lint`, then commit the harness and the first PNG set as one slice (stage only these files) with a message saying what the target does and why the pictures are generated

## 2. Logo

- [ ] 2.1 Draw `docs/assets/logo-light.svg` and `docs/assets/logo-dark.svg` per design §3 (square mark with chevron, rectilinear wordmark, palette colours only, no `<text>`, under 4 KB each); verify by opening both in the browser pane on a white and on a `#0d1117` background and checking legibility at 120 px and 32 px height

## 3. docs/dev pages

- [ ] 3.1 Create `docs/dev/README.md` (index + repository layout from the old Components table) and `docs/dev/make.md` (the make-target table, the proxy note for image pulls/builds); verify every target in spec `build-tooling` appears in `make.md`
- [ ] 3.2 Create `docs/dev/configuration.md` (server and console environment variables, the extension parameters file rules, links to `extension-host`, `tenant-configuration`, `incidents`), replacing archived-design item references with plain statements; verify every `HAWIER_*` variable from the old README is present
- [ ] 3.3 Create `docs/dev/simulator.md`, `docs/dev/load-testing.md` and `docs/dev/e2e.md` (no budget table, no type-compatibility note, link to `reference-device-image` for limits); verify every load-run variable and e2e scenario from the old README is present
- [ ] 3.4 Create `docs/dev/deployment.md` (inventory, preflight, what the deploy does, secrets, diagnostics, rollback, engines, TLS, the pre-production checklist, scaling demo) and `docs/dev/extensions.md` (module table, run example), `docs/dev/hawkbit-compatibility.md` (Flyway history, baseline, `hawier_*` tables); verify by diffing the old README section by section against design §1 that nothing kept is missing

## 4. README and references (commit 2)

- [ ] 4.1 Rewrite `README.md` per design §2 and spec `repository-docs`: `<picture>` logo header, one-liner, hero + table of screenshots, what it does, quick start, two-step deploy, migrating from hawkBit, documentation links; no reference tables; only SWUpdate named as verified; verify it is under 120 lines and has no make/env table (`grep -c '^| `' README.md` near zero)
- [ ] 4.2 Write `scripts/doc_links.sh` per design §5 and call it from `make lint`; verify it passes on the new tree and fails naming file, line and target when a link in `README.md` is pointed at `docs/dev/nope.md`
- [ ] 4.3 Repoint references per design §6 (`CLAUDE.md`, `apps/server/src/db/flyway.rs`, `EXTENSIONS.md`, `SCHEMA_NOTES.md`, `bench/NOTES.md`, `deploy/ansible/inventory.yml`); verify `git grep -n 'README' -- ':!openspec/changes/archive' ':!docs'` shows no pointer at a removed README section
- [ ] 4.4 Preview the README rendering locally (a scratch HTML page with the same `<picture>` and images, light and dark) in the browser pane; verify the logo switches with the colour scheme and screenshots appear before the first command block
- [ ] 4.5 `make lint` and `openspec validate rework-readme --strict`, then commit logo, `docs/dev/`, README, link check and repointed references as one slice with a message explaining the split between landing page and `docs/dev/`
