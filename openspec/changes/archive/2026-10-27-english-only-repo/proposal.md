# Proposal

## Why

The repository is about to be published on GitHub. Its project prose (README,
ARCHITECTURE, EXTENSIONS, API_NOTES, SCHEMA_NOTES, bench notes, CLAUDE.md) and
some code comments are in Russian, which most readers of a public repository
cannot read. The rule that kept them in Russian is retired: everything in the
repository is in English from now on, and the build has to keep it that way,
because the authors who write most of it are agents that follow whatever the
surrounding text looks like.

The only Cyrillic that was not prose was test data: an integration test
uploaded an artifact with a Cyrillic file name to prove that the management
download survives non-ASCII names. Rather than keep a deliberate exception to
the English-only rule, artifact file names are restricted to printable ASCII at
upload. A non-ASCII name was already a liability: the DDI download builds its
`Content-Disposition` without RFC 6266 encoding and answers 500 for such a name
(API_NOTES §9.2), and a device fleet cannot download what the server cannot
name.

## What Changes

- **Artifact file names.** `POST /rest/v1/softwaremodules/{id}/artifacts`
  refuses, with 400 and nothing stored, a file name that is empty, longer than
  256 bytes, contains a character outside printable ASCII, or contains `/` or
  `\`. The management download keeps its RFC 6266 header for names that
  already exist (a database migrated from hawkBit may hold them).
- **English-only repository.** All prose, comments and specs are translated.
  `make lint` fails when any tracked file contains Cyrillic.
- **Project rule.** CLAUDE.md and `openspec/config.yaml` state that the
  repository is English-only.

## Capabilities

### Modified Capabilities
- `mgmt-api`: artifact upload validates the file name.
- `build-tooling`: `make lint` refuses Cyrillic in tracked files.

## Impact

- `apps/server/src/mgmt/software_modules.rs` (upload validation),
  `apps/server/tests/it/mgmt.rs` (the non-ASCII download block is replaced by
  a rejection check), `Makefile` (lint), `apps/ui/docs/repository/artifacts.md`
  (the rule in the manual).
- hawkBit compatibility: hawkBit accepts any file name; hawier now refuses
  non-ASCII ones. Recorded in API_NOTES.md.
- The artifact download path itself is unchanged, so `make load` is not
  affected.
