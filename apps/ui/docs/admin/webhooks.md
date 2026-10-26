# Webhooks

> Tell another system when something happens here.

## Webhooks {#webhooks}

A webhook is a URL this server posts to when a chosen event occurs, and an optional secret used to sign what it sends.

Events are written down as they happen, in the same transaction as the change that caused them, and delivered afterwards. A change is never lost because a delivery failed.

## Events {#events}

Seven kinds, and a webhook subscribes to any set of them.

rollout.created :: A rollout was built.
rollout.started :: A rollout began sending.
rollout.paused :: A rollout stopped — by hand or by an error threshold.
rollout.finished :: A rollout completed every group.
assignment.created :: A device was assigned a distribution set.
artifact.uploaded :: A file was added to a module.
artifact.deleted :: A file was removed.

## Event mask {#event-mask}

Which of those to send: `*` for everything, or a comma-separated list. It is checked when you save, so a typo is refused rather than quietly delivering nothing.

## Signing secret {#secret}

When set, each delivery carries a signature header — a keyed hash of the body. The receiver recomputes it with the same secret and rejects anything that does not match.

Without a secret there is no signature, and the receiving end has no way to tell a real delivery from anything else that can reach its URL. The list marks unsigned webhooks for exactly that reason.

The secret is write-only: it can be set and replaced, never read back.

## Active {#active}

Whether the webhook is delivering. An inactive webhook keeps its settings and sends nothing.

## Nothing is delivered {#no-sink}

Delivery itself is done by an [extension](/help/admin/extensions#extensions). Without one, events accumulate and nothing is sent. The [deliveries](/help/admin/webhook-deliveries#deliveries) list being empty while events are clearly happening is the symptom.

## All {#all}

Every webhook in the tenant.

## Paused {#paused}

Webhooks that are switched off.

## Unsigned {#unsigned}

Webhooks with no signing secret. Their receivers cannot verify what they get.
