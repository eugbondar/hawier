# The target filter form

> A name and a query.

## The form {#the-form}

Saves a query over the device list under a name, so a rollout can aim at it and you do not have to retype it.

## Name {#name}

What the filter is called. Name it after the devices it selects — `eu-gateways-rev2`, not `filter3`.

## FIQL query {#query}

The query itself — see [the query reference](/help/reference/rsql#the-query-language).

It is checked when you save, so a filter that would fail is refused now rather than at the moment a rollout depends on it.

Test it first on the [targets](/targets) list: paste it into **Filters**, apply, and read the count. That count is exactly what this filter will select.

Examples:

```
updateStatus==error
tag==canary;attribute.hw==rev2
assignedDistributionSet==null
group==eu/west*
```
