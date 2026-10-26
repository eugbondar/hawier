# Target types

> What kind of device this is — and which distribution set types are allowed on it.

## Target types {#target-types}

A target type is the class of hardware or product a device belongs to. A device has at most one, and it is the only label that **restricts** something: a device of a given type accepts only the distribution set types that type declares compatible.

That is what makes it different from a [tag](/help/inventory/target-tags#tags). A tag is a label; a type is a rule.

## Compatible distribution set types {#compatible-types}

Each target type lists the distribution set types that may be installed on devices of that type. Assigning a set whose type is not on the list is refused.

A target type with no compatible types declared accepts nothing: every set is refused on its devices. A device with no target type at all accepts any set — unless the tenant [requires types](/help/admin/settings#target-type-required).

Rollouts and automatic assignment skip devices the rule excludes, see [compatibility](/help/deployment/assignment#compatibility).

## The key {#key}

A short stable identifier, separate from the display name. The name is for people and may be changed; the key is what other systems refer to. See [keys](/help/repository/sm-types#key).

## Setting a device's type {#setting}

On the device page. Changing the type of a device that already has something installed does not uninstall anything — it changes what may be assigned next.

## All {#all}

Every target type in the tenant.
