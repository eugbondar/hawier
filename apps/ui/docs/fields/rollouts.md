# Rollout fields

## status {#status}

Where the rollout is in its [lifecycle](/help/deployment/rollout-lifecycle#the-path): creating, ready, waiting for approval, starting, running, paused, stopping, stopped, finished, approval denied, deleting, deleted.

There is no error status — an error threshold [pauses](/help/glossary#paused) a rollout.

## totalTargets {#total-targets}

How many devices this rollout took. Fixed when the rollout was built: devices that started matching afterwards are not in it unless the rollout is [dynamic](/help/deployment/dynamic-rollouts#dynamic-rollouts).

## totalGroups {#total-groups}

How many waves the devices were divided into. See [deploy groups](/help/deployment/rollout-groups#deploy-groups).

## distributionSet {#distribution-set}

What every device in the rollout is told to install.

## targetFilterQuery {#target-filter-query}

The [query](/help/reference/rsql#the-query-language) that chose the devices, kept as the record of what the rollout aimed at.

## type {#type}

The [action type](/help/deployment/action-types#action-types) every device in this rollout gets: forced, soft, time-forced or download-only.

## weight {#weight}

Priority of this rollout's actions against other work on the same device. See [weight](/help/deployment/assignment#weight).

## forcetime {#forcetime}

For a time-forced rollout, the moment its actions stop being optional.

## startAt {#start-at}

The earliest moment the rollout may start. Before it, starting has no effect.

## dynamic {#dynamic}

Whether the rollout keeps absorbing newly matching devices after its planned groups finish. See [dynamic rollouts](/help/deployment/dynamic-rollouts#dynamic-rollouts).

## approvalRemark {#approval-remark}

What the approver or denier wrote. See [approve](/help/deployment/rollout-lifecycle#approve).

## approveDecidedBy {#approve-decided-by}

Who approved or denied it.
