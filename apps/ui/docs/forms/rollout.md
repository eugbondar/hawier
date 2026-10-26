# The rollout form

> Eight fields. Four of them decide what happens to your fleet.

## The form {#the-form}

A rollout is built from what you enter here and then waits at [ready](/help/glossary#ready). Nothing is sent to any device until someone starts it, so getting this wrong is recoverable — up to the moment you press start.

Read [roll out an update to a fleet](/help/start/fleet-rollout#step-by-step) for the whole procedure.

## Name {#name}

What this rollout is called. Must be unique in the tenant.

Name it after what it does and when — `agent-1.5.0-eu-2026-09` beats `test2`. Rollouts are kept as the record of what happened, and in six months the name is all you will have.

## Distribution set {#distribution-set-id}

What every device in this rollout will be told to install.

It must be [complete](/help/repository/distribution-sets#complete) and not invalidated. An incomplete set is the usual reason this refuses.

## Target filter {#target-filter-query}

A [query](/help/reference/rsql#the-query-language) selecting the devices this rollout aims at.

Whoever matches **at the moment the rollout is built** is who it takes. Devices that appear later are not in it.

> Run the query on the [targets](/targets) list first. The row count there is exactly the number of devices this rollout will take, and it is the last chance to notice that it selected the whole fleet.

The query is checked when you submit, so an invalid one is refused here rather than leaving a broken rollout.

## Groups {#amount-groups}

How many waves to divide the matching devices into, between 1 and 500.

The devices are split evenly, so ten groups over a thousand devices means ten waves of a hundred. The **first group is your canary**: it carries the risk for everyone else. More groups means smaller waves and more chances to stop.

One group means everybody at once — the same as assigning to the whole selection, with a record attached.

## Success threshold % {#success-expression}

The share of a group that must finish before the next group is activated. Defaults to 100.

100 means every single device in the wave must finish, so one device that is switched off stalls the rollout. 90 or 95 lets the wave complete and leaves the stragglers to be picked up afterwards from the [pending update](/help/inventory/targets#pending-update) list.

Devices that only [downloaded](/help/glossary#downloaded) count towards this too, which is what lets a download-only rollout finish.

## Error threshold % (pause) {#error-expression}

The share of a group that may fail before the whole rollout [pauses](/help/glossary#paused) itself. Leave it empty for no limit.

When it is reached, that group goes to [error](/help/glossary#error), the rollout pauses, and no further group is activated. Devices already working carry on.

Set it to what you would actually accept. If five failures in a hundred would make you stop and investigate, the number is 5. Empty means the rollout will keep going through every wave no matter how many devices it breaks.

## Type {#type}

How hard each device is pushed — see [action types](/help/deployment/action-types#action-types). One choice for the whole rollout.

Be careful with **soft**: it has no deadline, and devices may simply never install. Be careful with **download only**: nothing is installed at all, by design.

## Description {#description}

Free text. Worth using for why this rollout exists and what to do if it goes wrong — the next person to open it at 3am is the audience.
