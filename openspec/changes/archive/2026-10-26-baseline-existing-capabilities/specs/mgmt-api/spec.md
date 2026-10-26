# Spec Delta

## Purpose

The hawkBit-compatible management REST API under `/rest/v1`, and the resources
hawier adds to it. This spec covers shared list conventions, resource groups,
wire-format rules, and hawier's additive endpoints.

## ADDED Requirements

### Requirement: Paged list envelope
List endpoints SHALL accept these parameters:
- `offset` (default 0, must not be negative)
- `limit` (default 50, 1..500)
- `sort`
- `q` (RSQL)
- `representation`

They SHALL answer `{"content":[...], "total":N, "size":M}`, where `total` counts every match regardless of `limit` and `offset`. A limit outside 1..500 or a negative offset SHALL get 400. An empty or blank `q` SHALL be ignored.

#### Scenario: Total ignores the page
- **WHEN** `GET /rest/v1/targets?q=controllerid==t-*&limit=1` matches two targets
- **THEN** the response has `total: 2`, `size: 1` and one element

#### Scenario: Limit too large
- **WHEN** a list is requested with `limit=501`
- **THEN** the server answers 400

### Requirement: Sorting
`sort` SHALL be a comma-separated list of `field[:ASC|DESC]`. The direction SHALL be case-insensitive and default to ASC; any other direction SHALL get 400. Without `sort`, each resource SHALL use its own default order. Only fields declared sortable for the resource SHALL be accepted. Any other field SHALL get 400 with `errorCode` `hawkbit.server.error.repo.invalidParameter` and message `cannot sort by '<field>'`.

#### Scenario: Descending by name
- **WHEN** targets are listed with `sort=name:DESC`
- **THEN** they are returned in descending name order

#### Scenario: Unsortable field
- **WHEN** targets are sorted by `attribute.hw`
- **THEN** the server answers 400 naming the field

### Requirement: Array create
Create endpoints for targets, distribution sets, software modules, tags and types SHALL accept a JSON array or a single object, and SHALL answer 201 with an array. Target filters SHALL take and return a single object.

For targets:
- `name` SHALL default to `controllerId`
- the generated `securityToken` SHALL be 32 hex characters
- a duplicate `controllerId` SHALL get 409

#### Scenario: Create three targets
- **WHEN** three target objects are posted in one array
- **THEN** the response is 201 with an array of three targets, each with a 32-hex security token

### Requirement: hawkBit resource groups
The API SHALL serve the hawkBit resource groups at their hawkBit paths:
- targets: CRUD, attributes, actions and action status, assigned and installed distribution set, tags, metadata, target type, auto-confirm
- distribution sets: CRUD, assigned and installed targets, assigned modules, auto-assign filters, metadata, statistics, invalidate
- software modules: CRUD, artifacts with multipart upload and download, metadata
- target tags and distribution set tags, with assignment
- target types, distribution set types and software module types, with their type relations
- target filters, with auto-assign
- target groups
- rollouts, with deploy groups and lifecycle actions
- actions
- system configuration
- userinfo

Request and response bodies of these groups SHALL be wire-compatible with hawkBit, so existing hawkBit clients keep working. Additional response keys SHALL be additive only.

#### Scenario: hawkBit client
- **WHEN** a client written for hawkBit's management API lists targets and reads one
- **THEN** every field it relies on is present with the hawkBit name and type

### Requirement: hawier additive keys
The API SHALL add these response keys beyond hawkBit:
- targets: `pollState` (one of `responding`, `overdue`, `never`), `assignedDistributionSetName`, `installedDistributionSetName`, `targetTypeName`, `autoConfirmActive`
- action status rows: `repeats`, `lastReportedAt`
- actions: `controllerId`, `rollout`, `rolloutName`, `rolloutGroup`
- target filters: `autoAssignDistributionSetName`, null when unset
- userinfo: `permissions`

`pollState` SHALL be `overdue` when now exceeds the last poll + `pollingTime` + `pollingOverdueTime`. A malformed polling configuration SHALL fall back to the defaults rather than fail the list.

#### Scenario: Overdue device
- **WHEN** `pollingTime` and `pollingOverdueTime` are both one minute and a device has been silent for seven minutes
- **THEN** its `pollState` is `overdue` both in the list and on the single-target GET

