# Common fields

> Fields that mean the same thing on every kind of object.

## id {#id}

The object's identifier: a decimal number you can select with a double-click and read out. Paste it back into a list as `id==...` to find the row again.

Devices are the exception — they are identified by their [controllerId](/help/fields/targets#controller-id).

## name {#name}

What the object is called. Free text, and on most objects it can be changed without affecting anything that refers to it.

Names are matched with wildcards in a query: `name==prod-*`.

## description {#description}

Free text about the object. Not used for anything; worth filling in, because it is what the next person has.

## version {#version}

The version of this software module or distribution set. Name and version together are the identity, so two versions are two separate objects and both can exist at once.

## key {#key}

The short, stable identifier of a type — `os`, `application`, `gateway-v2`. Unlike the name it is what devices and other systems see, so changing it changes what they see. See [keys](/help/repository/sm-types#key).

## colour {#colour}

A colour for a tag or a type, so it stands out in a crowded list. Cosmetic; nothing behaves differently because of it.

## createdAt {#created-at}

When the object was created, shown in your own time zone — see [time](/help/reference/time#reading-a-timestamp).

In a query it takes the millisecond value, which is printed in brackets on the object's own page.

## createdBy {#created-by}

Who created it.

## lastModifiedAt {#last-modified-at}

When the object was last changed. A list sorted by this shows what has been touched recently, which is not the same as what was created recently.

## lastModifiedBy {#last-modified-by}

Who changed it last.

## username {#username}

The account a record belongs to, as they authenticated.

## deleted {#deleted}

Whether the object has been removed. Objects something still refers to are marked deleted and hidden rather than removed, so what points at them keeps working.
