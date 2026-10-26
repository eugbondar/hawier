# incidents Specification

## Purpose

How the server reports errors: hawkBit-compatible error bodies, a decimal
incident number on every request, the incident journal, and its query API.

## Requirements

### Requirement: ExceptionInfo error body
Every error SHALL carry a JSON body `{exceptionClass, errorCode, message, incident}`. Status codes and error codes SHALL map as follows; every `errorCode` SHALL carry the `hawkbit.server.error.` prefix:

| Status | errorCode suffix |
|---|---|
| 404 | `repo.entitiyNotFound` |
| 400 | `repo.invalidParameter` |
| 400 (RSQL) | `rest.param.rsqlInvalid` |
| 400 (assignment to an incompatible or, under `target.type.required`, untyped target) | `target.type.incompatible` |
| 401 | `unauthorized` |
| 403 | `forbidden` |
| 403 (quota) | `repo.quotaExceeded` |
| 409 | `repo.entitiyAlreayExists` (hawkBit's spelling) |
| 410 | `repo.actionNotCancelable` |
| 503 | `serviceUnavailable` |
| 500 | `internal` |

A plain-text rejection from the HTTP layer SHALL be rewritten into the same JSON shape. Examples are malformed JSON, 405 and 415. The rewritten body SHALL keep the original text as `message` and use these codes: `methodNotAllowed`, `unsupportedMediaType`, `notFound`, `internal`, otherwise `rest.invalidRequest`.

#### Scenario: Malformed JSON
- **WHEN** `POST /rest/v1/targets` is sent with the body `{not json`
- **THEN** the response is 400 JSON with an `incident` field

#### Scenario: Wrong method
- **WHEN** `DELETE /rest/v1/userinfo` is requested
- **THEN** the response is 405 with errorCode `hawkbit.server.error.methodNotAllowed`

#### Scenario: Incompatible assignment
- **WHEN** a set is assigned to a target whose type does not list the set's type
- **THEN** the response is 400 with errorCode `hawkbit.server.error.target.type.incompatible` and an `incident` field

### Requirement: Incident number on every request
Every request, successful or not, SHALL get an incident number in `X-Request-Id`. A client-supplied value SHALL be overwritten.

The number SHALL be a decimal 63-bit snowflake built from:
- 41 bits of milliseconds since 2026-01-01T00:00Z
- 10 bits of node id (`HAWIER_NODE_ID`, 0..1023, defaulting to a hostname hash)
- 12 bits of a per-millisecond sequence

Numbers SHALL be unique and monotonic per node. On error, the body's `incident` string SHALL equal the header.

#### Scenario: Header and body agree
- **WHEN** a management request fails with 404
- **THEN** the body's `incident` equals the `X-Request-Id` header, and decoding it yields this node's id

### Requirement: Incident journal
The server SHALL journal:
- every 5xx response
- every 4xx response on `/rest/*` and `/ui/*`

DDI 4xx responses SHALL NOT be journaled.

Each entry SHALL be keyed by the incident number and record:
- kind: `ddi`, `mgmt`, `ui`, or `console` for management calls marked as console-originated
- method, path, status, errorCode
- message, truncated to 2000 characters
- username
- tenant, for DDI

The journal SHALL keep the newest 10000 entries.

#### Scenario: Management error lands in the journal
- **WHEN** a management request fails with 400 for user admin
- **THEN** an incident with kind `mgmt` and username `admin` exists under the response's number

### Requirement: Incident query API
`GET /rest/v1/incidents` SHALL list journal entries as a paged, filterable and sortable resource. `GET /rest/v1/incidents/{n}` SHALL return the entry with these fields:
- `incident`
- `kind`
- `occurredAt`, decoded from the number
- `node` and `sequence`
- `tenant`, `username`
- `method`, `path`, `status`, `errorCode`, `message`

A missing number SHALL get 404, saying that the entry may have been trimmed. DDI entries SHALL be visible only to their own tenant.

#### Scenario: Look up by number
- **WHEN** an operator reads `/rest/v1/incidents/<n>` for a number from an error response
- **THEN** the entry shows the same path, status and errorCode as that response
