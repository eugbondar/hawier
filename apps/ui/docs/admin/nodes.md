# Nodes

> Which parts of the system need which server, and what stops when one is down.

## Control and sub nodes {#control-and-sub}

A node runs in one of two roles.

Control :: Runs the background work — advancing rollouts, applying auto-assignment rules, housekeeping, delivering events, retention.
Sub :: Serves devices and answers read-only management requests.

## What survives a control node outage {#what-survives}

Devices keep updating. Polling, downloading, reporting and cancelling all work on a sub node with the control node gone: nothing on the device-facing path depends on a background loop.

What stops is **progression**. Rollouts do not advance to their next group, auto-assignment rules do not fire, retention does not run, and events queue up instead of being delivered. All of it resumes when a control node returns; nothing is lost, it is deferred.

## Which node am I looking at {#which-node}

[Settings](/settings) names the server this console is talking to and its build. An [incident](/help/admin/incidents#incident-numbers) records which node produced it, which is how you tell whether a failure is on one node or all of them.

## Builds {#builds}

Console and server print their builds side by side on [Settings](/settings). After a deployment they should match — a mismatch explains a console that does not know about a field the server has, or the reverse.
