# Design

## Context

See proposal.md, section Why. The contract today lives in six prose documents
(about 2100 lines), plus CLAUDE.md's 29 UI rules and the integration and audit
tests. The tests are the most reliable source. A read-only survey of every area
compared the prose with the code and found two kinds of problem:
- the prose has drifted in places, including stale counts, stale file
  references, and behavior described that the code no longer has;
- the code has a set of divergences and likely bugs that no document mentions.

Both shape how the baseline is written.

## Goals / Non-Goals

**Goals:**
- One baseline spec per capability. After archive, `openspec/specs/` is the place
  to read what hawier does.
- Every requirement describes current, observable behavior, and every
  scenario could be a test. Most already are.
- A concrete plan for each existing document: what moves out, what stays, and
  what gets corrected.

**Non-Goals:**
- Fixing any divergence found during the survey. They are catalogued below and
  routed to the backlog or to follow-up changes. This change edits no code.
- Specifying planned work. Planned items stay in `API_NOTES.md`,
  `SCHEMA_NOTES.md` and the roadmap parts of `EXTENSIONS.md`.
- Replacing the in-console manual (`apps/ui/docs/**.md`). It is product content,
  compiled into the binary and audited, not developer documentation.

## Decisions

### D1. Flat capability layout, 31 capabilities
The specs use flat kebab-case names, with a `console-` prefix for the web
console. There are 18 server capabilities, 8 console capabilities and 5 tooling
capabilities, as listed in proposal.md.
- *Alternative: nested domains (`server/ddi-api`, `console/lists`).* Rejected.
  The project has no existing spec layout to follow. A prefix groups the console
  well enough, and flat paths are shorter to cite in future deltas.
- *Alternative: fewer, larger specs, one per crate.* Rejected. Future changes
  touch narrow areas, such as one rollout rule or one DDI resource. A delta
  against a 300-requirement "server" spec would be hard to review.

### D2. Specs in English, documents stay in Russian
The specs are written in English, as the owner chose. The structural keywords
are needed by the validator in any case. The prose documents and the manual
keep their language. `openspec/config.yaml` records this rule so later changes
follow it.

### D3. The code wins, and divergences are not codified
Where a document and the code disagree, the spec describes the code. Where the
code itself looks wrong, the requirement is written around the questionable
detail, and the detail goes into the catalogue below. Examples are a 400 where
416 is expected, and a rule that never fires. Writing a bug into a SHALL would
turn its fix into a "breaking" delta. Leaving it unstated keeps the fix an
ordinary ADDED or MODIFIED requirement later.

### D4. hawkBit compatibility stated once per surface
Requirements state hawkBit wire compatibility at the level of resource groups
and error bodies. They do not restate every hawkBit field, because hawkBit's
own API documentation is the reference. hawier's additive keys and resources are
listed explicitly in `mgmt-api`, because those are hawier's own contract.

### D5. No internal names in specs
Requirements do not name Rust functions, modules or test functions. Test
names are kept in the mapping in tasks.md, section 1, so that apply can check
each spec against its tests. They do not go into the specs, where they would
rot with refactors.

### D6. Documentation migration map
Each document ends up in one of three states: **link** (normative text replaced
by a pointer to the spec), **keep** (rationale, how-to or backlog), or **fix**
(a statement corrected to match the code).

