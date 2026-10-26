# extension-host Specification

## Purpose

The mechanism by which a hawier node delegates decisions to sandboxed
WebAssembly modules. It covers the extension slots, the ABI, guest capabilities,
deadlines, parameters, declared refusals when a module is absent, and the
extensions report endpoint.

## Requirements

### Requirement: Extensions are wasm modules in fixed slots
An extension SHALL be a WebAssembly module (wasm32-wasip1) loaded from a file path. There SHALL be no in-process or built-in implementation of any extension point, and no other transport. The node SHALL have six slots, each configured by one variable and holding at most one module:

| Slot | Variable | Points | Site |
|---|---|---|---|
| storage | `HAWIER_EXT_STORAGE` | `storage.locate` | device path and management path, on every node |
| event sink | `HAWIER_EXT_EVENT_SINK` | `event_sink.plan` | control loop |
| retention | `HAWIER_EXT_RETENTION` | `retention.select` | control loop |
| rollout | `HAWIER_EXT_ROLLOUT` | `rollout.plan`, `rollout.evaluate` | control loop |
| console | `HAWIER_EXT_CONSOLE` | `console.surfaces` | request (`/ui/v1/surfaces`) |
| authz | `HAWIER_EXT_AUTHZ` | `authz.decide` | request (management middleware) |

An empty or blank variable SHALL mean the slot is unconfigured.

#### Scenario: Every point has one slot
- **WHEN** the set of declared points is enumerated
- **THEN** each point belongs to exactly one slot

### Requirement: Decisions and descriptors cross the boundary, never bytes or handles
Every call SHALL exchange only serialized data. The host sends a snapshot. The module returns a decision or a descriptor, such as a URL, a path, a delivery plan, a group plan, a verdict, or a list of ids. The host SHALL perform all IO, transactions and locks, and SHALL apply the decision in its own transaction.

A module SHALL never receive artifact bytes, database connections, or other handles. It SHALL never sit inside a database lock.

#### Scenario: Storage returns a descriptor
- **WHEN** a download needs artifact content
- **THEN** the storage module returns a location and the host moves the bytes or redirects; the module never sees the bytes

### Requirement: Module loading fails loudly at startup
At startup the node SHALL load each configured module, instantiate it once and read its manifest. Startup SHALL fail with an error naming the module and the point when either of these holds:
- a configured path cannot be loaded
- the manifest does not declare the slot's point

#### Scenario: Missing module file
- **WHEN** `HAWIER_EXT_STORAGE=/nonexistent/x.wasm`
- **THEN** the node does not start and the error mentions the wasm extension

#### Scenario: Module in the wrong slot
- **WHEN** the webhook module is configured as the storage extension
- **THEN** startup fails naming the module and `storage.locate`

### Requirement: Declared refusal when a slot is empty
An unconfigured slot SHALL produce a declared refusal. The host SHALL NOT do the module's work itself.

| Slot | Behavior when empty |
|---|---|
| storage | artifact download and upload answer 503 with Retry-After |
| event sink | events are not delivered |
| retention | rules do not delete anything, and their last result records the failure |
| rollout | rollouts are not split into groups; a create naming an explicit strategy is rejected with 400 |
| console | the console shows only its built-in tree |
| authz | every authenticated management request is allowed, and `userinfo` reports `ROLE_ALL` |

#### Scenario: Retention without a module
- **WHEN** an armed retention rule comes due on a node without a retention module
- **THEN** nothing is deleted and the rule's last result says it failed

#### Scenario: Authorization without a module
- **WHEN** no authz module is configured
- **THEN** any authenticated management user may perform any management request

### Requirement: Guest sandbox
A guest SHALL get:
- no preopened directories
- no sockets
- no stdin or stdout
- an environment containing only the effective `HAWIER_EXT_*` parameters, minus the reserved names

It SHALL NOT receive node secrets such as the database URL or the internal token. Its only non-WASI import SHALL be the host-performed HTTP call. That call SHALL refuse unless the manifest declares the `http_out` capability with the target host. Host matching SHALL be exact. The call SHALL allow only http and https, cap request and response bodies at 1 MiB, and time out after 10 s.

#### Scenario: Undeclared outbound call
- **WHEN** a module without the `http_out` capability attempts an HTTP call
- **THEN** the host refuses it

#### Scenario: Declared signer host
- **WHEN** the S3 module is configured with a signer URL
- **THEN** its manifest declares that host and the host performs the POST for it

### Requirement: Deadline on every call
Every guest call, including the startup manifest read, SHALL be bounded by `HAWIER_EXT_DEADLINE_MS` (default 2000). The host SHALL enforce it the same way for every module, regardless of the deadline the module declares. In each of these cases the instance SHALL be discarded and the caller SHALL get a service-unavailable error (503 on request paths):
- a call exceeds the deadline
- a call traps
- a call returns an empty or oversized (over 8 MiB) answer

