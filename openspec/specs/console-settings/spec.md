# console-settings Specification

## Purpose

The console's system settings page: who and where you are, which builds are
running, which extensions the node has loaded, editing extension parameters,
and editing tenant configuration.

## Requirements

### Requirement: Context and builds
`/settings` SHALL show:
- tenant, user, the management URL and the state-API URL
- the console build stamp next to the server build stamp, with a `debug` pill for a debug build of either
- the node id and role

#### Scenario: Two stamps
- **WHEN** `/settings` is opened
- **THEN** exactly two `vYY.MM.DD-<sha>` stamps are shown, one for the console and one for the server

### Requirement: Extensions panel
The settings page SHALL list every extension slot of the node. A loaded slot SHALL show its module, version, sites, capabilities and variable, and its declared deadline next to the deadline the node enforces. An empty slot SHALL state what happens without it. If the node refuses the extensions report, for example for lack of READ_SYSTEM, the panel SHALL show the reason and the rest of the page SHALL be unaffected.

#### Scenario: Refused report
- **WHEN** the signed-in user lacks READ_SYSTEM
- **THEN** the extensions panel shows the refusal and the tenant configuration panel still renders

### Requirement: Extension parameters
The settings page SHALL list the extension parameters. When the node's parameter file is writable, the page SHALL allow adding, changing and removing them. Secret parameters SHALL be write-only password fields that never contain the stored value. The console SHALL record parameter changes in the action log without their values.

#### Scenario: Secret never reaches the page
- **WHEN** a secret parameter has been set and `/settings` is opened
- **THEN** its value appears nowhere in the HTML

### Requirement: Tenant configuration editing
The settings page SHALL show every tenant configuration key with its value and whether it is a default. A key SHALL be saved individually, and SHALL be reset to its default after confirmation. Each change SHALL be recorded in the action log.

#### Scenario: Toggle multi-assignment
- **WHEN** an operator enables `multi.assignments.enabled` on `/settings`
- **THEN** the server reports the key as `true` and the change appears in the action log
