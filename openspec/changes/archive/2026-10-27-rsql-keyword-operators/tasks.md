# Tasks

## 1. Parser

- [x] 1.1 End unquoted values at whitespace, ignore whitespace around `, ; ( )`, inside value lists and at both ends of `q`, and accept lowercase ` and ` / ` or ` as `;` / `,` in `apps/server/src/rsql.rs`; verify with `cargo test -p hawier-server rsql::`
- [x] 1.2 Unit tests: keyword AND/OR and precedence equal to the punctuation form, mixed forms, keyword after `)`, extra whitespace, `and`/`or` inside values, uppercase `AND`/`OR`, `&&`, a spaced unquoted value, and a dangling keyword are 400; verify they pass

## 2. Integration

- [x] 2.1 Integration test in `apps/server/tests/it/mgmt.rs`: `q` with ` or ` gives the same total as `,`, ` and ` the same as `;`, `OR` gets 400, a quoted spaced value matches, and a rollout with an uppercase-keyword `targetFilterQuery` gets 400; verify with `make it`

## 3. Console

- [x] 3.1 Quote the search needle in global search, live search, pickers and the action log search (`apps/ui/src/routes.rs`, `apps/ui/src/logs.rs`); verify a needle with spaces and ` and ` finds its target on the live search page
- [x] 3.2 Document the keywords and the whitespace rule in `apps/ui/docs/reference/rsql.md`; verify `/help/reference/rsql` renders

## 4. Close

- [x] 4.1 `make lint`, `make ui-audit` and `make it` are green
- [x] 4.2 Sync the deltas into `openspec/specs/`, archive the change, and commit one slice on master
