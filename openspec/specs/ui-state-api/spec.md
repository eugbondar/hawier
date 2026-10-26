# ui-state-api Specification

## Purpose

The internal `/ui/v1` API through which the console keeps its own state:
saved views, the two-phase action log, recent searches, sessions, the server
version, and extension-declared surfaces. The console holds no state of its
own outside this API.

## Requirements

### Requirement: Internal API conventions
`/ui/v1` SHALL be served on control and sub nodes and SHALL require the internal bearer token. Bodies SHALL be snake_case JSON. Ids SHALL be decimal strings, and timestamps epoch milliseconds.

#### Scenario: Served on a sub node
- **WHEN** the console talks to a sub node
- **THEN** `/ui/v1` answers with the same data as on the control node

### Requirement: Saved views
`/ui/v1/views` SHALL store per-tenant, per-user views. A view has `resource`, `name`, `q`, `sort`, `columns` and `page_size`; `page_size` defaults to 25, with a minimum of 1.

Operations:
- list, filtered by tenant, username and optionally resource, ordered by position
- create, appending at the end, answering 201
- partial update
- delete, answering 204
- reorder via `PUT /ui/v1/views/reorder` with `{tenant, username, resource, ids[]}`

A missing id SHALL get 404.

#### Scenario: Reorder
- **WHEN** three views are reordered to the reverse order
- **THEN** listing them returns the new order

### Requirement: Two-phase action log
`POST /ui/v1/logs` SHALL open an entry with status `running`. `PUT /ui/v1/logs/{id}` SHALL finish it with its status, a response summary and an error, and SHALL set `finished_at`. Each tenant SHALL keep its newest 1000 entries. Entries SHALL be read through the management resource `/rest/v1/actionlog`, scoped to the caller's tenant.

#### Scenario: Started then finished
- **WHEN** the console records an action and then finishes it with `ok`
- **THEN** `/rest/v1/actionlog` shows the entry with both timestamps in milliseconds

### Requirement: Recent searches
`/ui/v1/searches` SHALL keep at most 20 recent queries per user, newest first, with use counts:
- a repeated query increases its count
- an empty query is a no-op
- single entries and the whole history can be deleted

#### Scenario: Repeat search
- **WHEN** a user runs the same search twice
- **THEN** it appears once with `uses: 2`

### Requirement: Sessions
`POST /ui/v1/sessions` SHALL create a session that expires 8 hours later, with a numeric id and a separate random 32-hex token. `GET /ui/v1/sessions/{token}` SHALL return the session, or 404 if it is missing or expired. `DELETE /ui/v1/sessions/{token}` SHALL be idempotent and answer 204.

#### Scenario: Token is not the id
- **WHEN** a session is created
- **THEN** its token differs from its id and is 32 hex characters

### Requirement: Version and surfaces
`GET /ui/v1/version` SHALL return `{build, profile, node, role}` for the answering node. `GET /ui/v1/surfaces` SHALL return the console extension's declaration, or an empty declaration when none is configured.

#### Scenario: No console extension
- **WHEN** no console extension is configured
- **THEN** `/ui/v1/surfaces` answers 200 with no surfaces and no views
