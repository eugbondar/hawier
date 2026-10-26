# Webhook delivery fields

## deliveredAt {#delivered-at}

When the attempt was made.

## webhookName {#webhook-name}

Which subscription it belongs to, as a link.

## kind {#kind}

Which [event](/help/admin/webhooks#events) was being delivered.

## event {#event}

The identity of the event itself, for matching against what the receiver recorded.

## status {#status}

Whether the attempt succeeded.

## error {#error}

What went wrong. A run of failures against one subscription usually means its receiver is down or its URL has changed.

## durationMs {#duration-ms}

How long the attempt took, in milliseconds. A slow receiver shows up here before it starts failing.
