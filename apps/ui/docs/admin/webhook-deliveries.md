# Webhook deliveries

> The record of what was sent, where, and what came back.

## Deliveries {#deliveries}

One row per attempt: which webhook, which event, when, how long it took, and the result.

This is where you look when the other system says it never heard about something.

## Reading a failure {#failures}

The error column holds what went wrong — a refused connection, a timeout, a rejection from the far end. A run of failures against one webhook usually means the receiver is down or its URL has changed; failures scattered across all of them mean the problem is on this side.

## Latest {#latest}

Every delivery, newest first.

## Failed {#failed}

Deliveries that did not succeed.
