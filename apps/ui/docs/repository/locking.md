# Locking and invalidation

> Two different protections: one stops software changing, the other stops it spreading.

## Implicit lock {#implicit-lock}

When a distribution set is assigned to a device or used by a rollout, the set **and every module in it** are locked. Their contents can no longer be changed: no attaching or detaching modules, no adding or removing artifacts.

The reason is that the record of what a device installed has to keep meaning something. If the contents of version 1.4.0 could be edited afterwards, "this device installed 1.4.0" would say nothing.

To ship different content, make a new module version and a new set. That is the intended workflow, not a limitation to work around.

Locking is a tenant setting and is on by default — see [implicit lock](/help/admin/settings#implicit-lock-enabled).

## Invalidation {#invalidation}

Invalidating a distribution set marks it unusable: it can no longer be assigned and no new rollout can carry it. Use it when a release turns out to be bad.

Invalidation asks two questions:

- **What to do with actions already running** — leave them alone, cancel them politely, or cancel them hard.
- **Whether to stop the rollouts** that are handing it out.

> Invalidation does not uninstall anything. Devices that already have the set keep running it. To move a fleet back, roll out the previous set.

## Deleted, locked, invalid {#the-three-states}

Locked :: The contents are frozen. The set still works and can still be assigned.
Invalid :: The set may not start anything new. Existing installations are untouched.
Deleted :: The set is hidden from the lists. It is kept because actions still refer to it.
