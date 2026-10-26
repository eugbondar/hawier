# Tasks

## 1. Artifact file names

- [x] 1.1 Validate the artifact file name in the management upload before any byte is stored: 400 `invalidParameter` for empty, over 256 bytes, non-printable-ASCII, `/` or `\`. Verify with `cargo test -p hawier-server --test it -- --ignored mgmt::`.
- [x] 1.2 Replace the non-ASCII download block of the multipart upload test with rejection checks (non-ASCII via a Rust escape, and `a/b.bin`); assert the artifact list did not grow.
- [x] 1.3 State the rule in `apps/ui/docs/repository/artifacts.md#uploading`; run `make ui-audit`.
- [x] 1.4 Record the divergence from hawkBit in API_NOTES.md and narrow §9.2 to migrated names.

## 2. English-only

- [x] 2.1 Translate README.md, ARCHITECTURE.md, EXTENSIONS.md, API_NOTES.md, SCHEMA_NOTES.md, bench/NOTES.md and CLAUDE.md, keeping section and item numbering that code refers to.
- [x] 2.2 Translate the comments in `apps/server/tests/it/scenarios.rs`, the comment in `deploy/ansible/inventory.yml` and the quoted Russian titles in the archived baseline change.
- [x] 2.3 Change the language rule in CLAUDE.md and `openspec/config.yaml` to English-only.
- [x] 2.4 Add the Cyrillic check to `make lint`; verify it fails on a planted Cyrillic line and passes on the clean tree.
- [x] 2.5 `make lint`, `make test`, `make it`, `make ui-audit`, `openspec validate --strict`.
