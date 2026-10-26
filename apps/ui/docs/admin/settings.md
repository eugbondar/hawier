# Tenant configuration

> Fourteen settings that change how the server behaves for this tenant.

## Tenant configuration {#tenant-configuration}

Each row is one setting. A row marked **(default)** is not set for this tenant and is using the server default; saving a value makes it the tenant's own.

The page itself — who you are, which server, which build — is described in [settings](/help/console/settings#system-settings). These change behaviour for every device and every operator in the tenant. Several of them change what happens to updates already in flight.

## Defaults {#defaults}

**Reset** removes the tenant's own value and puts the setting back to the server default. It does not set it to empty — it stops overriding.

## pollingTime {#polling-time}

How long a device is told to sleep between polls, as `HH:MM:SS`. The default is five minutes.

This is the single biggest lever on how responsive the fleet feels and how much load it puts on the server. Five seconds makes assignments feel instant and multiplies request volume by sixty; an hour makes the server almost idle and means an urgent update takes up to an hour to start.

## pollingOverdueTime {#polling-overdue-time}

How long after an expected poll a device counts as [overdue](/help/glossary#overdue).

It only affects reporting — nothing is cancelled or retried because a device went overdue. Set it longer than the poll interval, or a fleet with normal jitter will flicker between responding and overdue.

## maintenanceWindowPollCount {#maintenance-window-poll-count}

How many times a device should poll in the run-up to a [maintenance window](/help/deployment/maintenance-windows#maintenance-windows) opening.

As a window approaches, the server shortens the sleep it hands the device so it is awake when the window opens rather than sleeping through it. There is a floor of thirty seconds, so this cannot be used to make a fleet poll continuously.

## authentication.targettoken.enabled {#authentication-targettoken-enabled}

Whether devices may authenticate with their own per-device [security token](/help/inventory/targets#security-token).

The stronger of the two schemes: each device has its own secret, and a leaked one compromises one device.

## authentication.gatewaytoken.enabled {#authentication-gatewaytoken-enabled}

Whether devices may authenticate with the tenant-wide gateway token.

This is what makes plug-and-play registration possible: a device with the shared token can introduce itself and be created on the spot. It is also the weaker scheme — one secret for the whole tenant, and a leak compromises all of it.

## authentication.gatewaytoken.key {#authentication-gatewaytoken-key}

The shared token itself. Treated as a secret and masked in the interface.

Changing it locks out every device still using the old value until they are reconfigured.

## rollout.approval.enabled {#rollout-approval-enabled}

Whether a new rollout must be approved before it can start.

On, a rollout is built and then waits at [waiting_for_approval](/help/glossary#waiting-for-approval) for someone to [approve or deny](/help/deployment/rollout-lifecycle#approve) it, with a remark recorded either way. Off, it goes straight to ready.

## repository.actions.autoclose.enabled {#repository-actions-autoclose-enabled}

What happens to an action that a new assignment supersedes.

Off (the default), the old action is asked to cancel and waits for the device to acknowledge. On, it is closed as [canceled](/help/glossary#canceled) immediately with no handshake — faster and tidier, but the device is never told, and it may install what it had already downloaded.

## user.confirmation.flow.enabled {#user-confirmation-flow-enabled}

Whether new actions wait to be confirmed before the device may install. See [confirmation](/help/deployment/confirmation#the-confirmation-flow).

Switching it on affects new actions only.

## multi.assignments.enabled {#multi-assignments-enabled}

Whether a device may carry several active actions at once.

Off, a new assignment supersedes the previous one. On, nothing supersedes anything and the device is served its actions one at a time, highest [weight](/help/deployment/assignment#weight) first.

> This one is effectively one-way. Switching it on changes what the outstanding work on every device means; plan it, do not experiment with it on a live fleet.

## batch.assignments.enabled {#batch-assignments-enabled}

Declared for compatibility. Nothing in this server reads it, and changing it has no effect.

## implicit.lock.enabled {#implicit-lock-enabled}

Whether assigning or rolling out a distribution set freezes its contents. On by default — see [locking](/help/repository/locking#implicit-lock).

Turning it off lets the contents of a set change after devices have installed it, which means the record of what a device installed stops meaning anything. There is very rarely a good reason.

## target.type.required {#target-type-required}

Whether a device must have a [target type](/help/inventory/target-types#target-types) to receive anything. Off by default.

Off, a device without a type accepts any distribution set — the usual behaviour, and what other clients of this API expect. On, it accepts none: assignments to it are refused as **not compatible**, and rollouts and automatic assignment pass it by. Typed devices are unaffected either way — they accept what their type [declares compatible](/help/inventory/target-types#compatible-types).

Switch it on when a wrong image on the wrong hardware must be impossible rather than unlikely: give every device its type when you create it, and nothing reaches a device whose hardware the server does not know.

## action.cleanup.auto.expiry {#action-cleanup-auto-expiry}

How old a closed action must be before housekeeping deletes it.

Unset means never. Cleanup only runs when this **and** the status list below are both set.

## action.cleanup.auto.status {#action-cleanup-auto-status}

Which closed actions housekeeping may delete, as a list of status names — typically the finished and cancelled ones.

Deleting actions deletes the history of what devices did. Keep failures long enough to investigate them.
