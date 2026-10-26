# Extensions

> Parts of the system that are deliberately not built in.

## Extensions {#extensions}

Some decisions are made outside the server by a loadable module: where artifacts are stored, how a rollout divides a fleet, what a retention rule keeps, where events are delivered.

They receive decisions and descriptions, never bulk data and never a handle on the database. A storage extension says **where** a file is; it does not carry the bytes.

## No extension, no fallback {#no-extension-no-fallback}

When the point an extension was supposed to fill is empty, the server **refuses** rather than doing the job itself.

Artifact storage :: No extension means uploads and downloads answer 503.
Rollout strategy :: No extension means a new rollout stays in [creating](/help/glossary#creating) and never gets its groups.
Retention :: No extension means rules fire and delete nothing.
Event delivery :: No extension means events accumulate undelivered.

This looks unhelpful and is the point. A silent fallback would mean a fleet updated by a division nobody chose, or a file served from somewhere nobody configured.

None of it is fixable from the console. It is a question for whoever deployed the node.

## Parameters {#parameters}

Extensions are configured by named parameters, and [Settings](/settings) lists them with the value each one is running with.

A parameter is part of a node's deployment, not of the tenant: two nodes can be configured differently, and the page names which node it is describing. Where the node keeps its parameters writable, they can be edited here; otherwise the page shows what is set and where it came from.

## Console surfaces {#surfaces}

An extension can also add a list to this console. Those arrive when the console starts and behave like any other list. See [extension surfaces](/help/console/surfaces#declared-by-an-extension).

Built-in lists always win a name collision, so an extension can add but never replace.
