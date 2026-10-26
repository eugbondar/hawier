# Spec Delta

## Purpose

Object pages, create and edit forms, server-backed pickers, and the per-object
panels through which operators act on single entities in the console.

## ADDED Requirements

### Requirement: Object page
`/{resource}/{id}` SHALL show:
- every field of the object except links, with time fields in full
- Edit, when the resource has a form
- Delete, with confirmation

Security and gateway tokens SHALL be masked until revealed.

#### Scenario: Token masked
- **WHEN** a target page is opened
- **THEN** the security token is not printed in clear text until the operator reveals it

### Requirement: Per-resource panels
Object pages SHALL offer these panels:
- target: metadata (paged), group, attributes, assign a set, tags, type, auto-confirm, recent actions
- distribution set: assigned and installed counts, each linking to the list query that counted it; metadata; modules with artifacts; attach a module; new rollout; tags
- software module: artifacts (upload up to 1 GiB, download, delete) and metadata
- types: their type relations
- target filter: auto-assignment
- incident: number, time, node, sequence, related log entries
- rollout: progress and deploy groups, with lifecycle buttons for the current state

#### Scenario: Counts agree with lists
- **WHEN** a distribution set page shows N assigned targets
- **THEN** its link opens a target list with total N

### Requirement: Rollout group and action pages
`/rollouts/{id}/groups/{gid}` SHALL page the group's actions. `/targets/{cid}/actions/{aid}` SHALL show the action's status, type, distribution set link and status history. A repeat counter `×N` SHALL be shown only when N is greater than 1. The action page SHALL offer cancel and force-cancel.

#### Scenario: Repeated status
- **WHEN** a device reported the same status 5 times
- **THEN** the history shows that row once with `×5`

### Requirement: Forms
`/{resource}/new` and `/{resource}/{id}/edit` SHALL render the resource's declared form. Each field SHALL carry a hint taken from the manual, and required fields SHALL be validated. After a successful create, the console SHALL go to the `next` target when one is given; `{id}` in it is replaced with the new id.

#### Scenario: Create form renders
- **WHEN** the create form of each creatable resource is opened
- **THEN** it renders without error

### Requirement: Pickers for server-populated choices
Every choice populated from the server SHALL be a typeahead picker backed by `GET /pick/{resource}?q=`. Examples are distribution sets, tags, modules and types.
- results are sorted by name, at most 20 rows
- when more exist: `Keep typing to narrow further`
- when none: `No matches`

A `<select>` SHALL be used only for a short fixed enumeration. Editing a normal picker's text SHALL clear its chosen value. A free picker SHALL suggest existing values but submit the typed text. The target group is an example.

#### Scenario: Free group picker
- **WHEN** an operator types a new group path that does not exist yet
- **THEN** the form submits that path as the group

#### Scenario: Only short enumerations are dropdowns
- **WHEN** any console page is rendered
- **THEN** every `<select>` offers at most 8 fixed options
