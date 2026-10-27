# hawier — rules for agents

Monorepo: hawier is a rewrite of Eclipse hawkBit in Rust (Mgmt + DDI API,
PostgreSQL-only) + hawier-ui (server-side rendered UI) + hawier-client
(device simulator).

## Behaviour lives in `openspec/specs/`, not here

**What the system does is recorded in the specs** at
`openspec/specs/<capability>/spec.md` — one per capability (DDI, Mgmt, rollout
engine, extensions, console, deployment…). This file is about **how to work**:
process, engineering rules, and hooks into the code that the spec
deliberately lacks (a spec doesn't name functions).

- Changing behaviour — create an OpenSpec change (`/opsx:propose`): the delta
  to the spec goes in the same slice as the code. Behaviour the spec doesn't
  describe is behaviour the next agent is entitled to break.
- Code and spec diverge — first find out which one is right. A defect in a
  SHALL is not written into the spec: it goes into the change's design.md as a
  divergence and is fixed by its own change.
- Backlog — `API_NOTES.md`, `SCHEMA_NOTES.md`, the roadmap in `EXTENSIONS.md`;
  "why it is built this way" — `ARCHITECTURE.md` and `EXTENSIONS.md`.
- Everything in the repository — specs, docs, code comments, commit messages —
  is in English; `make lint` refuses Cyrillic. Artifact rules are in
  `openspec/config.yaml`.

## Structure

- `apps/server` — crate `hawier-server`, binary `hawier` (server, both APIs)
- `apps/ui` — crate `hawier-ui` (UI; state only through the server's `/ui/v1` API)
- `apps/client` — crate `hawier-client`, binary `hawier-device` (DDI simulator)
- `apps/ext-*` — first-party wasm extensions (eight), `make ext`
- `libs/api` — crate `hawier-api`: shared serde DTOs and enum codes. **The only
  shared library — don't add new libs without extreme necessity.**

## Commit — part of the task, not a separate request

Finished a big task — **commit without waiting to be asked**. Work left in the
working tree stops being work: four independent slices once piled up here in
111 files, and they had to be sorted out afterwards, from someone else's
traces, with a risk of gluing unrelated things into one commit.

- **One slice — one commit.** Don't dump into it what the slice doesn't need,
  and don't pick up someone else's uncommitted edits that landed in the same
  files: stage your own changes, not whole files.
- **Every commit must build.** A change that drags a test or a caller edit
  along goes in one commit with them: "I'll fix it in the next one" is a
  broken history. That's what happened with moving the tests into one target:
  it dragged along the authz cache and the `HAWIER_EXT_*` permission, because
  the process became shared.
- **Before committing — `make lint`**; if you touched `apps/ui` or
  `static/assets` — also `make ui-audit`.
- **In the message — what and why.** git knows the list of files itself; what
  it doesn't know is why, and what breaks if it's done the other way.
- Don't create branches and don't push: the repository is local, the history
  is linear on `master`.

## Operation — only through make

`make lint` / `make test` (unit) / `make build` / `make it` (integration,
brings up Postgres from deploy/docker-compose.yml) / `make run` (server) /
`make run-ui` / `make device ARGS="..."` / `make load` (load run) /
`make deploy-<host>` (deploy to a host from `deploy/ansible/inventory.yml`;
`make deploy-list` — which ones exist) / `make logs-<host>` (what is going on
there) / `make creds-<host>` (what to log in with). Full list — `docs/dev/make.md`.

What the build, deploy, TLS and measurements do — specs `build-tooling`,
`remote-deployment`, `tls-proxy`, `load-benchmark`, `build-identity`. Below
are the rules for writing the Makefiles and the role so as not to break that
behaviour. Each one was paid for by a separate breakage.

**Container engine.**
- Not hardcoded: a new call is written through `$(ENGINE)` / `{{ hawier_engine }}`;
  `docker` in a recipe is a bug on a machine with podman. Order: podman, else
  docker; override — `ENGINE=` and `hawier_engine`.
- Only the common part of the CLI (`build`, `images`, `save`, `load`, `pull`,
  `rmi`, `compose`). Flags and Go templates that the engines name differently
  (`info --format`, `.CreatedAt`, `--project-directory`, `--env-file`, a glob
  in `--filter reference=`) are not to be used — or ask for both variants.
- An image from a registry — by its **full name** (`docker.io/library/...`,
  including in `FROM`): podman with `short-name-mode = enforcing` and no TTY
  doesn't resolve a short name. podman calls its own images `localhost/<repo>`
  — don't rely on the image name starting with the repository name. The image
  id differs between podman and docker, so presence on the host is checked by
  the **exact name**, not by id.
- The image stream and compose look at **one** store (`hawier_become_engine`
  versus `become`) — otherwise "no such image" right after the load; preflight
  checks this.

**The target machine downloads nothing.** This is a prohibition, not an
optimization: a host that can reach a registry can get the wrong bytes. The
base image is pulled here and travels as a stream; in the deployed compose
every service has `pull_policy: never`. If you add a step to the role, it has
no right to reach outward from the host.

**A wait must be able to give up.** Any wait in the role checks both "is it
ready" and "has it looped" — **in the same loop**: a check before the wait
sees a still-live container and sits out the whole timeout. A loop in ansible
is ended only by `until`; `failed_when` together with `until` merely marks the
attempt, and it is retried. A failed first start is stopped.

**A deploy must be readable when it doesn't work.**
- No compose call without `timeout`; the health wait is in the play, not in
  `depends_on: condition: service_healthy`.
- State and logs are asked **from the engine**, not from compose
  (podman-compose answers `ps -a` with nothing and mixes up service logs).
  Diagnostics are printed by make as plain text into a file — not ansible
  debug blocks. "Go to the host and run this" doesn't count as diagnostics.
- **No `${...}` in the deployed compose file**: every implementation does the
  substitution in its own way, and the failure is silent. Values are rendered
  into a 0600 file.
- **A `debug` string has no arithmetic and no filter chains** — a value from
  `-e` or quotes arrives as a string, and a cosmetic string has already
  brought a deploy down three times. A numeric variable is cast with `| int`
  at the point of use.

**Refuse the host — before the build.** `deploy-<host>` first runs
`preflight.yml`. A check that needs something on the host is written through
`raw` — otherwise the refusal arrives as a traceback about
`ansible.legacy.setup`. The role doesn't gather facts; the role doesn't
install host preconditions, it names them.

**TLS.** The proxy overwrites `X-Forwarded-For` (Caddy appends by default —
hence the explicit `header_up`). The Caddyfile is applied with `caddy reload`,
falling back to a container restart. The `caddy-data` volume holds the
certificates: `down -v` means re-issuing under Let's Encrypt limits.

**Deployment — only through the role.** Two compose files —
`deploy/docker-compose.yml` (test stand) and
`deploy/ansible/roles/stack/templates/docker-compose.yml.j2` (deploy) —
describe one stack: when you edit one, look at the other. Secrets are
generated on the host once and are not overwritten. The `ext-config` volume is
written by the node itself — ansible never writes into it. The deploy checks
the node's `node.build`, `node.profile`, `node.role` and `paramsFileWritable`,
the console's build, and, with TLS, the public endpoints.

**Hot path.** After changes in DDI, artifact or the rollout engine — `make
load` and a comparison with `bench/load-history.csv`; a degradation >20% at
the same parameters is a regression, sort it out before committing.

**Profiles.** Images are built with the `dev` profile — that's what people
look at; measuring and handing out — only `CARGO_PROFILE=release make scale`.
`make load` refuses to measure a debug node (it asks the node for
`debug_assertions`). `[profile.dev] debug = "line-tables-only"`: full DWARF is
76% of the test binary's bytes. If you need a debugging session in a
dependency, enable it selectively (`[profile.dev.package.sqlx] debug = 2`) and
don't commit it. Don't touch `debug-assertions` at all: the `make load` probe
and the `debug` pill on `/settings` rest on it. A feature that `cfg`-gates
nothing is not a feature but a second copy of the build graph: don't create
one. The toolchain is pinned in `rust-toolchain.toml` and in `Dockerfile` —
change them together: a change invalidates the whole `target/`.

**The build cache doesn't clean up after itself.** Cargo never deletes an
artifact it has abandoned (in four days — 47 GB, 24 of them unreachable). To
clean — `make gc` (`make gc-dry` shows what would go); heavy targets call
`gc-auto` themselves — don't disable it and don't replace it with a manual
ritual. Don't run `cargo clean`: it wipes `target/release/hawier-device` (used
by `scripts/load.sh` for measuring) and `target/wasm32-wasip1` (loaded by
`make it`). If you deleted or renamed a test target — sweep once explicitly,
`make gc GC_MAX_AGE=1`.

## DB schema compatibility — TEMPORARY

The schema is byte-for-byte compatible with hawkBit 1.0.0 (baseline `B1_20_0`
+ 2 deltas, Flyway history in `schema_version`). This is a temporary decision
for the development stage; hawier's own tables — only with the `hawier_*`
prefix. Contract — spec `database-schema`.

hawier's own migration track is **one file** `0001_hawier.sql`, edited in
place: there is no backward compatibility with the previous hawier at the
development stage. After an edit — `make reset-ui-state` (drops the `hawier_*`
console-state tables, doesn't touch hawkBit's `sp_*` data).

**Duty of every agent:** if you find a narrow place in the schema (a legacy
crutch, a missing constraint, an unfortunate type, a potential scale problem) —
1) mark the place in the code with the comment `// NARROW: <gist>` (in SQL
`-- NARROW:`), 2) add an item to `SCHEMA_NOTES.md` with a fix plan.
These items get fixed in hawier's pre-production schema revisions — not
"someday".

