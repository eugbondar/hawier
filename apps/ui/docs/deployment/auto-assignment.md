# Auto-assignment

> A standing rule: every device matching this filter gets this distribution set, forever.

## Auto-assignment {#auto-assignment}

Attach a distribution set to a [target filter](/help/deployment/target-filters#target-filters) and the server keeps checking: any device that matches the filter and does not already have that set assigned is given it.

It runs continuously in the background. Not once — continuously. A device that is factory-reset and rematches will be assigned again.

## What you set {#what-you-set}

Distribution set :: What matching devices are told to install.
Action type :: How hard they are pushed — see [action types](/help/deployment/action-types#action-types).
Weight :: Priority against other actions on the same device. See [weight](/help/deployment/assignment#weight).
Confirmation required :: Whether the assignment waits for a yes. See [confirmation](/help/deployment/confirmation#confirmation-required).

## When to prefer it over a rollout {#versus-rollouts}

Auto-assignment is right when the answer is "everything that looks like this should run this", with no waves and no thresholds — a baseline version for new devices, a mandatory security fix.

A rollout is right when you want to see what a hundred devices do before the other nine hundred try it.

## Stopping it {#stopping}

**Clear** removes the rule. Devices that were already assigned keep their assignment and finish what they started — clearing the rule stops future assignments, it does not undo past ones.

> A filter that is too broad hands the set to the whole fleet at once, with no wave to check first. Run the query on the target list and read the count before attaching a set to it.

## Nothing is auto-assigned {#nothing}

The usual, and safe, state: the filter names devices and does nothing to them.
