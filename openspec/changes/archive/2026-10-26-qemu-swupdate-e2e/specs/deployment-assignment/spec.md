# Spec Delta

## ADDED Requirements

### Requirement: Target type compatibility on assignment
An assignment SHALL be refused when the target has a target type and that
type does not list the set's distribution set type as compatible. The refusal
SHALL be 400 with errorCode `hawkbit.server.error.target.type.incompatible`.
Its message SHALL name the incompatible target types and the set's type, in
hawkBit's wording: `Targets of types [<types>] are not compatible with
distribution set of type <type>`. This matches hawkBit 1.0.

A target without a target type SHALL accept any set, unless
`target.type.required` is on.

The check SHALL apply to `POST /rest/v1/targets/{controllerId}/assignedDS`
(one target, one or more sets) and to
`POST /rest/v1/distributionsets/{id}/assignedTargets` (one set, one or more
targets). If any target in
one request is incompatible, the whole request SHALL be refused: no action is
created, no set is locked, and no event is emitted for any target in it.

#### Scenario: Incompatible type
- **WHEN** a set of type `os-rev2` is assigned to a target whose type lists only `os-rev1` as compatible
- **THEN** the server answers 400 with errorCode `hawkbit.server.error.target.type.incompatible`, and the target has no new action

#### Scenario: One bad target spoils the batch
- **WHEN** a set is assigned to two targets in one request and one of them is incompatible
- **THEN** the server answers 400 and neither target has a new action

#### Scenario: Untyped target, default configuration
- **WHEN** a set is assigned to a target without a target type while `target.type.required` is off
- **THEN** the assignment succeeds

### Requirement: Typed targets only
When the tenant key `target.type.required` is on, an assignment to a target
without a target type SHALL be refused with 400
`hawkbit.server.error.target.type.incompatible`. The message SHALL say that
the target has no type. This is a hawier addition and part of the hardened
profile. With the key off, behaviour SHALL be exactly as in hawkBit.

#### Scenario: Hardened tenant refuses an untyped target
- **WHEN** `target.type.required` is on and a set is assigned to a target without a target type
- **THEN** the server answers 400 and the target has no new action
