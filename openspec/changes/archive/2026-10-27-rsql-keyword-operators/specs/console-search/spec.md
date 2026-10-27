## MODIFIED Requirements

### Requirement: Global search runs on the server
`/search?q=` SHALL query every resource on the server, ORing `field=="*needle*"` over that resource's search fields, plus the action log. The search fields are:
- targets: controller id, name, description
- actions: controller id
- incidents: path, message, error code

The console SHALL neutralise RSQL metacharacters in the needle and SHALL quote it before building the query, so that whitespace and the words `and` and `or` in the needle are matched as text. The query SHALL be remembered as a recent search.

#### Scenario: Needle beyond the first page
- **WHEN** the only matching target sits behind 120 non-matching ones
- **THEN** the search still finds it, and a substring of its id also finds it

#### Scenario: Needle with spaces and a keyword
- **WHEN** a target is named `north and south` and the needle is `h and s`
- **THEN** the search finds that target