Each module SHALL have at most 32 concurrent calls and at most 64 MiB of linear memory.

#### Scenario: Runaway module
- **WHEN** a module call does not return within the deadline
- **THEN** the call is aborted and the request answers 503 rather than hanging

### Requirement: Guest ABI
A module SHALL export `memory`, `hawier_alloc(len) -> ptr`, `hawier_manifest() -> packed` and `hawier_call(ptr, len) -> packed`. `packed` is `(ptr << 32) | len` and points at JSON. Requests SHALL be `{point, payload}`. Responses SHALL be `{ok:{payload}}` or `{err:{message}}`. A module SHALL answer an unknown point or an undecodable payload with `err`.

#### Scenario: Unknown point
- **WHEN** a module is called with a point it does not implement
- **THEN** it answers `err` and the host does not crash

### Requirement: Extension parameters
The effective parameters SHALL be the process environment's `HAWIER_EXT_*` variables plus the `KEY=value` file named by `HAWIER_EXT_PARAMS_FILE`. On a name conflict the environment SHALL win, and the origin of each value SHALL be recorded.

File format:
- the first `=` splits name from value
- `#` starts a comment only at line start
- `"..."` quoting is allowed, with `\n \t \\ \"` escapes
- names SHALL match `HAWIER_EXT_[A-Z0-9_]+`

Startup:
- a malformed file, a duplicate name or a reserved name SHALL stop startup, naming the lines involved
- a missing or unreadable file SHALL NOT stop startup

These names are reserved: `HAWIER_EXT_{STORAGE,EVENT_SINK,RETENTION,ROLLOUT,CONSOLE,AUTHZ,DEADLINE_MS,PARAMS_FILE}`. They SHALL NOT be settable from the file or the API (400), and SHALL NOT be passed to guests. Choosing a module is a trust decision, not a parameter.

Parameter changes SHALL take effect only after a node restart.

#### Scenario: Environment pins a value
- **WHEN** a parameter is set both in the environment and in the file
- **THEN** the environment value is effective, the parameter reports origin `environment`, and an API write to it answers 409

#### Scenario: Duplicate in the file
- **WHEN** the parameters file defines the same name twice
- **THEN** the node refuses to start and names both lines

### Requirement: Parameter management API
`GET /rest/v1/system/extensions` SHALL list the parameters. Each entry SHALL have `name`, `value`, `secret`, `set`, `origin`, `inFile`, `editable` and `changedSinceStart`. Reserved names SHALL be excluded.

`PUT` and `DELETE` on `/rest/v1/system/extensions/parameters/{name}` SHALL edit the file:
- the file is rewritten atomically, with mode 0600, preserving comments and order
- a name outside the prefix or a reserved name gets 400
- with no params file configured, PUT gets 400
- a file that cannot be written gets 500, and GET keeps working

A name containing SECRET, PASSWORD, TOKEN or CREDENTIAL, or ending in `_KEY`, SHALL be a secret:
- its value SHALL never be returned (`value: null, set: true`)
- an empty PUT SHALL keep the stored value
- DELETE SHALL clear it

Reading SHALL require READ_SYSTEM and writing UPDATE_SYSTEM. On a sub node, writes SHALL get 405.

#### Scenario: Secret is write-only
- **WHEN** `HAWIER_EXT_S3_SECRET_KEY` is set through the API and read back
- **THEN** the response has `value: null`, `secret: true` and `set: true`

#### Scenario: Drift after edit
- **WHEN** a parameter is changed through the API
- **THEN** the report shows it `changedSinceStart` and the running modules keep the startup value until restart

### Requirement: Extensions report endpoint
`GET /rest/v1/system/extensions` SHALL report:
- `node`: `id`, `role` (control or sub), `build`, `profile` (debug or release), `enforcedDeadlineMs`, `paramsFile`, `paramsFileState` (one of unconfigured, absent, ok, unreadable, malformed), `paramsFileWritable` (probed by an actual write), and `writable`
- `points[]`, one entry per slot, including unconfigured slots: `key`, `label`, `envVar`, `points`, `configured`, `absence` (what happens without the slot), and `module`. `module` SHALL be null or have `name`, `version`, `points`, `sites`, `capabilities`, `declaredDeadlineMs` and `path`
- `parameters[]`
- `drift`

#### Scenario: Unconfigured slots are listed
- **WHEN** a node runs with only a storage module and the report is read
- **THEN** all six slots are listed, the rollout slot shows two points, and the empty slots describe their absence
