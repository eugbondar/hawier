# Distribution sets

> The bundle you assign: a named, versioned collection of software modules.

## Distribution sets {#distribution-sets}

A distribution set is what a device is told to install. It has a name, a version, a type, and it contains [software modules](/help/repository/software-modules#software-modules).

Devices are never given a module. They are given a set, and the set decides which modules go together — which is what stops an operating system landing without the application that has to match it.

## Complete {#complete}

Whether the set contains everything its type demands.

The set's [type](/help/repository/ds-types#distribution-set-types) lists module types as mandatory or optional. A set holding a module of every mandatory type is complete; anything less is not. **An incomplete set cannot be assigned or rolled out** — this is the usual answer to "why can I not assign this".

## Valid {#valid}

Whether the set is still usable. A set that has been [invalidated](/help/repository/locking#invalidation) is not, and cannot start anything new. Devices that already installed it keep it.

## Locked {#locked}

Whether the set's contents are frozen. Sets that have been assigned or rolled out are locked so that what a device installed cannot change underneath the record of it. See [locking](/help/repository/locking#implicit-lock).

## Modules {#modules}

Attach and detach modules on the set's page. Each module shows its artifacts, so you can see exactly what a device would download.

Attaching is refused if the set is locked, or if the module's type is not one the set's type allows.

## Tags {#tags}

Sets carry [tags](/help/repository/distribution-set-tags#tags) the same way devices do — `stable`, `beta`, `recalled`.

## Where this set is {#where-this-set-is}

The cards on a set's page count where it is in use: devices it is assigned to, devices that have it installed, rollouts that carry it, filters that hand it out automatically. Each card opens the list it counted.

## Required migration step {#required-migration-step}

Marks the set as a step that cannot be skipped over. It is recorded and handed to the device; what a device does with it is up to its agent.

## Latest {#latest}

Every set, newest first.

## Complete sets {#complete-sets}

Sets that hold everything their type demands — the ones you can actually assign.

## Incomplete {#incomplete}

Sets still missing a mandatory module type. Usually a set someone started and did not finish.

## Assigned {#assigned}

Sets that are assigned to at least one device.

## Unassigned {#unassigned}

Sets nothing is using. Candidates for deletion.

## Recently changed {#recently-changed}

Sets touched most recently.
