# console-search Specification

## Purpose

Global search across all console resources: a results page, a live
suggestion dropdown and per-user recent searches, all answered by the server
and never by filtering a loaded page.

## Requirements

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

### Requirement: Live search
`/search/live?q=` SHALL return suggestions:
- for an empty query: the 8 most recent searches
- for fewer than 2 characters: nothing
- otherwise: up to 8 matches per resource and 5 log entries

Ctrl/Cmd+K SHALL focus the search box.

#### Scenario: Recent searches offered
- **WHEN** a user has searched for `fleet` and focuses the empty search box
- **THEN** `fleet` is offered

### Requirement: Recent search history
Each user SHALL be able to remove a single recent search or clear the whole history.

#### Scenario: Clear history
- **WHEN** a user clears their search history
- **THEN** the empty search box offers no recent searches
