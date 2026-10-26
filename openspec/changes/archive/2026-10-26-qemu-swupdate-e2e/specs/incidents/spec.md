# Spec Delta

## MODIFIED Requirements

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
