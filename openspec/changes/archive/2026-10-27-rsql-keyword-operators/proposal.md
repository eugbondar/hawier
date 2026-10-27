# Proposal

## Why

hawkBit 1.0 parses `q` with rsql-parser 2.1.0, whose grammar accepts ` and ` and ` or ` as aliases of `;` and `,`, and ends an unquoted value at whitespace. hawier knows only the punctuation and lets an unquoted value run to the next `, ; ( )`, so `controllerid==fleet-* or controllerid==cross*` is read as one comparison with the value `fleet-* or controllerid==cross*`. It matches nothing and answers 200 with total 0. As a rollout `targetFilterQuery` the same text creates a rollout with no targets that finishes at once. A filter written for hawkBit must mean the same thing here, and a filter hawkBit refuses must not silently match nothing.

## What Changes

- `q` accepts the lowercase keywords `and` and `or`, surrounded by whitespace, as `;` and `,`, with the same precedence.
- **BREAKING** An unquoted value ends at whitespace, as in hawkBit. Text after it that is not a separator, a keyword or a closing parenthesis is trailing input and gets 400. `name==rock and roll`, `a==1 AND b==2` and `a==1 && b==2` are now 400 instead of a comparison with a spaced value. A value with inner spaces must be quoted.
- Whitespace around `,` `;` `(` `)`, around the keywords and at either end of `q` is ignored. Whitespace around a comparison operator is still 400.
- The console's global search and pickers quote the needle, so a needle with spaces or the words `and`/`or` stays text.
- The in-console query manual documents the keywords and the new quoting rule.

## Capabilities

### New Capabilities

### Modified Capabilities
- `rsql-query`: "Grammar and operators" gains the keywords and whitespace rules; "Quoting" ends an unquoted value at whitespace.
- `console-search`: "Global search runs on the server" quotes the needle.

## Impact

- `apps/server/src/rsql.rs` (parser), its unit tests, and an integration test in `apps/server/tests/it/mgmt.rs`.
- Every `q` surface: Mgmt list resources, target filter queries, rollout target filter queries, retention rule filters.
- Stored filters created by hawier with an unquoted spaced value now fail validation when evaluated. hawkBit never accepted such a filter, so data migrated from hawkBit is unaffected.
- `apps/ui/src/routes.rs`, `apps/ui/src/logs.rs` (needle quoting), `apps/ui/docs/reference/rsql.md`.
- Not on the DDI, artifact or rollout-engine hot path: the parser runs once per request or rollout creation.
