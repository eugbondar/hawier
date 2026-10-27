# Spec Delta

## MODIFIED Requirements

### Requirement: Classic rollout strategy module
`hawier-ext-rollout-classic` SHALL implement `rollout.plan` and `rollout.evaluate` for the strategy names "" (empty), `default`, `classic` and `percentage`. Any other name SHALL get an error.

Plan:
- every declared group but the last SHALL get `ceil(total × targetPercentage / 100)` targets (percentage default 100), taken from targets not yet in a group
- the last group SHALL get the rest
- per-group target filters SHALL be ignored, which is hawkBit-classic semantics

Evaluate:
- an empty group SHALL have met its success condition
- a group whose error share reaches the error threshold, with at least one error, SHALL be errored and SHALL pause the rollout
- a group whose finished share reaches the success threshold SHALL have met its success condition

The module SHALL NOT judge a group finished: meeting the success condition starts the next group, and whether a group is finished is the engine's observation of its actions, as in hawkBit.

#### Scenario: Even split loses nobody
- **WHEN** a rollout over 11 targets declares two 50% groups
- **THEN** the groups hold 6 and 5 targets

#### Scenario: Threshold met with stragglers
- **WHEN** a group of 10 with a 50% success threshold has 5 finished and 5 running actions
- **THEN** the verdict says the group met its success condition, and neither errored nor paused
