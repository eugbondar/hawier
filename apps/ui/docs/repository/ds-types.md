# Distribution set types

> The rule that says what a distribution set must contain.

## Distribution set types {#distribution-set-types}

A distribution set type is a template: it names which [software module types](/help/repository/sm-types#software-module-types) a set of that type must hold, and which it may hold.

Every distribution set has exactly one type, and that type is what decides whether the set is [complete](/help/repository/distribution-sets#complete).

## Mandatory and optional {#mandatory-and-optional}

**Mandatory** module types must be present for a set of this type to be complete. **Optional** ones may be present.

A type declaring **operating system** mandatory and **application** optional means: no set of this type may be assigned until it holds an operating system module, but applications are up to whoever builds it.

Adding a mandatory module type to a type that already has sets makes those sets incomplete until they are filled in.

## Compatibility {#compatibility}

A [target type](/help/inventory/target-types#target-types) lists which distribution set types may be installed on its devices. That is where "this software does not belong on that hardware" is enforced.

## The key {#key}

A short stable identifier, separate from the display name. See [keys](/help/repository/sm-types#key).

## All {#all}

Every distribution set type in the tenant.
