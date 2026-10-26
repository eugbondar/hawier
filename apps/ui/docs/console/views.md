# Views

> Named filters in the navigation column: a query, a sort order and a column set, saved under a name.

## System views {#system-views}

A **system view** is a filter that ships with the console — **Needs attention**, **Running**, **Without artifacts**. It is a saved question, not a separate list: it opens the same page with a query already filled in, so counts, sorting and paging behave exactly as they do on the unfiltered list.

System views cannot be edited or deleted. To make one of your own, adjust the filters and save the result as a view of your own.

A list contributed by an extension brings its own views with it; the console shows them next to the built-in ones.

## Saved views {#saved-views}

A **saved view** is your own filter. Set the query, the sort order, the page size and the columns you want, type a name into **Save view** in the filter panel, and it appears in the navigation column under the list it belongs to.

Saved views are private to you and to the tenant you are signed in to. Nobody else sees them, and they survive sign-out.

## Editing a view {#editing}

The pencil next to a saved view opens its editor. You can change its name, its query, its sort order, its columns and its page size; you can duplicate it as a starting point for another; you can move it up or down the list; and you can delete it. Deleting a view never touches the objects it selected.

## Columns {#columns}

**Columns** in the filter panel chooses which columns the list shows. The choice travels with the view, so two views of the same list can show different columns.

Every column header is a link that sorts by that column. Click it again for the opposite direction. Every timestamp is shown in your own zone — see [time](/help/reference/time#reading-a-timestamp).
