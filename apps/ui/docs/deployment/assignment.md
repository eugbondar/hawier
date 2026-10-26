# Assigning a distribution set

> Telling one device, or a selection of devices, to install something.

## Assigning {#assigning}

Pick a distribution set, choose an [action type](/help/deployment/action-types#action-types), and assign. An [action](/help/deployment/actions#actions) is created, the device's **assigned** set becomes the one you chose, and its status becomes [pending](/help/glossary#pending).

Nothing reaches the device until its next poll.

You can assign from a device's own page, or to a whole selection of rows in the target list through the operations menu.

## What happens to the previous action {#superseding}

A device normally works one job at a time, so a new assignment supersedes whatever was outstanding.

Superseding is a request, not an instruction: the old action goes to [canceling](/help/glossary#canceling) and waits for the device to acknowledge. A device that has passed the point of no return may [refuse](/help/glossary#cancel-rejected), and then it finishes what it was doing.

If the tenant has automatic closing of superseded actions switched on, the old action is closed as [canceled](/help/glossary#canceled) immediately with no handshake — faster, but the device is never told, and it may still install what it had already downloaded. See [autoclose](/help/admin/settings#repository-actions-autoclose-enabled).

## Multiple assignments {#multiple-assignments}

With multiple assignments enabled for the tenant, a device may carry several active actions at once, and nothing supersedes anything. The device is served them one at a time, highest [weight](#weight) first. See [multi-assignment](/help/admin/settings#multi-assignments-enabled).

## Weight {#weight}

A priority between 0 and 1000, defaulting to 1000. It decides which action a device is served first when it has more than one.

With multiple assignments switched off, a device never has more than one active action, and weight has no visible effect.

## Confirmation required {#confirmation-required}

Whether this assignment must be confirmed before the device may install it. See [confirmation](/help/deployment/confirmation#the-confirmation-flow).

## Target types and compatibility {#compatibility}

A device with a [target type](/help/inventory/target-types#target-types) only accepts distribution sets whose type its target type lists as compatible. Assigning anything else is refused with **not compatible**. If one device in a selection is incompatible, nothing is assigned to any of them, so fix the selection and assign again. From a device's page the refusal is shown as **Assign stopped** with the server's reason and an [incident number](/help/admin/incidents#incident-numbers); it is also in the [action log](/help/admin/action-log#the-action-log).

A device without a target type accepts any set, unless the tenant [requires types](/help/admin/settings#target-type-required); then it accepts nothing.

Rollouts and automatic assignment apply the same rule, but they skip incompatible devices quietly instead of failing.

## What an operator cannot do {#not-possible}

You cannot push. You cannot make a device poll sooner. You cannot install on a device that is not talking to this server. Every one of those is the device's decision, and the console can only leave a job waiting for it.
