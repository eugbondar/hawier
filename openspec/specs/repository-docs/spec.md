# repository-docs Specification

## Purpose

What a visitor of the repository reads first and where the rest of the prose
lives: the README is a short landing page with the logo and console
screenshots, and the reference for building, running, measuring and deploying
hawier lives in `docs/dev/`.

## Requirements

### Requirement: The README is a landing page
`README.md` SHALL contain, in this order: the logo in the header, a one-line
statement of what hawier is, console screenshots, a short list of what hawier
does, a quick start, a deploy in at most two steps, and a section of links to
further documentation. The quick start SHALL bring up a working stack with one
make command and name the console and Mgmt API addresses and the stand
credentials.

The README SHALL NOT contain tables of make targets, of environment variables,
of inventory or tuning variables, or of load-test parameters; it SHALL NOT
contain resource budgets of CI runners, changelog or "is now enforced" notes,
or references to items of archived OpenSpec changes. Each of these either
lives in `docs/dev/`, is stated by a spec, or is dropped.

The README SHALL claim compatibility only with what the repository verifies:
the hawkBit Mgmt and DDI wire contracts, the hawkBit 1.0.0 database schema,
and SWUpdate/suricatta as a device client (verified by `make e2e`). Other
hawkBit clients MAY be named only as untested.

#### Scenario: A visitor opens the repository
- **WHEN** the README is rendered on the repository's front page
- **THEN** the logo, the one-line statement and at least one console
  screenshot are visible before the first command block

#### Scenario: Reference material creeps back
- **WHEN** a change adds a table of environment variables to `README.md`
- **THEN** the change does not meet this requirement and the table belongs in
  `docs/dev/configuration.md`

### Requirement: Developer and operator reference lives in docs/dev
The repository SHALL keep its reference for people who build, run, measure
and deploy hawier under `docs/dev/`, with `docs/dev/README.md` as the index
linking every page in the folder. The folder SHALL cover at least: the make
targets, server and console configuration (environment variables and
extension parameters), the device simulator, the load run, the device
end-to-end run, deployment to a host including TLS and the pre-production
checklist, and running with extension modules.

A page in `docs/dev/` SHALL describe how to do something and SHALL link to the
spec for what the system guarantees instead of restating its requirements.
`docs/` itself SHALL hold no prose pages directly: each kind of documentation
is a subfolder (`docs/dev/` now; product documentation in its own sibling
later), and shared images live in `docs/assets/`.

#### Scenario: Looking up an environment variable
- **WHEN** an operator needs the meaning and default of `HAWIER_MAX_CONNECTIONS`
- **THEN** it is in `docs/dev/configuration.md`, reachable from the README's
  documentation links through `docs/dev/README.md`

#### Scenario: A new page in docs/dev
- **WHEN** a page is added under `docs/dev/` without a link from
  `docs/dev/README.md`
- **THEN** the change does not meet this requirement

### Requirement: Logo
The repository SHALL carry the hawier logo as SVG in `docs/assets/`, in a
variant for light backgrounds and one for dark backgrounds. The README header
SHALL show the variant that matches the reader's colour scheme and SHALL give
the image the alternative text `hawier`. The logo SHALL use only colours of
the console palette and square corners, matching the console's visual
language. The console itself SHALL NOT show the logo: its top bar keeps the
plain word `Hawier` (spec `console-shell`).

#### Scenario: Dark theme on the repository host
- **WHEN** the README is rendered for a reader whose colour scheme is dark
- **THEN** the header shows the dark-background variant of the logo

#### Scenario: Image not loaded
- **WHEN** the logo file cannot be loaded
- **THEN** the header still reads `hawier` through the image's alternative text

### Requirement: Screenshots are generated, not hand-made
The console screenshots shown in the README SHALL be the files written by
`make screenshots` into `docs/assets/screenshots/`. They SHALL NOT be edited or
replaced by hand; refreshing them means running the target again. Each
screenshot SHALL show the console with demo data (non-empty lists, a rollout
in progress) and no credentials, tokens or host names other than the local
stand's.

#### Scenario: The console changed
- **WHEN** a change alters the look of a page shown in the README
- **THEN** running `make screenshots` and committing its output is enough to
  bring the README up to date

### Requirement: Links in the documentation resolve
Every relative link and image reference in `README.md` and in every Markdown
file under `docs/` SHALL point at a file or directory that exists in the
working tree; a `#fragment` after the path is not checked. `make lint` SHALL
fail on a reference that does not resolve and SHALL name the file, the line
and the target.

#### Scenario: A page is moved
- **WHEN** `docs/dev/deployment.md` is renamed and the README still links to
  the old name
- **THEN** `make lint` fails, naming `README.md`, the line and
  `docs/dev/deployment.md`

#### Scenario: External links
- **WHEN** the README links to an `https://` address
- **THEN** the link check ignores it
