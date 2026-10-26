# Proposal

## Why

hawier is moving to OpenSpec. Today its behavior contract is spread across prose
in CLAUDE.md, README.md, ARCHITECTURE.md, EXTENSIONS.md, API_NOTES.md and the
integration tests. `openspec/specs/` is empty, so every future change has
nothing to state a delta against. This change records what is already built and
tested as baseline capability specs. It also plans which prose should then be
replaced by a pointer to a spec, reduced to rationale, or kept as it is.

## What Changes

- Add baseline specs (ADDED requirements only) for every implemented capability
  of the server, the console, the device simulator and the build/deploy tooling.
  Each requirement describes behavior as it exists today and is backed by code,
  and wherever possible by an integration or audit test.
- Behavior does not change. No code, schema or wire format changes. Where
  existing documentation contradicts the code, the specs follow the code. The
  discrepancy is listed as a documentation fix in tasks.md and is not "fixed" in
  the spec.
- Planned or unimplemented items are kept out of the specs and stay as backlog in
  the `*_NOTES.md` files. They become separate changes later. Examples are DDI
  `pollingBase` delegation, the hardened profile, resumable upload, remote-shell,
  the oauth `AuthProvider`, and the NARROW schema items.
- Plan a documentation migration (executed in tasks.md):
  - Normative sections of README.md, ARCHITECTURE.md, EXTENSIONS.md and
    API_NOTES.md that duplicate a spec are replaced by a link to that spec.
  - Rationale moves into or stays in ARCHITECTURE.md and EXTENSIONS.md as "why"
    documents.
  - CLAUDE.md keeps only agent and engineering process rules (commit discipline,
    make-only, NARROW marking, test layout, style), and points to specs for
    product behavior.
  - `openspec/config.yaml` receives project context.
  - `apps/ui/docs/**.md` is product content (the in-console manual) and is not
    touched.

## Capabilities

### New Capabilities

Server:
- `node-roles`: control vs sub node roles, what runs where, and survival of the device path when the control node is dead.
- `ddi-api`: the device-facing API: endpoints, device authentication modes, the poll response and `_links`, deployment/cancel/confirmation/installed-base/config-data, the feedback state machine, maintenance windows, and overload behavior.
- `artifact-delivery`: artifact download semantics: streaming, ETag/Range/conditional requests, hash files, redirects, and the declared refusal without storage.
- `mgmt-api`: the `/rest/v1` management API: resource groups, list conventions (paging/sort/q/total), hawier additive resources, and the read-only sub node.
- `rsql-query`: the FIQL/RSQL filter language, per-resource field maps, sortability, and error behavior.
- `access-control`: Mgmt Basic users, DDI tokens, the internal bearer token, roles, and the authorization decision point.
- `incidents`: the hawkBit-compatible error body, decimal incident numbers, and incident journaling and lookup.
- `tenant-configuration`: supported tenant configuration keys, their defaults and effects.
- `database-schema`: hawkBit 1.0.0 schema compatibility, Flyway history, the hawier migration track, and startup database wait.
- `deployment-assignment`: assigning distribution sets to targets: action types, supersede/autoclose, confirmation, implicit lock, multi-assignment, and cancellation.
- `rollout-engine`: the rollout lifecycle, group materialization via strategy, thresholds, auto-assignment, and engine idempotence.
- `action-housekeeping`: control-node housekeeping: action auto-cleanup and expiry of console sessions.
- `artifact-retention`: retention rules and their evaluation through the retention extension.
- `event-delivery`: the transactional event outbox, event kinds, delivery through the event-sink extension, and webhooks.
- `extension-host`: the wasm extension mechanism: points, execution sites, the decisions-not-bytes ABI, deadlines, guest capabilities, parameters, declared refusals, and the extensions report endpoint.
- `first-party-extensions`: the shipped extension modules (fs, s3, webhook, retention, rollout-classic, rollout-bucket, console, authz) and their decisions and parameters.
- `ui-state-api`: the internal `/ui/v1` API for console state: saved views, action log, sessions, searches, surfaces.
- `build-identity`: the `vYY.MM.DD-<sha>` build stamp and the build profile each component reports.

Console:
- `console-shell`: login and session, the top bar and brand, the sidebar and its collapse, navigation, and health.
- `console-lists`: resource lists: system and saved views, columns, sorting, server-side search, the counter and pager, the three states, and row links.
- `console-objects`: object pages, forms and pickers, and cross-object links.
- `console-bulk-actions`: row selection, the context menu, declared bulk operations, and destructive-operation confirmation.
- `console-search`: global search, live search, and recent searches.
- `console-settings`: the settings page: tenant configuration editing, extension parameters, and component versions.
- `console-help`: the in-console manual, the "what is this?" mode, and documentation coverage of every declared thing.
- `console-visual-language`: the visual and interaction invariants: shape, color tokens, the status colour table, time format, decimal ids, layering and focus, and JS degradation.

Tooling:
- `device-simulator`: the `hawier-device` simulator and load client: modes, fault injection, and the `_links` doctrine.
- `load-benchmark`: `make load` / `make load-poll`: refusal to measure debug nodes, history files, and cleanup.
- `build-tooling`: the make interface, container engine selection, build profiles, toolchain pin, and build cache GC.
- `remote-deployment`: `make deploy-<host>` via ansible: preflight, image streaming, the rendered compose, secrets, health wait, diagnostics, verification, rollback, and creds.
- `tls-proxy`: the Caddy front with ACME, forwarded headers, routing, and config reload.

### Modified Capabilities

None. No specs exist yet.

## Impact

- New files only under `openspec/changes/baseline-existing-capabilities/`.
  Archiving creates `openspec/specs/<capability>/spec.md` for each capability
  above.
- Apply phase edits documentation only:
  - README.md, ARCHITECTURE.md, EXTENSIONS.md, API_NOTES.md and CLAUDE.md are
    trimmed or linked;
  - `openspec/config.yaml` gets project context;
  - doc statements found to contradict the code are corrected.
- No Rust, SQL, template, static asset or deploy file changes. `make lint` and
  `make ui-audit` are unaffected. `every_declared_thing_is_documented` is
  unaffected because `apps/ui/docs` stays.
