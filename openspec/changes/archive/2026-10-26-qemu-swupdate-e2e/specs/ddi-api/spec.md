# Spec Delta

## ADDED Requirements

### Requirement: When the server asks for attributes
The server SHALL ask a device for its controller attributes, by offering the
`configData` link in the poll, in these cases, as hawkBit 1.0 does:
- a target registered through plug-and-play
- a target created through `POST /rest/v1/targets`
- a target whose action has just ended FINISHED, or DOWNLOADED for a
  download-only action
- a target whose `requestAttributes` an operator has set through the
  management API

A successful `PUT configData` SHALL clear the request.

#### Scenario: Provisioned device reports on its first poll
- **WHEN** a target created through the management API polls for the first time
- **THEN** the poll offers `configData`

#### Scenario: Asked again after an update
- **WHEN** a device that had already reported its attributes finishes an update
- **THEN** its next poll offers `configData` again
