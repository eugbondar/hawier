# Action types

> How hard the device is pushed: four settings, and they are not interchangeable.

## Action types {#action-types}

Chosen when you assign a set, create a rollout, or set up an auto-assignment.

Forced :: Install it now. The device is told it has no discretion.
Soft :: The device may install when convenient. It may defer indefinitely, and nothing ever forces it.
Time-forced :: Soft until a moment you name, forced from then on.
Download only :: Download the files and stop. The device is never told to install.

## Forced {#forced}

The device is told to download and to install, both without discretion. This is the default, and the right choice when the update must land.

## Soft {#soft}

The device is told it may download and may install, at a moment that suits it. What "suits it" means is entirely the agent's business.

> Soft has **no deadline**. A device that never finds a convenient moment never installs, and the action stays active forever. If you need it to land eventually, use time-forced.

## Time-forced {#timeforced}

Soft until the moment you set, then forced. The device sees a soft job until that time passes and a forced one after it.

The moment is a specific date and time, set when the action is created — not a duration.

## Download only {#downloadonly}

The device downloads the artifacts and reports [downloaded](/help/glossary#downloaded). **The action then closes**: the device is never told to install, and `downloaded` is the terminal state rather than a step on the way.

Use it to stage large files ahead of a maintenance window, then send the real update afterwards. Do not use it expecting an installation.

## Which one a rollout uses {#in-rollouts}

A rollout has one action type for all of its groups, chosen on the create form.