Edits to `sp_*` tables are forbidden (they break compatibility); only
`hawier_*` may be changed — in `apps/server/migrations/hawier/0001_hawier.sql`.

Limitations of the inherited **API** are recorded in `API_NOTES.md` (the
analogue of SCHEMA_NOTES for the protocol): the main recorded flaw — DDI
can't delegate polling to sub-nodes (plan: `_links.pollingBase`).
Doctrine: clients follow all `_links` and don't build URLs from base (except
the initial poll); don't write code that violates this.

The extension mechanism — specs `extension-host` and `first-party-extensions`,
rationale — `EXTENSIONS.md`. Two rules that must not be broken: **decisions and
descriptors cross the extension boundary, never bytes and never handles** (an
extension doesn't hold a `PgConnection` and doesn't stand in the byte path),
and **there are no in-process implementations of extension points** — an
extension is a wasm module, and when it is absent the host answers with a
declared refusal instead of doing the work itself. If you introduce an
extension point — do it as a delta to the `extension-host` spec.

## Architectural invariants

Normative — specs `node-roles`, `artifact-delivery`, `extension-host`. In code
this means:

- The DDI path (poll/deploymentBase/feedback/download) doesn't depend on
  singletons/background loops: everything that `spawn_background` spawns is
  control-node only.