#### Scenario: Liveness partitions the fleet
- **WHEN** targets are listed with `q=pollState==responding` and with `q=pollState!=responding`
- **THEN** the two totals add up to the unfiltered total

### Requirement: hawier additive resources
The API SHALL serve these hawier resources, each as an ordinary paged and filterable resource unless noted:

| Resource | Paths | Notes |
|---|---|---|
| target group details | `GET /rest/v1/targetgroupdetails` | rows `{name, targets}`; `name` is filterable, and `targets` is sortable only (filtering on it gets 400) |
| distribution set tags | `GET /rest/v1/distributionsets/{id}/tags` | |
| action log | `GET /rest/v1/actionlog`, `GET /rest/v1/actionlog/{id}` | read-only; written through `/ui/v1` |
| incidents | `GET /rest/v1/incidents`, `GET /rest/v1/incidents/{n}` | |
| webhooks | `/rest/v1/webhooks`, `/rest/v1/webhookdeliveries` | |
| retention rules | `/rest/v1/retentionrules` | |
| roles and grants | `/rest/v1/roles`, `/rest/v1/grants` | |
| extensions | `/rest/v1/system/extensions` | one document, not a list |

The legacy `GET /rest/v1/targetgroups`, a bare array of names, SHALL stay unchanged.

#### Scenario: Largest group first
- **WHEN** `GET /rest/v1/targetgroupdetails?sort=targets:DESC` is requested
- **THEN** the group with the most targets is first

#### Scenario: Group name with a comma
- **WHEN** targets are listed with `q=group=="eu,west"`
- **THEN** the total equals that group's `targets` count

### Requirement: Software module artifact transfer
`POST /rest/v1/softwaremodules/{id}/artifacts` SHALL accept a multipart upload. When the optional `sha1sum` or `md5sum` is given and does not match the content, the upload SHALL get 400.

`GET .../artifacts/{aid}/download` SHALL support a single Range with 206. It SHALL send an RFC 6266 `Content-Disposition` with both `filename` and `filename*`, so non-ASCII names work.

#### Scenario: Wrong checksum
- **WHEN** an artifact is uploaded with `md5sum=deadbeef` that does not match
- **THEN** the server answers 400 and stores nothing

#### Scenario: Non-ASCII filename
- **WHEN** an artifact with a non-ASCII name such as `firmware-ü.bin` is downloaded through the management API
- **THEN** the response is 200 with an RFC 6266 `filename*` parameter

### Requirement: Implicit tenant bootstrap
The first authenticated management request for a tenant SHALL create it, with its name upper-cased. The new tenant SHALL get:
- software module types `os` and `application`
- distribution set types `os`, `app` and `os_app`
- `os_app` as its default set type

#### Scenario: Fresh tenant
- **WHEN** `GET /rest/v1/userinfo` is the first request of tenant `acme`
- **THEN** tenant `ACME` exists with the default types

### Requirement: Links from forwarded headers
`_links` hrefs in management responses SHALL be absolute. They SHALL be built from `X-Forwarded-Proto` (default `http`) and `Host`.

#### Scenario: Behind a proxy
- **WHEN** a management request arrives with `X-Forwarded-Proto: https`
- **THEN** every `_links` href in the response uses `https`

### Requirement: Wire format for time, ids and enums
Every timestamp on the wire SHALL be epoch milliseconds as a JSON number. Ids of hawkBit-origin resources SHALL be numbers. Ids of hawier-origin resources (webhooks, deliveries, roles, grants, retention rules, action log, incidents, `/ui/v1` rows) SHALL be decimal strings, so that 64-bit snowflakes survive JavaScript.

Enum values SHALL be stored with the exact hawkBit numeric codes. Gaps in those codes, such as rollout status 7 and 8, SHALL be preserved. The management API SHALL render enum values as lower-case names, and DDI `actionHistory` as upper-case names.

#### Scenario: Millisecond timestamps
- **WHEN** any resource with `createdAt` is read
- **THEN** `createdAt` is a number greater than 1.7e12

#### Scenario: Snowflake ids as strings
- **WHEN** a role is created
- **THEN** its `id` is a JSON string of decimal digits

### Requirement: Request body limit
Management request bodies SHALL be limited to `HAWIER_MAX_ARTIFACT_SIZE`, default 1 GiB.

#### Scenario: Oversized upload
- **WHEN** an upload exceeds the configured limit
- **THEN** the server rejects it with an ExceptionInfo error and stores nothing
