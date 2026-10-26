# Spec Delta

## ADDED Requirements

### Requirement: Rollouts contain only compatible targets
A rollout SHALL count, place and assign only those filter-matching targets
that may receive its set under the target type compatibility rules of
assignment. This includes the typed-targets-only rule when
`target.type.required` is on. The rule SHALL cover:
- `totalTargets`
- the snapshot given to the strategy extension
- group membership
- dynamic groups

A matching target that is not compatible SHALL be left out silently. It
SHALL NOT fail the rollout.

#### Scenario: Mixed fleet
- **WHEN** a rollout of a set of type `os-rev1` has a filter matching one `rev1`-typed and one `rev2`-typed target
- **THEN** after materialization `totalTargets` is 1, and only the `rev1` target is in a group

#### Scenario: Late incompatible device
- **WHEN** a `rev2`-typed target matching a dynamic rollout of an `os-rev1` set registers
- **THEN** no dynamic group is created for it

## MODIFIED Requirements

### Requirement: Target filter auto-assignment
On the engine loop, every target filter with an auto-assign set SHALL assign
that set to each matching target that meets two conditions:
- the target's assigned and installed sets both differ from it
- the target may receive it under the target type compatibility rules of
  assignment

An incompatible target SHALL be skipped without an error. It SHALL NOT stop
the assignment to the others. The assignment SHALL use the filter's action
type (default FORCED), its weight, its confirmation flag, and the operator who
configured it as initiator. Running again SHALL NOT create duplicate actions.

#### Scenario: Matching device is picked up
- **WHEN** a filter `attribute.hw==rev2` auto-assigns set S and a device reports `hw=rev2`
- **THEN** that device gets an action for S on the next tick, and a second tick creates none

#### Scenario: Incompatible device is skipped
- **WHEN** a filter auto-assigns a set of type `os-rev1` and it matches a `rev1`-typed and a `rev2`-typed target
- **THEN** only the `rev1` target gets an action, and the tick completes without an error
