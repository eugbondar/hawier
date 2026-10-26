# Spec Delta

## ADDED Requirements

### Requirement: The repository is English-only
`make lint` SHALL fail when any file in the working tree that git tracks or would track (not ignored) contains a character from the Cyrillic block (U+0400–U+04FF), and SHALL print the offending lines. Specs, documentation, code comments and test data are all covered; input that has to be non-ASCII is written with escapes in code.

#### Scenario: Cyrillic in a comment
- **WHEN** a tracked source file gains a comment with Cyrillic text
- **THEN** `make lint` fails and names the file and line

#### Scenario: Clean tree
- **WHEN** no tracked file contains Cyrillic
- **THEN** this check passes and `make lint` continues with its other checks
