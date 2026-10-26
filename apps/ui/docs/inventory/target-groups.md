# Target groups

> One path per device, like a folder: `eu/west/gateway`.

## Groups {#groups}

A group is a free-text path stored on the device. A device belongs to at most one, and the path is whatever you type — the slashes carry no meaning to the server, they are a convention for reading.

Groups exist for hierarchy: sites, regions, customers, racks. For "which devices do I mean" in a rollout, a [tag](/help/inventory/target-tags#tags) or a [filter](/help/deployment/target-filters#target-filters) is usually the better tool, because a device can carry many tags and only one group.

## Setting a group {#setting}

On a device page, or on a selection of rows in the target list. The field suggests paths that already exist but accepts anything you type, because the group you want may not exist yet — there is no separate step for creating one. Typing a new path creates it by using it.

## The list {#the-list}

The Target Groups list shows each path in use and how many devices carry it. A row does not open a page of its own: it opens the devices in that group, which is also the query the count came from.

## All {#all}

Every group path in use, with its device count.