| Document | Becomes | What changes |
|---|---|---|
| README.md | operator quick start and reference | Keep: quick start, the make table (completed), environment tables, deploy how-to. Link: deploy internals, TLS behavior, extension overview. Fix: claims about comparing image IDs, the `.env` file, `IMAGE_REPO`, "compose ps -a", the hardcoded `docker compose` in the scaling section, the missing load parameters |
| ARCHITECTURE.md | the rationale document | Keep: topology, failure model, the 10 Gbit path, trade-offs, measurements. Fix: the monorepo list (8 ext crates), "16 endpoints", "three hawier tables" (there are 12), the stale enum-vs-dyn paragraph about storage, the `ui_state` module description |
| EXTENSIONS.md | rationale plus backlog | Link: §4 point contracts → `extension-host`, `first-party-extensions`, `event-delivery`, `artifact-retention`, `rollout-engine`; §5 parameters → `extension-host`. Keep: §1–2 rationale and the invariant, NARROW notes, status block, §4.3 remote-shell, the §4.4 AuthProvider half, the hardened profile, §6. Fix: stale line references; "guest gets only clock and `HAWIER_EXT_*`"; "failure → safe default"; manifest limits presented as enforced; event-sink absence |
| API_NOTES.md | API backlog | Keep: §1, §3, §4, §6, the open part of §7, §8. Link: §2 doctrine → `device-simulator`, keeping a two-line doctrine. §5 "IMPLEMENTED" (~260 lines of history) → replaced by links to `mgmt-api` / `ddi-api`; history remains in git |
| SCHEMA_NOTES.md | schema backlog | Keep. Fix: the numbering of #19, #19a and #20 |
| CLAUDE.md | agent and engineering rules only | Keep: structure, commit discipline, make-only, NARROW duty, `sp_*` prohibition, style, test layout. The engineering-only UI rules (4, 9, 13, 19, 26, 27, the field-map half of 1, the docs-discipline half of 28) stay as rules. Product rules (UI rules 2, 3, 5–8, 10–12, 14–18, 20–25, 28, 29, brand) → one pointer to the `console-*` specs. The ops paragraphs (engine, target downloads nothing, waits, diagnostics, preflight, TLS, profiles, gc) → condensed to one-line invariants with pointers to `build-tooling`, `remote-deployment` and `tls-proxy`. The architectural invariants → links to `node-roles`, `artifact-delivery` and `extension-host`. Add: "behavior lives in openspec/specs; change it through an openspec change" |
| bench/NOTES.md | keep as is | It is an analysis and measurement log, not a contract |
| apps/ui/docs/** | keep as is | Product manual; audited by `every_declared_thing_is_documented` |
| openspec/config.yaml | project context | Add: stack, layout, language rule, where the backlog lives, "code wins, catalogue divergences", the NARROW duty |

*Alternative: delete the documents outright.* Rejected. Much of their value is
the "why", which specs deliberately exclude. CLAUDE.md is loaded into every agent
session, so it must stay short and point to the specs, not duplicate them.

## Risks / Trade-offs

- [Specs drift from code, the same way the prose did] → Each scenario was taken
  from an existing test where one exists (tasks.md §1 lists them). Future
  behavior changes go through deltas. config.yaml says so.
- [Some requirements cover untested behavior] → These are marked in tasks.md §1,
  and the gaps are listed for follow-up. The spec is still the right home for
  the contract. A missing test is a test gap, not a reason to leave the
  behavior unstated.
- [CLAUDE.md slimming loses rules agents follow today] → Only text that a spec
  now states is removed. Each removed paragraph is replaced by a one-line
  invariant and a pointer. Engineering rules that no spec covers stay verbatim.
- [31 specs are a lot to review at once] → The specs are grouped by area and
  each is short (4–15 requirements). The divergence catalogue is the list to
  read first.

## Divergence catalogue (not codified; routed by tasks.md §4)

Security and isolation:
1. `GET /rest/v1/rollouts/{id}/deploygroups/{g}/targets` does not check that the
   group belongs to the rollout or the tenant.
2. `X-Hawkbit-Tenant` lets any configured management user act in any tenant. The
   tenant is created on first use and has no grants, so everything is allowed
   there.
3. Incidents of management and ui kind have no tenant and are visible to every
   tenant, including the unverified usernames of failed logins.
4. `authentication.gatewaytoken.key` is returned in clear by
   `GET /system/configs`. `/ui/v1/sessions` stores and returns the password
   (already SCHEMA_NOTES #18).
5. Deleting a console search-history entry has no ownership check. Update and
   delete of a `/ui/v1` view by id are not scope-checked.
6. The console session cookie lacks `Secure`. The login form is pre-filled with
   `admin`/`admin`.

Behavior that is wrong or surprising:

7. A retention rule created through the API never fires: a never-run rule is
   measured from "now". The retention claim is released before the work runs.
8. With no event sink, the control node marks events delivered, whereas the
   documents say they accumulate. `DomainEvent.occurred_at` is set at drain time,
   not at emit time. `webhook_delivery.duration_ms` is never written.
9. With no rollout module, or with only the bucket module, a rollout created
   without `strategy` stays CREATING forever without a 400.
10. `startAt` does not auto-start a READY rollout.
11. The engine counts DOWNLOADED as finished, while the management API's
    `totalTargetsPerStatus` counts it as running.
12. Auto-assign has no cross-node claim, so two control nodes could
    double-assign.
13. A verdict's group id is not checked against the rollout being evaluated.
14. `assignedDS`/`assignedTargets` do not check that the set is valid, and
    `assignedTargets` does not check that it is deleted. Multi-item calls are not
    atomic.
15. Housekeeping action cleanup does not filter on `active`. One failing tenant
    aborts the whole tick.
16. DDI: an unsatisfiable or malformed Range answers 400, not 416.
17. DDI: `Content-Disposition` is not RFC 6266, so a non-ASCII filename gives 500.
18. DDI: deploymentBase feedback can overwrite a CANCELING action.
19. DDI: the DOWNLOAD status attaches to the top-weight action, and insert errors
    are ignored.
20. The relative default `HAWIER_ARTIFACT_DIR=./artifacts` makes every path
    descriptor be refused.
21. Extension failures: a module `err`, or an undecodable answer, gives 500, not
    503. A failed rollout evaluate leaves the rollout RUNNING. Manifest deadline,
    sites and fuel are reported but not enforced. The guest also receives the
    WASI default context, stderr, and the HTTP import.
22. RSQL: an existence field with `=in=(a,b)` uses only `a`. Attribute and
    metadata keys are lower-cased, so mixed-case keys never match. Wildcards on
    them do not escape `%` and `_`.
23. Tenant configuration: `/system/configs` shows
    `authentication.targettoken.enabled=false (global)` while the effective
    default is true. `batch.assignments.enabled` has no effect. Unknown keys are
    accepted silently. One reader matches the tenant case-sensitively.
24. `HAWIER_MGMT_USERS`: a password containing `:` is split into a tenant.
25. Error bodies over 64 KiB are returned empty.
26. Several sub-lists ignore `q`/`sort` (DS assigned/installed targets, assigned
    modules, deploy groups, `targetgroups/{g}/assigned`). Several return bare
    arrays.
27. Console: the time tooltip shows the raw epoch-ms, not the exact UTC time.
    Card sub-lists are capped without a pager (rollout deploy groups 100, target
    actions 10, DS modules 100, action status 100, overview rollouts 10). The
    dashboard console-noise counter uses a different query than the list it
    links to. Search input is stripped rather than escaped. The glyph audit scans
    only Rust and not the templates.
28. Simulator: without `--poll-override` every idle cycle polls twice. The
    configData PUT is built from the base URL when the poll has no link.
29. Tooling: `load_poll.sh` has no profile check and no `COUNT>=2` guard.
    Rollback has no guard that the image is present.
    A deploy log (`hawier-<host>.log.ansible`) is tracked by git because `.gitignore` misses
    `*.log.ansible`.
30. The "numeric codes only from codes.rs" rule is violated in three management
    files.
31. Untested behavior worth a test:
    - background loop intervals and isolation
    - stop, delete, retry, dynamic groups
    - confirmation and supersede inside rollout groups
    - outbox dead-letter and stale-claim release
    - the 302 redirect
    - anonymous DDI
    - gateway fleets with more than one device
    - `actionHistory`
    - configData limits
    - extension deadline and trap
    - the console's own extension-parameter form

## Migration Plan

Documentation only. It is applied in the order given in tasks.md and committed
in slices: the OpenSpec scaffold, then the documentation migration, then the
config. Rollback is `git revert` of a slice. Specs move to `openspec/specs/` only
when the change is archived (`/opsx:archive`), after apply.

## Open Questions

- Whether security items 1–5 get one follow-up change or one each. This does
  not affect this change.
