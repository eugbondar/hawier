# Design

## Context

`README.md` (489 lines) is the only place where most of the operator
reference is written as prose: make targets, environment variables, simulator
flags, load-run knobs, the e2e harness, deployment, TLS and a production
checklist. Much of it restates specs (`build-tooling`, `load-benchmark`,
`device-e2e`, `remote-deployment`, `tls-proxy`, `extension-host`,
`tenant-configuration`); some of it is stale or has no reader (the GitHub
runner budget table with "now" values, the "type compatibility is now
enforced" note, "design.md `baseline-existing-capabilities` item 20",
"item 8 of the catalogue").

Other files point at README sections by name: `CLAUDE.md` ("Full list —
README"), `apps/server/src/db/flyway.rs` (README "DB compatibility"),
`EXTENSIONS.md:395`, `SCHEMA_NOTES.md:50`, `bench/NOTES.md:237`,
`deploy/ansible/inventory.yml:11,40,73`.

There is no logo anywhere, and the console spec (`console-shell`) says the top
bar shows the word `Hawier` "with no badge or logo". The console is dark-only
(`color-scheme: dark`, palette on `:root` in `static/assets/main.css`).

Product documentation already exists as the in-console manual
(`apps/ui/docs/**`), rendered by the console at `/help`. It is not moved by
this change.

The e2e harness (`e2e/run.sh`) already shows the pattern for a throwaway
stand: compose PostgreSQL, a separate database dropped on exit, a dev node on
a loopback port, `trap cleanup EXIT`, bounded `wait_for`.

## Goals / Non-Goals

**Goals:**
- A README a newcomer reads in a minute and that sells the console with
  pictures.
- Nothing worth keeping is lost: every operator fact of the old README is
  either on a `docs/dev/` page or behind a spec link.
- Screenshots that can be refreshed with one command and never block a
  normal build.

**Non-Goals:**
- Moving `ARCHITECTURE.md`, `EXTENSIONS.md`, `API_NOTES.md`,
  `SCHEMA_NOTES.md` or `bench/NOTES.md` (see Open Questions).
- A logo or favicon in the console; a product documentation folder under
  `docs/` (the in-console manual is that today).
- A license file, a CI job for screenshots, a docs site generator.

## Decisions

### 1. Where each part of the old README goes

| Old README section | Goes to |
|---|---|
| Compatibility blockquote | one sentence in the README "Why hawier"; details stay in `ARCHITECTURE.md` and spec `database-schema` |
| Components table | `docs/dev/README.md` (repository layout) |
| Quick start | stays in the README, trimmed to `make up` + addresses + credentials + one simulator line |
| Make targets table | `docs/dev/make.md` |
| Device simulator | `docs/dev/simulator.md` |
| Load test | `docs/dev/load-testing.md` (links `bench/NOTES.md` and spec `load-benchmark`) |
| Run on a real client (e2e) | `docs/dev/e2e.md`, minus the type-compatibility note (it is spec `deployment-assignment` + `API_NOTES` §9.10) |
| Budget table (GitHub runner) | dropped; `docs/dev/e2e.md` says only that `make e2e-image` refuses an image over its budget and links spec `reference-device-image`, where the limits are |
| Environment variables, console variables, extension parameters file | `docs/dev/configuration.md`; the archived-design item references are replaced by a plain statement of the caveat |
| Incident numbers, assignment semantics, maintenance windows | cut: they are product behaviour, in specs `incidents`, `tenant-configuration`, `deployment-assignment`, `ddi-api` and in the in-console manual; `configuration.md` links the specs |
| hawkBit DB compatibility | a short "Migrating from hawkBit" paragraph in the README (the product's strongest argument); the Flyway details go to `docs/dev/hawkbit-compatibility.md` |
| Scaling | one bullet in the README; the demo and node roles go to `docs/dev/deployment.md` |
| Deployment, TLS, "before the first production host" | README: the two-step deploy; everything else `docs/dev/deployment.md` |
| Extensions | one bullet in the README; the module table and the run example go to `docs/dev/extensions.md` (rationale stays in `EXTENSIONS.md`) |
| Documents table | README "Documentation" section: `docs/dev/`, the in-console manual, `openspec/specs/`, the root notes |

`docs/dev/` is a subfolder, not `docs/` itself, because it is engineering
material, not documentation of the product; the product's docs get a sibling
folder when they are written outside the console. Alternative considered:
folding everything into `ARCHITECTURE.md` and `bench/NOTES.md` — rejected,
those files are about rationale and measurements, not how-to.

### 2. README shape

Header: centred `<picture>` with `<source media="(prefers-color-scheme: dark)">`
for `docs/assets/logo-dark.svg` and `<img src="docs/assets/logo-light.svg"
alt="hawier">`, then the one-liner (positioning: self-hosted OTA update server
for device fleets — one binary plus PostgreSQL, a live console, speaks the
hawkBit API). Then one hero screenshot (the running rollout), then the other
captures in a two-column table. Then: what it does (6–8 bullets), quick start,
deploy, migrating from hawkBit, documentation links. Target length: under 120
lines. Only SWUpdate is named as a verified client (spec requirement); RAUC
and Zephyr may appear only as "speak the same DDI API, not tested here".

### 3. Logo

Hand-written SVG, two files differing only in the wordmark colour. Mark: a
square with square corners holding an upward chevron over a bar ("update
lands"), filled with `--accent-top` (#23557f) and stroked/detailed with
`--link` (#6fb3ea). Wordmark `hawier` drawn as rectilinear paths, not `<text>`:
an `<img>`-embedded SVG cannot rely on any font being present, and blocky
lettering matches the rectangular console. Wordmark colour: `--surface`
(#101418) on light, `--text-bright` (#f2f6fa) on dark. No gradients, no
filters, viewBox-sized so it scales; target under 4 KB each. Alternative
considered: a `<text>` wordmark with a font stack — rejected, it renders
differently per machine.

### 4. `make screenshots`

- `Makefile`: `screenshots: ext` → `cargo build -p hawier-server -p hawier-ui
  -p hawier-client`, then `ENGINE=$(ENGINE) bash scripts/screenshots.sh`.
  Not a prerequisite of anything.
- `scripts/screenshots.sh` (bash, shellcheck'd by `make lint`), modelled on
  `e2e/run.sh`: compose PostgreSQL up, `DROP/CREATE DATABASE hawier_shots`,
  node `target/debug/hawier` on `127.0.0.1:18180` with `ext-fs` storage and
  `ext-rollout-classic`, console `target/debug/hawier-ui` on `127.0.0.1:18188`
  pointed at it with the same internal token. Ports differ from the compose
  stand (8080/8081/8088) and from e2e (18080). `trap cleanup EXIT` kills both
  processes, drops the database and stops PostgreSQL only if this run started
  it. Every wait is a bounded loop that also checks the child process is
  still alive (same rule as the deploy role: "ready" and "died" in one loop);
  on give-up it prints the tail of the node/console log.
- Seeding, through Mgmt and the simulator only: tenant configs (gateway token
  on, short polling), a software module with a small generated artifact, a
  distribution set, target types and tags; a fleet of ~40 simulators with a
  gateway token plus a few with `--fail-download`; one finished rollout over
  part of the fleet; one rollout started over a group that includes targets
  created through Mgmt that never poll, so it stays in progress
  deterministically instead of racing the fleet to FINISHED.
- Capture: a pinned `mcr.microsoft.com/playwright/python:v<exact>-noble`
  image (full registry name for podman's short-name enforcement; the exact
  version is chosen at implementation time and must be at least two weeks
  old), run with `--network=host` so loopback is the stand, `--user` set to
  the caller so the written files are theirs, `scripts/screenshots.py`
  mounted read-only and an output directory under `target/screenshots/`
  mounted read-write. The page list (`name path`) lives in one array in
  `screenshots.sh`, with ids filled in after seeding, and is passed to the
  Python script as arguments. The script signs in through `/login`
  (username/password/tenant fields), visits each path at 1440×900, device
  scale factor 1, fails on a non-2xx status or on the text `template render
  failed`, and writes `<name>.png`.
- Promotion: only after every page succeeded and each file is ≤ 600 KB does
  the harness replace `docs/assets/screenshots/*.png` with the new set; any
  failure leaves the committed pictures untouched.
- Why Playwright in a container and not headless Chromium's CLI
  `--screenshot`: the console needs a session cookie and the CLI cannot sign
  in. Why not a host-installed browser: it is not pinned and not every
  developer machine has one. The pull happens on the developer machine; the
  "target machine downloads nothing" rule is about deploy hosts and is not
  affected.

### 5. Link check

`scripts/doc_links.sh`, called from `make lint`: for `README.md` and
`docs/**/*.md`, extract Markdown link/image targets `](...)` and HTML
`src="..."` / `srcset="..."`, skip `http(s):`, `mailto:` and pure `#…`, strip
`#fragment`, resolve relative to the file's directory, and report
`file:line: target` for each missing one. A ~40-line shell script, not a
crate or a markdown parser: the repository's Markdown is plain, and a false
positive is fixed by writing the link plainly.

### 6. Repointing references

`CLAUDE.md` "Full list — README" → `docs/dev/make.md`; the `flyway.rs` module
comment → `docs/dev/hawkbit-compatibility.md`; `EXTENSIONS.md:395` →
`docs/dev/configuration.md`; `SCHEMA_NOTES.md:50` →
`docs/dev/hawkbit-compatibility.md`; `bench/NOTES.md:237` → spec
`load-benchmark` and `CLAUDE.md`; `deploy/ansible/inventory.yml` comments →
`docs/dev/deployment.md`. Archived changes under `openspec/changes/archive/`
are history and are not edited.

## Risks / Trade-offs

- [Screenshots in git grow the history with every refresh] → per-file cap of
  600 KB, a dark UI compresses well, refresh only when the console visibly
  changed.
- [The Playwright image is large (≈2 GB) and the pull may need the proxy] →
  pulled only by `make screenshots`; a pull failure is reported by the engine
  as is. If the pull needs the local privoxy, it is the same situation as
  `make docker` (see the build-network notes) and is documented in
  `docs/dev/make.md`, not worked around in the script.
- [Captured content varies run to run (ids, times)] → accepted; the pages are
  the same, a rerun produces a fresh but equivalent set.
- [A rollout "in progress" could still finish during capture] → its group
  contains targets that never poll, so it cannot finish.
- [The README drifts back into a reference] → the `repository-docs`
  requirement lists what the README must not contain; reviewers have a rule
  to point at.
- [Link check misses links written in unusual Markdown forms] → acceptable;
  it catches the moved-file case, which is the one this change creates.

## Migration Plan

Two commits on `master`:
1. `make screenshots` harness, its first set of PNGs, and the
   `build-tooling` spec delta's target-list/lint text (shellcheck of the new
   script).
2. Logo, `docs/dev/` pages, the rewritten README, the link check in
   `make lint`, repointed references.

Rollback is `git revert`; nothing at runtime depends on either commit.

## Open Questions

- Whether the other root documents (`ARCHITECTURE.md`, `EXTENSIONS.md`,
  `API_NOTES.md`, `SCHEMA_NOTES.md`, `bench/NOTES.md`) move into `docs/dev/`.
  They are referenced from `CLAUDE.md`, `openspec/config.yaml` and many
  specs, so it is a separate change; this one does not block it.
