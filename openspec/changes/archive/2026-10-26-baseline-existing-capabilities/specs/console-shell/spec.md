# Spec Delta

## Purpose

The frame of the hawier web console (`hawier-ui`): sign-in and session, the
top bar and brand, the sidebar and its collapsed state, the overview page,
health, and the console's configuration.

## ADDED Requirements

### Requirement: Console holds no state of its own
The console SHALL be a server-side rendered application. It SHALL read and write state only through the server's management API, as the signed-in user, and the server's `/ui/v1` API, using the internal token.

It SHALL be configured by:
- `HAWKBIT_MGMT_URL` (default `http://localhost:8080`)
- `HAWIER_API_URL` (defaults to the management URL)
- `HAWIER_INTERNAL_TOKEN`
- `HAWIER_BIND_ADDR` (default `0.0.0.0:8088`)
- `HAWIER_STATIC_DIR` (default `static`)

#### Scenario: Restarted console
- **WHEN** the console process is restarted
- **THEN** saved views, recent searches and sessions are unchanged

### Requirement: Sign-in and session
`/login` SHALL take a username, a password and a tenant, defaulting to `DEFAULT`. It SHALL verify them against the management API. On success it SHALL answer 303 to `/` and set a cookie `hawier_session` holding only an opaque random token, with `HttpOnly`, `SameSite=Lax` and an 8-hour lifetime.

A bad sign-in SHALL return to `/login` with an error. An unauthenticated page request SHALL redirect to `/login`. An unauthenticated fragment request SHALL get 401; fragments are live search, pickers and the help panel. Every upstream management call SHALL carry the user's credentials and mark itself as console-originated.

#### Scenario: Successful sign-in
- **WHEN** valid credentials are posted to `/login`
- **THEN** the response is 303 to `/` with a `hawier_session` cookie

#### Scenario: Not signed in
- **WHEN** an anonymous browser opens `/targets`
- **THEN** it is redirected to `/login`

### Requirement: Top bar and brand
Every page SHALL show a top bar. At its left is the sidebar toggle, followed by the word `Hawier`, with no badge or logo. It also holds the "what is this?" control and global search. The interface SHALL NOT show the strings `hawkBit` or `hawier-ui`.

#### Scenario: Branding
- **WHEN** any console page is rendered
- **THEN** the top bar contains the brand `Hawier` and the page contains neither `hawkBit` nor `hawier-ui`

### Requirement: Sidebar navigation
The sidebar SHALL list:
- Overview
- the groups Inventory, Deployment, Repository and Admin, with Admin holding the system settings
- each resource with its system views, followed by the user's saved views
- Help
- a footer with tenant, user and sign-out

Every resource SHALL have a distinct icon. Controls SHALL be SVG icons, not text glyphs. Navigation SHALL be the sidebar and the page title; there SHALL be no "Back" buttons.

#### Scenario: Distinct icons
- **WHEN** the sidebar is rendered
- **THEN** no two resources share an icon

### Requirement: Collapsed sidebar
The toggle SHALL collapse the sidebar to an icon rail, and the choice SHALL persist per browser. In the collapsed state, anything that cannot be shown in the rail, such as group headers, SHALL be hidden entirely. No empty clickable strips SHALL remain.

#### Scenario: Collapse persists
- **WHEN** a user collapses the sidebar and opens another page
- **THEN** the sidebar is still collapsed and shows only icons

### Requirement: Overview page
`/` SHALL show five target status cards: in sync, pending, error, registered and unknown. Each card SHALL link to `/targets?q=updatestatus==<status>`, and its number SHALL be counted by that same query. The page SHALL also show per-resource counts, active rollouts and important incidents. A counter that cannot be loaded SHALL show `—`, never 0.

#### Scenario: Numbers converge
- **WHEN** the overview shows N pending targets
- **THEN** following that card lists exactly N targets

### Requirement: Console health
`GET /api/health` SHALL answer without authentication, with `{ok: true, service, build, hawkbit_mgmt_url}`.

#### Scenario: Probe
- **WHEN** a deploy probe calls `/api/health`
- **THEN** it receives 200 with the console's build stamp
