# Rollout strategies

> How the devices are divided into waves. Two decisions, made outside the server.

## Strategies {#strategies}

A strategy answers two questions and nothing else: how to split the matching devices into groups, and whether a running group is done.

It never sees the devices. It is given the total, the group definitions and the counts, and it answers with a description of each slice that the server then executes.

## Even shares {#classic}

The default. Each group takes its percentage of what the earlier groups **left**, and the last group takes everything remaining, so rounding can never drop a device.

Ten groups over 1000 devices gives ten waves of 100. Ten groups over 1003 gives nine waves of 101 and a last wave with the remainder.

## Stable buckets {#bucket}

A canary strategy. Each device is placed in a bucket derived from its controller id, and each wave takes a range of buckets.

The point is stability: the same device lands in the same bucket every time, on every node, without asking the device anything. Your canaries are the same machines rollout after rollout, and re-planning a rollout picks the same devices it picked before.

## No strategy, no rollout {#no-strategy}

Dividing a fleet is done by an [extension](/help/admin/extensions#extensions). If the node has none available, a new rollout stays in [creating](/help/glossary#creating) and never gets its groups.

This is deliberate: with nobody to answer "how should this be divided", the server refuses rather than inventing a division and sending an update to the wrong devices. It is a deployment question, not something to fix in the console.
