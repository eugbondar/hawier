# console-bulk-actions Specification

## Purpose

Selecting rows in any console list and applying a declared operation to all of
them, with visible selection and explicit confirmation of destructive
operations.

## Requirements

### Requirement: Row selection
Every list SHALL offer a selection checkbox per row and a select-all checkbox. Ctrl-click SHALL toggle a row, and Shift-click SHALL select a range. The selection cell SHALL be a full-cell label, so a click beside the checkbox selects the row instead of opening the object. Selected rows SHALL be visibly marked.

#### Scenario: Click beside the checkbox
- **WHEN** an operator clicks the empty area of a row's selection cell
- **THEN** the row becomes selected and highlighted and no navigation happens

### Requirement: Declared bulk operations
The bulk operations of each resource SHALL be declared with the resource, and SHALL be offered in a right-click menu and in a toolbar with a selection counter. They are:
- targets: assign set, tag, untag, set group, clear group, cancel actions, delete
- rollouts: start, pause, resume, stop, delete
- distribution sets: tag, untag, invalidate, delete
- target filters, software modules, roles, grants, retention rules, webhooks, tags and types: delete

#### Scenario: Context menu
- **WHEN** an operator selects three targets and right-clicks
- **THEN** a menu with the target bulk operations appears

### Requirement: Destructive operations confirm with a count
A destructive bulk operation SHALL ask for confirmation, stating the number of selected rows. For example: `Delete 3 row(s)? This cannot be undone.` The confirmation SHALL be determined by the button that was pressed.

#### Scenario: Delete three
- **WHEN** an operator chooses Delete with three rows selected
- **THEN** a confirmation naming 3 rows is shown before anything is sent

### Requirement: Bulk result report
After a bulk operation the console SHALL report `<operation>: <done> of <n> done.`. It SHALL report `<operation> stopped: …` if none succeeded. Each bulk operation SHALL be recorded in the action log.

#### Scenario: Partial success
- **WHEN** a bulk delete succeeds for 2 of 3 rows
- **THEN** the console reports `2 of 3 done`
