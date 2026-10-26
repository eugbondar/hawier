# Distribution set fields

## type {#type}

The [distribution set type](/help/repository/ds-types#distribution-set-types), by key. Decides which module types the set must contain and which devices may receive it.

## typeName {#type-name}

The same type, by name.

## complete {#complete}

Whether the set holds a module of every type its type demands. **An incomplete set cannot be assigned or rolled out** — see [complete](/help/repository/distribution-sets#complete).

## valid {#valid}

Whether the set may still be used. An [invalidated](/help/repository/locking#invalidation) set cannot start anything new; devices that have it keep it.

## locked {#locked}

Whether the set's contents are frozen. See [locking](/help/repository/locking#implicit-lock).

## requiredMigrationStep {#required-migration-step}

Marks the set as a version that must not be skipped. Recorded and handed to the device; acting on it is the agent's business.

## modules {#modules}

The [software modules](/help/repository/software-modules#software-modules) the set contains.

## assigned {#assigned}

Whether anything is using this set. `assigned==false` finds sets nothing needs.

## tag {#tag}

Used in queries: `tag==stable` selects sets carrying that [tag](/help/repository/distribution-set-tags#tags).
