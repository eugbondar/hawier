# Sorting

> How the sort order is written and which fields accept it.

## Sort orders {#sort-orders}

A sort order is `field:DIRECTION`, where direction is `ASC` or `DESC`, and several may be separated by commas: `status:ASC,createdAt:DESC`.

Sorting happens on the server, over the whole result set — not over the page you can see.

## Which fields sort {#sortable}

A field sorts when it is a real column of the object. Fields that are looked up through a relation — a tag name, a joined distribution set name — can be filtered but not sorted, and asking for them returns an error instead of an arbitrary order.

The field names are the ones listed in [the query reference](/help/reference/rsql#field-names).
