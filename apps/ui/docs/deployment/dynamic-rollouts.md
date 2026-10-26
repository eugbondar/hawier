# Dynamic rollouts

> A rollout that never finishes, and keeps absorbing devices as they appear.

## Dynamic rollouts {#dynamic-rollouts}

An ordinary rollout takes a fixed set of devices: whoever matched its query at the moment it was built. Devices that appear later are not in it.

A dynamic rollout keeps going. When all of its planned groups are done, it creates a new group from devices that now match its query and were not in any earlier group, activates it immediately, and waits for more.

## When to use one {#when}

For a fleet that grows continuously and where every new device should arrive on a known version — a factory line, a shipping product. The rollout becomes a standing rule rather than an event.

For a one-off update to a known fleet, do not use one: it will never reach [finished](/help/glossary#finished) and you will never know when the job is done.

## What it does not do {#limits}

The extra groups are created at full size with fixed thresholds. Only the planned groups are staged and watched the way you set them up; the dynamic tail is not a canary.

A dynamic rollout stays [running](/help/glossary#running) indefinitely. Stop it when the standing rule no longer applies.

## The alternative {#alternative}

[Auto-assignment](/help/deployment/auto-assignment#auto-assignment) on a target filter does much the same job with less machinery: every device that matches the filter is given the set, forever, with no groups and no thresholds. Prefer it unless you actually want the waves.
