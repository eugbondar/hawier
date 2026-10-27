## MODIFIED Requirements

### Requirement: Grammar and operators
`q` SHALL be parsed as `or := and (("," | " or ") and)*`, `and := cmp ((";" | " and ") cmp)*`, `cmp := "(" or ")" | selector op args`. AND SHALL bind tighter than OR, and parentheses SHALL group.

The keywords `and` and `or` SHALL be recognised only in lowercase and only with whitespace on both sides, as in hawkBit. They SHALL mean exactly what `;` and `,` mean, and the two forms MAY be mixed in one query.

Whitespace SHALL be ignored around `,` `;` `(` `)`, around the keywords, inside a value list, and at the start and end of `q`.

The operators SHALL be `== != =in= =out= =lt= =le= =gt= =ge= < <= > >=`. Selectors SHALL be matched case-insensitively.

The following SHALL get 400:
- trailing input, including a word after a value that is not a keyword, such as `AND`, `OR` or `&&`
- an empty value
- a dangling separator or keyword
- whitespace around an operator

Every value SHALL be bound as a parameter, never spliced into SQL. A filter SHALL always be combined by AND with the caller's tenant.

#### Scenario: Grouping
- **WHEN** `q=(name==a,name==b);controllerid!=c`
- **THEN** the result is targets named a or b, excluding controller id c

#### Scenario: Keyword operators
- **WHEN** `q=controllerid==fleet-* or controllerid==cross*`
- **THEN** the result is the same as for `q=controllerid==fleet-*,controllerid==cross*`

#### Scenario: Keyword precedence
- **WHEN** `q=name==a or name==b and controllerid!=c`
- **THEN** the result is the same as for `q=name==a,name==b;controllerid!=c`

#### Scenario: Uppercase keyword
- **WHEN** `q=name==a OR name==b`
- **THEN** the server answers 400 instead of matching nothing

#### Scenario: Rollout with a keyword filter
- **WHEN** a rollout is created with `targetFilterQuery` `controllerid==a* AND controllerid==b*`
- **THEN** the server answers 400 and no rollout is created

### Requirement: Quoting
A value MAY be wrapped in `'...'` or `"..."`, with a backslash escaping the next character. An unquoted value SHALL end at the first whitespace or `, ; ( )`. A value containing whitespace or those characters therefore SHALL be quoted. An unterminated quote SHALL get 400.

#### Scenario: Quoted group path
- **WHEN** `q=group=="eu/west,2"`
- **THEN** targets of the group `eu/west,2` are matched

#### Scenario: Spaced value
- **WHEN** a target is named `rock and roll`
- **THEN** `q=name=="rock and roll"` matches it, and `q=name==rock and roll` gets 400
