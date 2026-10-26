# Naming and labelling devices

> Five different ways to say something about a device. They do not mean the same thing, and this is the most common source of confusion in the product.

## Five ways to label a device {#five-ways-to-label-a-device}

Attributes :: What the **device** says about itself. Reported by the device, never edited here.
Metadata :: What **you** say about the device. Free key/value pairs, edited here, optionally readable by the device.
Tags :: Named, coloured labels. A device has any number of them. Made for selecting sets of devices.
Target type :: The **kind** of device it is. A device has at most one, and the type restricts which distribution set types may be installed on it.
Group :: A single path like `eu/west/gw`. A device is in at most one. Made for hierarchy.

If you are choosing between them: tags for "which devices do I mean", type for "what kind of thing is this", group for "where does it live", metadata for notes, attributes for facts the device reports.

## Attributes {#attributes}

Facts the device reports about itself — firmware revision, hardware version, serial numbers, whatever its agent is written to send. They arrive with the device's own updates and cannot be edited from this console.

They are filterable: `attribute.hw==rev2` selects every device that reported that.

> Attribute values are compared as **text**, always. `attribute.bucket=le=50000` keeps `9` and drops `10000`, because `'10000'` sorts before `'9'`. It does not warn you. See [sharp edges](/help/reference/sharp-edges#numeric-attributes).

## Metadata {#metadata}

Your own key/value notes on an object. Targets, distribution sets and software modules each have their own set.

The key travels as data, so it may contain slashes, spaces and percent signs. Saving an existing key overwrites it — one form both adds and edits.

## Visible to device {#visible-to-device}

Metadata on a **software module** can be marked visible to the device. Visible entries are included in the job description the device downloads, alongside the artifacts; everything else stays on the server.

Use it to hand a device a value it needs at install time. Do not use it for anything secret: it goes to the device in plain text.

## Tags {#tags}

See [target tags](/help/inventory/target-tags#tags).

## Target types {#types}

See [target types](/help/inventory/target-types#target-types).

## Groups {#groups}

See [target groups](/help/inventory/target-groups#groups).
