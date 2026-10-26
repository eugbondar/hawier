# Spec Delta

## Purpose

The FIQL/RSQL filter language accepted as `q` across the management API, target
filters and rollout queries: its grammar, quoting, wildcards, typed fields,
per-resource field maps, and error behavior.

## ADDED Requirements

### Requirement: Grammar and operators
`q` SHALL be parsed as `or := and ("," and)*`, `and := cmp (";" cmp)*`, `cmp := "(" or ")" | selector op args`. AND SHALL bind tighter than OR, and parentheses SHALL group.

The operators SHALL be `== != =in= =out= =lt= =le= =gt= =ge= < <= > >=`. Selectors SHALL be matched case-insensitively.

The following SHALL get 400:
- trailing input
- an empty value
- a dangling separator
- whitespace around an operator

Every value SHALL be bound as a parameter, never spliced into SQL. A filter SHALL always be combined by AND with the caller's tenant.

#### Scenario: Grouping
- **WHEN** `q=(name==a,name==b);controllerid!=c`
- **THEN** the result is targets named a or b, excluding controller id c

### Requirement: Quoting
A value MAY be wrapped in `'...'` or `"..."`, with a backslash escaping the next character. An unquoted value SHALL end at the first `, ; ( )` and SHALL be trimmed. A value containing those characters therefore SHALL be quoted. An unterminated quote SHALL get 400.

#### Scenario: Quoted group path
- **WHEN** `q=group=="eu/west,2"`
- **THEN** targets of the group `eu/west,2` are matched

### Requirement: Wildcards
On text columns, `*` with `==` or `!=` SHALL match case-insensitively as a wildcard, with literal `%` and `_` escaped. `*` SHALL NOT be expanded inside `=in=` or `=out=`.

#### Scenario: Prefix match
- **WHEN** `q=controllerid==t-*`
- **THEN** every target whose controller id starts with `t-` matches, regardless of case

### Requirement: Typed fields
- `field==null` and `field!=null` SHALL test for absence or presence on column fields.
- Numeric fields SHALL reject non-numbers with 400.
- Boolean fields SHALL accept only `true` and `false`, case-insensitively.
- Enum fields SHALL accept their value names case-insensitively, and only `== != =in= =out=`. An unknown name or another operator SHALL get 400.

#### Scenario: Enum by name
- **WHEN** `q=updatestatus==in_sync`
- **THEN** targets with update status IN_SYNC match

#### Scenario: Unknown enum value
- **WHEN** `q=updatestatus==sideways`
- **THEN** the server answers 400

### Requirement: Key-value and existence fields
`attribute.<key>` and `metadata.<key>` SHALL match by key and value, with `== != < <= > >=`. `=in=`/`=out=` or a missing key SHALL get 400. Values SHALL be compared as text.

Relation fields SHALL test existence. Examples are `tag`, `assignedds.name`, `installedds.version` and `targettype.key`. `==`/`=in=` SHALL test that such a relation exists, and `!=`/`=out=` that it does not. Any other operator SHALL get 400.

#### Scenario: Metadata filter
- **WHEN** a target has metadata `site=berlin` and `q=metadata.site==berlin`
- **THEN** that target matches

#### Scenario: Tag filter
- **WHEN** two targets carry tag `eu-fleet` and `q=tag==eu-fleet`
- **THEN** the total is 2

### Requirement: Per-resource field maps
Each resource SHALL declare which fields are filterable and which are sortable. An unknown field SHALL get 400 with:
- `exceptionClass` `org.eclipse.hawkbit.repository.exception.RSQLParameterSyntaxException`
- `errorCode` `hawkbit.server.error.rest.param.rsqlInvalid`

Only plain and enum column fields, and declared sort aliases, SHALL be sortable. The sort aliases are `tag` on targets, `complete` on distribution sets, and `targets` on target group details.

The target field map SHALL also validate target filter queries and rollout target filter queries. An invalid query SHALL get 400 when the filter or rollout is created.

#### Scenario: Unknown field
- **WHEN** `q=bogusfield==1`
- **THEN** the server answers 400 with errorCode `hawkbit.server.error.rest.param.rsqlInvalid`

#### Scenario: Invalid target filter
- **WHEN** a target filter is created with query `nosuchfield==1`
- **THEN** the server answers 400
