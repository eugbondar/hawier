# Target tags

> Named, coloured labels. A device carries as many as you like.

## Tags {#tags}

A tag is a label with a name, an optional description and an optional colour. Devices carry any number of them, and tags are the usual way to say "these devices" without writing a query.

Tagging changes nothing about a device. It is a label, not a setting: nothing is installed, cancelled or scheduled because a tag was added.

## Using tags to select devices {#selecting}

`tag==canary` in a filter selects every device carrying that tag, and that is a query a rollout can aim at. Because a device can hold several tags, overlapping sets — `canary`, `eu`, `field-trial` — work the way you would expect.

Tag names are matched exactly. Wildcards work: `tag==eu-*`.

## Adding and removing {#adding}

On a device page, or on a selection in the target list through the operations menu. Removing a tag from a device never deletes the tag itself.

Deleting a tag removes it from every device that carried it. The devices are otherwise untouched.

## Colour {#colour}

A colour makes a tag findable at a glance in a crowded page. It has no other effect.

## All {#all}

Every tag in the tenant.
