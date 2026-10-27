# Proposal

## Why

The README is the first thing a visitor of the repository sees, and today it is
a 490-line operator reference: every make target, every environment variable,
the load-test knobs, the QEMU command line, a size budget for a free GitHub
runner, a changelog paragraph about type compatibility, pointers into an
archived design.md. Someone evaluating hawier as a self-hosted OTA server
cannot find out from it what the product looks like or why to pick it — there
is no logo and not a single screenshot of the console, which is the product's
most visible part.

## What Changes

- The README is rewritten as a landing page: a logo in the header, a one-line
  pitch, screenshots of the console, a short list of what hawier does, a
  quick start, a two-step deploy and links onward. It stays short by contract
  (no reference tables, no make-target or environment-variable lists).
- The technical reference that is worth keeping moves into a dedicated
  subfolder, `docs/dev/` — material for people who build, run, measure and
  deploy hawier, as opposed to documentation of the product itself (which
  will get its own sibling folder later). Content that only repeats a spec is
  cut and replaced by a link to the spec; content that has no reader at all
  (the GitHub-runner budget table, the "type compatibility is now enforced"
  changelog note, references to archived design items) is dropped.
- A logo: an SVG mark plus wordmark drawn in the console's rectangular style
  and palette, with light and dark variants. It is used in the README only;
  the console top bar keeps the plain word `Hawier`.
- A new make target, `make screenshots`, produces the README screenshots from
  a throwaway node and console seeded with demo data, captured by a pinned
  headless browser in a container. The pictures are reproducible after any
  console change instead of being captured by hand and going stale.
- `make lint` checks that every relative link and image in `README.md` and
  `docs/**` points at a file in the tree, so moving content cannot leave dead
  links behind.
- References to README sections from elsewhere (CLAUDE.md, a code comment in
  the server) are repointed at the new locations.

## Capabilities

### New Capabilities
- `repository-docs`: what the README contains and what it must not, where the
  developer/operator reference lives (`docs/dev/`), the logo and screenshot
  assets, and that links in them resolve.

### Modified Capabilities
- `build-tooling`: the make target list gains `screenshots`; `lint` gains the
  link check; a new requirement describes how `make screenshots` builds its
  stand, what it captures, where it writes and when it refuses.

## Impact

- Files: `README.md` (rewritten), new `docs/dev/*.md`, new `docs/assets/`
  (logo SVGs, screenshot PNGs), `Makefile` (`screenshots` target, link check
  in `lint`), new `scripts/screenshots.sh` and `scripts/screenshots.py`,
  `CLAUDE.md` ("Full list — README" pointer), the module comment in
  `apps/server/src/db/flyway.rs`.
- No change to the server, the console, the APIs or the database. The
  `console-shell` requirement "no badge or logo" in the top bar is unchanged.
- New external tool: a pinned Playwright container image, pulled on the
  developer machine only when `make screenshots` runs; nothing in `make all`,
  `make build`, `make it` or the deploy depends on it.
- Repository size: the screenshots are committed binaries; the harness caps
  the size of each picture so a careless capture cannot bloat the history.
- Not touched by this change: the other root documents (`ARCHITECTURE.md`,
  `EXTENSIONS.md`, `API_NOTES.md`, `SCHEMA_NOTES.md`). They are referenced
  from CLAUDE.md, `openspec/config.yaml` and many specs; whether they move
  into `docs/dev/` is a separate decision.
