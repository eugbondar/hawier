# Target filters

> A saved question about devices — and, optionally, a standing rule attached to it.

## Target filters {#target-filters}

A filter is a named query over the target list. Two uses:

- **As an aim.** A rollout takes a query; keeping the good ones named and saved beats retyping them.
- **As a rule.** A filter with a distribution set attached hands that set to every device that matches, from now on. See [auto-assignment](/help/deployment/auto-assignment#auto-assignment).

## The query {#query}

Written in the query language — see [the query reference](/help/reference/rsql#the-query-language). It is checked when you save it, so a filter that would fail is refused at creation rather than at the moment a rollout depends on it.

Test it on the [targets](/targets) list first. The row count there is exactly the number of devices the filter selects.

## Filters versus tags {#versus-tags}

A tag is a label you put on devices. A filter is a question asked of them. `tag==canary` names devices somebody chose; `updateStatus==error;lastControllerRequestAt=gt=...` names devices that got into a state.

Use tags for deliberate sets, filters for conditions.

## All {#all}

Every saved filter.

## Auto-assignment {#auto-assignment}

Filters that are handing out a distribution set automatically. These are standing rules: worth reviewing, because they act without anyone pressing anything.

## Recently used {#recently-used}

Filters touched most recently.

## Without auto-assignment {#without-auto-assignment}

Filters that only name devices, with no rule attached.
