# Action fields

## controllerId {#controller-id}

Which device this job belongs to.

## status {#status}

The coarse state: [pending](/help/glossary#pending) while the action is active, [finished](/help/glossary#finished) once it is closed. **Whether it succeeded is not in this word** — a failed action is `finished` here and `error` in [detailStatus](#detail-status).

## detailStatus {#detail-status}

The precise state — scheduled, retrieved, download, downloaded, running, wait for confirmation, canceling, cancel rejected, canceled, finished or error. See [action state](/help/deployment/actions#state).

## type {#type}

[update](/help/glossary#update) — install something — or [cancel](/help/glossary#cancel), the job of stopping one.

## forceType {#force-type}

How hard the device is being pushed: [forced, soft, time-forced or download-only](/help/deployment/action-types#action-types).

## rollout {#rollout}

The rollout that created this action, if any. `rollout!=null` finds every action a rollout produced.

## rolloutName {#rollout-name}

The same rollout by name, as a link.

## weight {#weight}

Priority against other actions on the same device. See [weight](/help/deployment/assignment#weight).

## externalRef {#external-ref}

A reference set by whatever created the action through the API. Free text; the console never sets it.

## lastStatusCode {#last-status-code}

The last code the device reported. Its meaning is defined by the device's agent, not by this server.
