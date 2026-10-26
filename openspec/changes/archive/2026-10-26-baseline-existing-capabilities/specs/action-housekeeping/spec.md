# Spec Delta

## Purpose

The control node's periodic housekeeping: expiry of console sessions,
hawkBit-compatible automatic action cleanup, and the trigger for retention.

## ADDED Requirements

### Requirement: Housekeeping loop
The control node SHALL run housekeeping every `HAWIER_HOUSEKEEPING_INTERVAL_MS` (default 60000). Each tick SHALL, in order:
1. Delete expired console sessions.
2. Evaluate retention rules.
3. Run action cleanup per tenant.

Sub nodes SHALL NOT run housekeeping.

#### Scenario: Interval from the environment
- **WHEN** the control node starts with `HAWIER_HOUSEKEEPING_INTERVAL_MS=5000`
- **THEN** housekeeping ticks about every 5 seconds

### Requirement: Expired sessions are removed
Housekeeping SHALL delete every console session whose expiry time has passed.

#### Scenario: Expired session swept
- **WHEN** a session's expiry lies in the past and housekeeping ticks
- **THEN** the session no longer exists and its token no longer authenticates

### Requirement: Automatic action cleanup
For each tenant, housekeeping SHALL delete actions whose status is listed in `action.cleanup.auto.status` and whose last modification is older than `action.cleanup.auto.expiry`. Each action SHALL be deleted together with its status rows and messages, in one transaction.

`action.cleanup.auto.status` holds status names separated by `,` or `;`. `action.cleanup.auto.expiry` is in milliseconds. Cleanup SHALL be off unless the expiry is set to a non-negative value and the status list is non-empty.

#### Scenario: Nothing configured
- **WHEN** neither cleanup key is set
- **THEN** an old CANCELED action survives housekeeping

#### Scenario: Cleanup configured
- **WHEN** expiry is 1000 and status is `CANCELED,ERROR`
- **THEN** a CANCELED action last modified a minute ago is deleted, and a fresh RUNNING action survives
