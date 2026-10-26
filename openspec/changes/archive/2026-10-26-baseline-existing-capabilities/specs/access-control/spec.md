# Spec Delta

## Purpose

Who may call which API. Covers management Basic users, the console's internal
bearer token, per-request permission classification, role and grant
management, and the authorization decision delegated to the authz extension.
Device authentication is in `ddi-api`.

## ADDED Requirements

### Requirement: Management Basic authentication
The management API SHALL authenticate HTTP Basic credentials against `HAWIER_MGMT_USERS`.
- The format is a comma-separated list of `user:password[:tenant]`. The tenant defaults to `DEFAULT`, and the whole variable defaults to `admin:admin:DEFAULT`.
- At least one user SHALL be configured.
- Credentials SHALL be compared in constant time across all configured users.
- A missing header, another scheme, bad encoding or a wrong password SHALL get 401 with `errorCode` `hawkbit.server.error.unauthorized` and `WWW-Authenticate: Basic realm="hawier"`.
- A non-empty `X-Hawkbit-Tenant` header SHALL select the tenant, upper-cased.

#### Scenario: Wrong password
- **WHEN** a management request carries a wrong password
- **THEN** the server answers 401 with an ExceptionInfo body

### Requirement: Internal API bearer token
`/ui/v1` SHALL require `Authorization: Bearer <HAWIER_INTERNAL_TOKEN>`, compared in constant time. The default token is `dev-internal-token`. A missing or wrong token SHALL get 401.

#### Scenario: No token
- **WHEN** `/ui/v1/views` is requested without a bearer token
- **THEN** the server answers 401

### Requirement: Permission per request
Every authenticated `/rest/v1` request SHALL be classified to one hawkBit-style permission by method and path:
- `userinfo` SHALL need no permission.
- System resources SHALL need READ_SYSTEM to read and UPDATE_SYSTEM to write. These are webhooks, webhook deliveries, retention rules, roles, grants, system configuration, extensions, the action log and incidents.
- Rollouts SHALL need READ_ROLLOUT to read, CREATE_ROLLOUT to create, DELETE_ROLLOUT to delete, HANDLE_ROLLOUT for lifecycle actions, and UPDATE_ROLLOUT otherwise.
- Targets, target filters and target groups SHALL need READ, CREATE, UPDATE or DELETE `_TARGET`.
- Everything else, including any unclassified path, SHALL need the matching `_REPOSITORY` permission. No endpoint is open by omission.

#### Scenario: Running is not creating
- **WHEN** a user holding only HANDLE_ROLLOUT starts a rollout and then tries to create one
- **THEN** the start succeeds and the create gets 403

#### Scenario: Unclassified endpoint is guarded
- **WHEN** a user without repository permissions calls an endpoint that has no explicit classification
- **THEN** the request is denied

### Requirement: Enforcement begins with the first grant
A tenant with no grants SHALL be unconfigured, and every authenticated request in it SHALL be allowed. Defining a role without granting it SHALL NOT start enforcement. Once any grant exists, every request SHALL be decided by the authz extension. A refusal SHALL be 403 with `errorCode` `hawkbit.server.error.forbidden` and the extension's reason as the message. Without an authz extension, every authenticated request SHALL be allowed.

#### Scenario: First grant switches enforcement on
- **WHEN** the first role is granted to user `alice` in a tenant
- **THEN** user `admin`, who holds no grant, gets 403 on requests that need a permission

#### Scenario: ROLE_ALL
- **WHEN** a user holds a role containing ROLE_ALL
- **THEN** every request is allowed

### Requirement: Roles and grants
`/rest/v1/roles` SHALL manage named roles, each a set of permissions from the known set. An unknown or empty permission set SHALL get 400, and a duplicate name SHALL get 409. `/rest/v1/grants` SHALL bind a username to a role, and a duplicate binding SHALL get 409.

Any write that would leave a tenant with grants but no holder of UPDATE_SYSTEM or ROLE_ALL SHALL be rolled back with 400. Every role or grant change SHALL take effect immediately on the node that made it. Cached verdicts SHALL expire within 5 seconds elsewhere.

#### Scenario: No lock-out
- **WHEN** an operator deletes the last grant that carries UPDATE_SYSTEM while other grants remain
- **THEN** the server answers 400 and the grant remains

### Requirement: Userinfo tells the truth
`GET /rest/v1/userinfo` SHALL always answer for an authenticated user with `{tenant, username, permissions[]}`. `permissions` SHALL be the user's granted permissions. It SHALL be `["ROLE_ALL"]` when the tenant is unconfigured or no authz extension is configured.

#### Scenario: Reader
- **WHEN** a user whose only role grants READ_TARGET calls userinfo
- **THEN** `permissions` is `["READ_TARGET"]`
