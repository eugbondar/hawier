# Spec Delta

## Purpose

The console's single list mechanism. Every entity list is a declared resource
with columns, system views and user-saved views. Every list sorts, searches
and pages on the server, shows its counter and pager first, and keeps its frame
when the query fails.

## ADDED Requirements

### Requirement: Declared resources
Every console list SHALL be a declared resource with:
- an endpoint
- an id field
- columns
- system views, the first of which is the default
- optional bulk actions and a create form

The declared resources SHALL be:
- targets, rollouts, distribution sets, target filters, software modules
- actions, incidents, roles, grants, retention rules
- webhooks, webhook deliveries, the action log
- target groups
- target tags, target types, distribution set tags, distribution set types, software module types

No entity SHALL have a list page outside this mechanism.

#### Scenario: Every view renders
- **WHEN** every resource is opened in every one of its system views
- **THEN** each renders a list, or an empty list, without an error callout

### Requirement: Server-side query
A list SHALL fetch exactly one server page. The page parameters SHALL be resolved in this order: the query string, then the saved view, then the system view, then the resource default. They are `q`, `sort`, `page`, `limit` and the columns.

`limit` SHALL default to 25 and be clamped to 10..100. Columns SHALL be restricted to the declared set. The console SHALL NOT filter or sort rows in memory.

#### Scenario: Saved view with override
- **WHEN** a saved view with `sort=name:ASC` is opened with `&sort=name:DESC` in the URL
- **THEN** the list is sorted descending

### Requirement: Every column is sortable and present
Every declared column SHALL be sortable by the server, and its header SHALL be a link that toggles ascending and descending. Every declared column SHALL exist as a key in the server's response, so that no column silently renders as empty.

#### Scenario: Header sort
- **WHEN** a user clicks a column header twice
- **THEN** the list is sorted by that column ascending, then descending

### Requirement: Counter and pager on top
A list SHALL show `first–last of total` and the pager above the table. The pager SHALL offer first page, previous page, next page, last page, and a jump to a page number. The jump SHALL carry the current view, q, sort, limit and columns.

#### Scenario: Pager position
- **WHEN** a list is rendered
- **THEN** the pager appears in the document before the table

### Requirement: Three states with a stable frame
A list SHALL have three designed states:
- normal: rows
- empty: `No rows returned for this view.`
- error: a danger callout in the data area

In the error state, the toolbar, filters, navigation, pager and create button SHALL remain. Header strips SHALL NOT change height with their content.

#### Scenario: Bad filter
- **WHEN** a list is opened with a `q` that the server rejects
- **THEN** the toolbar, filter panel, pager and New button remain and the table is replaced by a danger callout

### Requirement: Row links
A row SHALL link to its object page `/{resource}/{id}`. Alternatively it SHALL link to a filtered list of another resource when the resource declares that. The value in that filter SHALL be quoted and escaped. A named reference in a row, a `<field>Name` with its id, SHALL be a link to that object.

#### Scenario: Target group row
- **WHEN** the target group `eu,west` with 2 targets is clicked
- **THEN** the target list opens filtered to that group and shows a total of 2

#### Scenario: Assigned set is a link
- **WHEN** a target row shows its assigned distribution set name
- **THEN** that name links to the distribution set

### Requirement: Saved views
A user SHALL be able to save the current q, sort, columns and page size as a named view of one resource. The default name is `Custom view`, and the page size is at least 10. Saved views SHALL be private to the user and tenant and shall appear under the resource in the sidebar. They SHALL be editable, deletable, duplicable (`… copy`) and reorderable. The console SHALL check that a view belongs to the user before changing it.

#### Scenario: Save and reopen
- **WHEN** a user saves a filtered list as `My pending` and later opens it from the sidebar
- **THEN** the same filter, sort and columns apply

### Requirement: Long values and trailing controls
Long cell values SHALL be truncated with an ellipsis, with the full value available on hover. Trailing per-row buttons SHALL sit in their own aligned column, independent of label length.

#### Scenario: Long name
- **WHEN** a target name is 200 characters long
- **THEN** the column keeps its width and the name ends with an ellipsis