- Download — streaming only (`Body::from_stream`), a file is never buffered
  whole.
- The core has only `artifact::Storage`: a spool with hashing, descriptor IO
  and a check that the path lies inside `HAWIER_ARTIFACT_DIR`. This check
  doesn't canonicalize the root, so the root is set **absolute** and equal to
  `HAWIER_EXT_FS_ROOT` (with a relative default, serving through `ext-fs` is
  500).
- Exact numeric enum codes — only from `libs/api/src/codes.rs`; don't
  hardcode numbers in queries. The rule is currently violated in
  `mgmt/mod.rs`, `mgmt/targets.rs`, `mgmt/actions.rs` — don't multiply it,
  convert to `codes.rs` when touching them.

## UI/UX — mandatory rules (not "at your discretion")

The console is a product just like the API. Edits to `apps/ui` and
`static/assets` are verified **on a live page**, not by the fact of
compilation. The owner must not find with their eyes what the agent is obliged
to catch.

**What the console must do** — specs `console-shell`, `console-lists`,
`console-objects`, `console-bulk-actions`, `console-search`,
`console-settings`, `console-help`, `console-visual-language`. A console
feature is done only if it meets their requirements and has passed
`make ui-audit`. How this is built in the code and what the audit catches:

| Rule (gist) | Spec | How in code / what checks it |
|---|---|---|
| A view/filter/sort is confirmed by the server | `console-lists` | before adding a view with `q=`/`sort=`, check the resource's field-map in `apps/server/src/mgmt/*.rs`; `order_by_with` admits only `Column`/`EnumColumn`, not `Field::Exists` |
| An error doesn't break the frame, three states | `console-lists` | `list_frame_survives_a_failing_query`; header strips don't change height |
| Rectangular style, palette, px | `console-visual-language` | `border-radius` is reset by the base CSS; don't reference undefined variables (`stylesheet_stays_consistent`) |
| Colour only through tokens | `console-visual-language` | all colours are on `:root` in `static/assets/main.css`, no hex outside the block (`colours_come_from_tokens`) |
| Counter and pager on top; sublists with a limit — with pagination | `console-lists`, `console-objects` | the pager is shared: macro `m::pager` over `view::Pager` |
| Numbers add up | `console-shell`, `console-objects` | the counter is computed by the same query the link leads to; an entity's "own" statistics (like `/distributionsets/{id}/statistics`) won't do if they answer a different question |
| Search — on the server | `console-search` | only `q` with RSQL wildcards; input is stripped of metacharacters (`hawkbit::rsql_literal`) |
| Single time format | `console-visual-language` | only `view::Cell::Time` / `Cell::TimeFull`; on the wire — epoch **milliseconds** as a number |
| JS degrades piecewise | `console-visual-language` | initializers go through a shared list with `try/catch`; a line starting with a parenthesis is protected by a `;` on the previous one (`client_script_cannot_break_silently`) |
| Trailing buttons in their own column | `console-lists` | `.list-row` + `.list-row-actions`; long labels — ellipsis |
| No custom lists | `console-lists` | a new entity — an entry in `RESOURCES` + `system_views()`; API — a mgmt resource with `ListParams`/`rsql_filter`/`order_by`/`paged`. An inherited endpoint won't do (bare array, no total, ignores `q`/`sort`) — a **resource alongside**, the old path untouched (that's how target groups are done). `Resource.row_href` — the value in RSQL **in quotes** |
| The column exists in the API response | `console-lists` | `declared_columns_exist_in_the_payload` |
| Every column is sortable | `console-lists` | `every_declared_column_is_sortable` (all resources × all columns); `mgmt_console_contract` — tricky fields |
| A resource has its own icon, svg instead of glyphs | `console-shell` | not symbols like `✎`; `every_resource_has_its_own_icon` |
| A collapsed panel without empty strips | `console-shell` | whatever can't be shown in 40px is hidden entirely |
| A named reference is clickable | `console-lists` | `<field>Name` + an adjacent id → `<a>` (`view::REFERENCE_FIELDS`) |
| Bulk actions | `console-bulk-actions` | the set is in `resources::bulk_actions`; the selection is visible (`tr.selected`), the select cell is a `<label>`; the confirmation is read from the pressed button — `event.submitter`, not from the form |
| Markup only in templates | — | HTML is in `apps/ui/templates` (askama); `apps/ui/src/**.rs` has no `<div>`/`<section>`/`<table>`/`<form>`/`<button>`/`<input>`; the only fragment is `view::icon` (`markup_lives_only_in_templates`) |
| No UUIDs | `console-visual-language` | ids are decimal snowflakes; the exception is the session token in the cookie |
| A dropdown — only for a short enumeration | `console-objects` | server lists — `[data-picker]` via `GET /pick/{resource}?q=`; a value that may be absent — `data-picker-free` (without it, input zeroes the hidden field and the form goes out empty) |
| The build version is visible | `build-identity`, `console-settings` | `HAWIER_BUILD` from `build.rs` |
| No "Back" and no logs outside the Action Log | `console-shell`, `console-visual-language` | `no_back_buttons_and_no_stray_log_panels` |
| Depth, focus, hiding | `console-visual-language` | layers — shadow `--shadow-*` and layer `--z-*`; `:focus-visible`; hiding — **only** with the `hidden` attribute on top of the global `[hidden] { display: none !important }`: any rule with `display` overrides the browser's `[hidden]` |
| A render error is 500 | `console-visual-language` | `view::respond` returns 500, `assert_no_error` looks for `template render failed` |
| A new page — a new row in the audit | — | `apps/ui/tests/it/ui_audit.rs` walks resources × system-views and static pages (the latter by a hand-written list); quick run `make ui-audit` |
| Everything is explained | `console-help` | the manual — `apps/ui/docs/**.md` (a subset of markdown, parsed by `apps/ui/src/help.rs`, HTML — only in `templates/help_*.html`); link `data-help="topic#anchor"`; anchor — `help::slug`; a file is created only together with a row in `src/help_topics.rs`; the text never contains `hawkBit`; a chapter is one page (`help_topics_render` counts `class="panel"`). A new column/field/view/operation/enum value without a section fails `every_declared_thing_is_documented` |
| Status colour — from one table | `console-help` | `help::status` is the only "word → colour"; read by `view::is_status`/`status_class` (`.status-badge`) and `.status-pill`. A new variant in `codes.rs` without a row fails `every_state_the_server_can_report_has_a_colour`; a badge coloured by the word is `a_status_word_looks_the_same_everywhere` |

Brand: the product is called **hawier**; in the interface — the word `Hawier`
at the left of the top bar, to its left — the sidebar collapse button
(`console-shell`). `hawkBit` is mentioned only in documentation as the source
of compatibility.

## Style

- Rust 2021, axum 0.8, sqlx (no ORM), no micro-dependencies: before adding a
  crate — check whether it can be written in ~100 lines.
- API errors — through `AppError` (hawkBit-compatible `ExceptionInfo` JSON).
- Integration tests — **one target per crate**: `apps/server/tests/it/` and
  `apps/ui/tests/it/`, a file per area, the root — `main.rs` with the list of
  `mod`s. A new file in `tests/` next to them Cargo would turn into a separate
  binary of a hundred and fifty megabytes — add only a module and a line to
  `main.rs`. All are marked `#[ignore]`, the targets have `test = false`,
  run with `make it` (needs `HAWIER_IT_DATABASE_URL`); one area —
  `cargo test -p hawier-server --test it -- --ignored ddi::`. `#[ignore]`
  never occurs in `src/`: `make it` doesn't go there, such a test would never
  run.
