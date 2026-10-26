# Webhook fields

## url {#url}

Where deliveries are posted. Use HTTPS.

## eventMask {#event-mask}

Which [events](/help/admin/webhooks#events) this subscription wants: `*` or a comma-separated list. Validated when saved.

## active {#active}

Whether the subscription delivers. Off keeps the settings and sends nothing.

## signed {#signed}

Whether a signing secret is set. An unsigned webhook gives its receiver no way to tell a real delivery from anything else that can reach the URL — see [signing](/help/admin/webhooks#secret).

The secret itself is never shown; only whether one exists.
