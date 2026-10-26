# The webhook form

> Where to post, what to post, and how the receiver knows it was you.

## The form {#the-form}

Creates a subscription: when one of the chosen events happens, this server posts it to your URL.

## Name {#name}

What the subscription is called. Name it after the receiver — `pagerduty-rollouts`, `warehouse-sync`.

## URL {#url}

Where to post. Deliveries go to exactly this address, and failures are recorded in [deliveries](/help/admin/webhook-deliveries#deliveries) rather than retried forever.

Use HTTPS. Without a [signing secret](#secret) the URL is the only thing protecting the receiver, and an unencrypted one is not protecting it at all.

## Event mask {#event-mask}

Which [events](/help/admin/webhooks#events) to send: `*` for everything, or a comma-separated list.

It is validated on save, so a misspelled event name is refused rather than quietly delivering nothing.

```
*
rollout.started,rollout.finished,rollout.paused
artifact.uploaded
```

## Signing secret {#secret}

A shared secret. When set, every delivery carries a signature the receiver can recompute and check.

Without it, anything that can reach your URL can pretend to be this server. Set one.

Write-only: it can be replaced, never read back.

## Active {#active}

Whether the subscription delivers. Off keeps the settings and sends nothing.
