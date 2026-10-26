# Spec Delta

## ADDED Requirements

### Requirement: A refused assignment is reported on the page
When the server refuses an assignment made from a target page's assign
panel, the console SHALL show `Assign stopped: <server message>` as a danger
message in the page frame. The message includes the incident number. The
console SHALL NOT return to the target page as if the assignment had
succeeded. The refusal SHALL also be recorded in the action log, as every
operation is.

#### Scenario: Incompatible set from the target page
- **WHEN** an operator assigns a set whose type the target's type does not accept
- **THEN** the page shows `Assign stopped: Target of type … is not compatible …` as a danger message, and the action log has the same error
