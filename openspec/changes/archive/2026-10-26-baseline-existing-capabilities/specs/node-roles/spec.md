# Spec Delta

## Purpose

Defines the two node roles of a hawier installation, what each role serves and
runs, and the guarantee that devices keep updating through sub nodes when the
control node is down.

## ADDED Requirements

### Requirement: Two roles from one binary
The `hawier` binary SHALL run as a control node by default. It SHALL run as a sub node exactly when `HAWIER_ROLE=sub`. Any other value SHALL mean control. A single control node SHALL be a complete installation.

#### Scenario: Default role
- **WHEN** the node starts without `HAWIER_ROLE`
- **THEN** it reports `node.role` = `control`

### Requirement: Control node serves everything and runs the engines
A control node SHALL serve:
- the full management API
- DDI
- `/ui/v1`

It SHALL also run the background loops:
- the rollout engine, including auto-assignment
- housekeeping, including retention
- event outbox delivery

#### Scenario: Engines only on control
- **WHEN** a rollout is started on an installation with one control node and one sub node
- **THEN** its groups are advanced by the control node and never by the sub node

### Requirement: Sub node is DDI plus read-only management
A sub node SHALL serve DDI, artifact delivery, `/ui/v1`, and the management API for reading. A management request with a method other than GET, HEAD or OPTIONS SHALL get 405 with `errorCode` `hawkbit.server.error.readOnlyNode`. A sub node SHALL NOT run background loops. Events written on a sub node SHALL be delivered later by the control node.

#### Scenario: Write on a sub node
- **WHEN** a client POSTs to `/rest/v1/targets` on a sub node
- **THEN** it receives 405 with errorCode `hawkbit.server.error.readOnlyNode`

### Requirement: Devices survive a dead control node
Poll, deployment base, download and feedback SHALL depend only on request-scoped database work. They SHALL NOT depend on singletons or background loops, so a sub node serves devices with the control node stopped.

While the control node is down, only rollout progression and auto-assignment SHALL pause. They SHALL resume where they stopped when it returns. All nodes SHALL share one PostgreSQL database and hold no state of their own.

#### Scenario: Failover
- **WHEN** the control node is killed and a device with an assigned set cycles against a sub node
- **THEN** the device completes the update, the action is FINISHED, and management GET on the sub node still works

### Requirement: Device traffic is isolated from management traffic
DDI SHALL use its own database pool (`HAWIER_DDI_POOL_SIZE`, default 20), separate from the management/UI pool (`HAWIER_MGMT_POOL_SIZE`, default 10). Bursts of management traffic SHALL NOT starve devices of connections.

#### Scenario: Management burst
- **WHEN** the management pool is exhausted
- **THEN** DDI requests still obtain connections from their own pool
