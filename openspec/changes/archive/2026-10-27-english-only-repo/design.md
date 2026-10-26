# Design

## File name rule

The rule is checked in the upload handler right after the name is chosen (the
`filename` query parameter, else the multipart part's file name) and before a
single byte reaches storage, so a refused upload leaves nothing to clean up.
The refusal is the ordinary validation error: 400 with
`hawkbit.server.error.repo.invalidParameter`, the same body the handler already
returns for a missing name or a checksum mismatch.

A name is accepted when it is:
- not empty and not only spaces;
- at most 256 bytes (the width of `sp_artifact.provided_file_name`; an
  over-long name used to surface as a database error, i.e. 500);
- printable ASCII only, 0x20–0x7E;
- free of `/` and `\` (the DDI download addresses an artifact by its name as a
  single path segment, so a name with a slash could never be downloaded).

The console proxies uploads to the management API and already shows the
management error message on its "Upload failed" page, so it needs no rule of
its own.

## Divergences

1. **hawkBit accepts any file name.** hawier refuses non-ASCII names. A client
   that uploads such names gets 400 instead of 201. Recorded in API_NOTES.md as
   a deliberate divergence.
2. **Names that already exist are not migrated.** A database taken over from
   hawkBit may contain non-ASCII names. The management download still serves
   them with an RFC 6266 header; the DDI download still answers 500 for them
   (API_NOTES §9.2 stays open, now limited to migrated data).

## Lint guard

`make lint` runs `git grep --untracked -P` for the Cyrillic block
(U+0400–U+04FF): tracked files and new files not yet added, ignored ones
excluded, because lint runs before `git add`. The pattern is written with escapes, so the Makefile does not
match itself. Tests that need non-ASCII input use Rust escapes (`\u{fc}`), not
literal characters.
