# Spec Delta

## Purpose

Domain events are written transactionally to an outbox and delivered by the
control node through the event-sink extension. Webhook subscriptions and their
delivery log are management resources.

## ADDED Requirements

### Requirement: Event kinds
The server SHALL emit exactly these event kinds:
- `rollout.created`
- `rollout.started`
- `rollout.paused`, emitted by the engine on an error threshold and by an operator pause
- `rollout.finished`
- `assignment.created`
- `artifact.uploaded`
- `artifact.deleted`

#### Scenario: Operator pause looks like an engine pause
- **WHEN** an operator pauses a running rollout
- **THEN** a `rollout.paused` event with reason `paused by <user>` is emitted

### Requirement: Transactional outbox
An event SHALL be written to the outbox in the same transaction as the change it describes. A rolled-back change SHALL leave no event.

#### Scenario: Rejected create
- **WHEN** a rollout create fails validation
- **THEN** no `rollout.created` event exists

#### Scenario: Upload and delete
- **WHEN** an artifact is uploaded and then deleted
- **THEN** `artifact.uploaded` and `artifact.deleted` events exist for it

### Requirement: Delivery by the control node
The control node SHALL drain the outbox every 2 seconds:
- it SHALL claim up to 32 due events, oldest first, without blocking other drainers
- for each event, it SHALL ask the event-sink extension for a delivery plan against the tenant's active subscriptions
- the host SHALL perform each delivery itself, with a 10 s timeout, where any 2xx counts as success
- each attempt SHALL be recorded in the delivery log, with the HTTP status or the error text
- an event matching no subscription SHALL be marked delivered without any call

#### Scenario: Delivered and signed
- **WHEN** a subscription with a secret matches a pending event and the drainer runs
- **THEN** the receiver gets one POST with a valid signature, the event is delivered, and the delivery log has a 2xx entry

#### Scenario: Mask keeps unrelated events away
- **WHEN** a subscription's mask is `rollout.finished` and an `artifact.uploaded` event is drained
- **THEN** the receiver gets nothing

### Requirement: Retry with backoff and dead-letter
A failed delivery SHALL return the event to pending, record the last error, and back off `30 s × 2^(attempts−1)`, capped at one hour. After 8 attempts the event SHALL be marked dead and stay visible. A claim older than 300 s SHALL be released back to pending.

#### Scenario: Failing subscriber
- **WHEN** the receiver answers 500
- **THEN** the event is pending again with an error, and an immediate second drain does not retry it

### Requirement: Webhook subscriptions
`/rest/v1/webhooks` SHALL offer create, read, update, delete and list for subscriptions. A subscription SHALL have:
- `name`
- `url` (http or https only)
- `eventMask` (default `*`; every listed kind must be known, otherwise 400)
- `secret`
- `active`

The secret SHALL be write-only: it is shown only as `signed: true|false`, and an update without `secret` SHALL keep the stored one. `GET /rest/v1/webhookdeliveries` SHALL list the delivery log as an ordinary paged resource.

#### Scenario: Secret is never shown
- **WHEN** a subscription is created with a secret and read back
- **THEN** the response has `signed: true` and no secret value

#### Scenario: Unknown kind refused
- **WHEN** a subscription's mask names `rollout.exploded`
- **THEN** the server answers 400
