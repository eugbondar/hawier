# Confirmation

> An extra gate: something has to say yes before the device may install.

## The confirmation flow {#the-confirmation-flow}

When the tenant switches confirmation on, a new action does not go straight to work. It waits at [wait_for_confirmation](/help/glossary#wait-for-confirmation) until it is confirmed.

Who confirms is up to the deployment: the device's own agent through its channel, an operator with a person in front of the machine, or nobody — in which case it waits.

The gate applies per action. A rollout group can waive it, and a device can waive it permanently.

## Confirmation required {#confirmation-required}

The switch on one assignment, one auto-assignment rule, or one rollout group. On means this work waits for a yes; off means it starts as usual even while the tenant flow is on.

## Auto-confirmation {#auto-confirmation}

A standing "yes" for one device. With it active, that device's actions skip the gate entirely and start as if the flow were off.

It records who activated it and why, so an exemption has an owner. Deactivating it puts the device back behind the gate for future actions; actions already running are unaffected.

## Denial {#denial}

A confirmation can be denied as well as given. A denied action does **not** fail, and does not go away: it stays at [wait_for_confirmation](/help/glossary#wait-for-confirmation) indefinitely.

> A denial is not a decision the system acts on — it is a refusal to decide. Cancel the action if you want it gone. See [sharp edges](/help/reference/sharp-edges#denied-confirmation).
