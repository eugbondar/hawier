# The console

> How this interface is laid out, and what the pieces do.

## The overview {#the-overview}

The first page is a summary of the whole tenant: how many devices are in each state, how many objects of each kind exist, which rollouts are moving, and what has recently failed.

Every number on it is a link, and it opens exactly the list that produced the number. If a card says 218 pending, the list behind it holds those 218 rows.

## Counts that add up {#counts-that-add-up}

A count and the list it leads to always come from the same question, so they always agree. If a card says 474 and the list shows 474, that is not a coincidence — the card is the total of the query the card links to.

This matters when a number surprises you: click it. The list will show you which rows the number is made of, and the query that selected them is in the filter panel.

## The navigation column {#the-navigation-column}

Lists are grouped by what they are for.

Inventory :: The devices themselves, and the ways they are labelled.
Deployment :: Everything about getting software onto devices — rollouts, distribution sets, filters, actions.
Repository :: The software: modules and the artifacts inside them.
Admin :: Tenant configuration, access, automation and the record of what happened.

Under each list are its **views** — saved questions. See [views](/help/console/views#system-views).

The control at the top left collapses the column to a rail of icons. The rail is why every list has an icon of its own: at 40 pixels wide, the icon is the only thing telling two lists apart.

## The top bar {#the-top-bar}

The search box searches everything at once — see [search](/help/console/search#global-search). The question mark to its right arms [What's This? mode](/help/console/help#whats-this).

## Who you are {#who-you-are}

The foot of the navigation column shows the tenant and the user you are signed in as, and the way out. Everything you do is recorded against that name in the [action log](/help/admin/action-log#the-action-log).
