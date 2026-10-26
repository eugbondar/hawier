# The query language

> How to write the queries in the filter panel, in target filters, and in retention rules.

## The query language {#the-query-language}

A query is one or more comparisons, combined. It runs on the server, over every row, not over the page in front of you.

```
updateStatus==error
name==prod-*;updateStatus==pending
updateStatus==error,updateStatus==pending
attribute.hw==rev2;tag==canary
assignedDistributionSet==null
```

## Combining {#combining}

`;` :: and
`,` :: or
`( )` :: grouping

**`;` binds tighter than `,`.** So `a==1,b==2;c==3` means `a==1 OR (b==2 AND c==3)`. Use brackets when you mean otherwise.

There are no `and` / `or` keywords. Only the two punctuation marks.

## Operators {#operators}

`==` :: equal to
`!=` :: not equal to
`=in=` :: one of a list — `status=in=(running,paused)`
`=out=` :: none of a list
`=lt=` or `<` :: less than
`=le=` or `<=` :: at most
`=gt=` or `>` :: greater than
`=ge=` or `>=` :: at least

## Wildcards {#wildcards}

`*` matches any run of characters, in a value used with `==` or `!=`, on a text field. Matching is case-insensitive.

```
name==prod-*
controllerId==*-gw-*
group==eu/*
```

Literal `%` and `_` in a value are treated as themselves, not as wildcards.

## Empty and not empty {#null}

`field==null` matches rows where the field is not set; `field!=null` matches rows where it is. Case does not matter.

This is how you ask the useful negative questions:

```
assignedDistributionSet==null
rollout!=null
```

## Quoting {#quoting}

An unquoted value ends at the first `,` `;` `(` or `)` and is trimmed of spaces. Anything containing those, or leading and trailing spaces you mean to keep, must be quoted:

```
name=="my device, mark 2"
group=="eu/west (spare)"
```

Both single and double quotes work, and `\` escapes inside them.

## Field names {#field-names}

Field names are the ones the server knows, not the column headings. They are case-insensitive.

The fastest way to find the right name and value is the **raw** switch on any object page: it shows every field exactly as the server has it, and that text is what a query takes. See [raw values](/help/console/lists#raw).

Three kinds of field behave differently:

- **Plain fields** — most of them. Everything above works.
- **Status fields** — `updateStatus`, `status`, `detailStatus`. They take **names**, not numbers, and only `==`, `!=`, `=in=` and `=out=` work on them. `status=gt=running` is refused.
- **Looked-up fields** — `tag`, and joined names like `assignedDs.name`. Only `==`, `!=`, `=in=` and `=out=`, and they cannot be [sorted](/help/reference/sorting#sortable) by.

## Attributes and metadata {#key-value}

Written with the key after a dot:

```
attribute.hw==rev2
metadata.owner==logistics
attribute.serial==SN-*
```

> These are always compared as **text**, whatever they look like. `attribute.bucket=le=50000` keeps `9` and drops `10000`, because as text `'10000'` sorts before `'9'`. There is no warning. See [sharp edges](/help/reference/sharp-edges#numeric-attributes).

## When a query is wrong {#errors}

The list is replaced by a message and the rest of the page stays where it is. The message names what went wrong and where — an unknown field, a bracket that never closed, a number where text was given, a status name that does not exist.

An unknown field is usually a column heading typed instead of a field name. Use the raw switch to find the real one.

## Worked examples {#examples}

```
updateStatus==error,updateStatus==pending
assignedDistributionSet==null
complete==true
artifactCount=gt=0
rollout!=null
status=in=(ready,creating)
kind=in=(ddi,mgmt);status=ge=500
active==true;dryRun==false
tag==canary;attribute.hw==rev2
```
