# Lists

> Every list in the console works the same way: count and paging above the table, filters on the left, operations on the selection.

## The list page {#the-list-page}

The row count and the paging controls sit above the table, so you can see how many rows the query matched before reading any of them. The controls jump to the first page, step back, jump to a page by number, step forward and jump to the last page.

## Filters {#filters}

**Filters** opens the panel on the left: a query, a sort order, a page size and the column set. **Apply** reloads the list; **Reset** clears everything and returns to the plain list.

The query is written in the filter language described in [the query reference](/help/reference/rsql#the-query-language), and the names it accepts are covered in [columns](/help/reference/columns#an-undocumented-column). It runs on the server, so it searches every row, not just the page in front of you.

## Selecting rows {#selection}

The box in the first column selects a row; the box in the header selects everything on the page. **Ctrl+click** adds a single row to the selection, **Shift+click** selects a range. Selected rows are highlighted, and the top bar of the list shows how many are selected.

Right-click over the selection — or use the counter in the toolbar — to open the operations menu.

## Bulk operations {#bulk}

The operations offered depend on the list. They apply to every selected row. Operations that destroy something ask first and tell you how many rows they are about to affect. See [bulk operations](/help/bulk#bulk-operations).

## Search {#search}

The box in the top bar searches across targets, rollouts, distribution sets, modules and tags at once. **Ctrl+K** puts the cursor in it.

## Page size {#page-size}

**Limit** is how many rows one page holds, between 10 and 100. It is part of the view, so a list you watch all day can be short and a list you scan occasionally can be long.

Changing it does not change what the query matched — only how much of it you see at a time. The count above the table is always the full total.

## Pickers {#pickers}

A field that has to name an existing object — a distribution set, a tag, a module, a type — is a search box, not a dropdown. Type a fragment, the server answers with matches, and you pick one. A tenant with a thousand distribution sets does not fit in a dropdown, so there are none.

Some pickers accept text that does not exist yet — a target group path is the example. Those suggest what already exists but keep whatever you type.

## Creating an object {#creating}

**New** in the toolbar opens the create form for that list. Every field on it has an explanation: point at it with the question mark, or read the chapter the form links to.

Required fields are marked by the browser and refused on submit. A field the server rejects comes back with the reason printed under it, and the rest of what you typed is still there.

## Editing and deleting {#editing}

**Edit** on an object page reopens the same form with the current values. Saving changes only the fields the form carries — everything else about the object is untouched.

**Delete** always asks first. Deleting from a list, with rows selected, tells you how many rows are about to go. Some objects cannot be deleted while something refers to them; those are marked deleted and disappear from the lists instead.

## Raw values {#raw}

The **raw** switch on an object page shows every value exactly as the API returned it, instead of the formatted version. Those are the values a query takes, so it is the fastest way to build a filter: read the raw value, paste it into the query.
