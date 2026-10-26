# Targets

> A target is a device: an identity, a poll schedule, and whatever it last told this server about itself.

## Targets {#targets}

Every device this server knows about is a target. The list is the fleet.

A target is created one of two ways. Either the device polls with an identity the server has not seen and is registered on the spot, or someone creates the record in advance through this console or the API. A record created in advance is inert until the real device turns up and starts polling.

## The controller id {#controller-id}

The device's own name for itself, and the only identity that matters. It appears in every URL the device uses, and it is what you type when you look a device up.

It is chosen by whoever provisions the device — a serial number, a MAC address, an asset tag. The server never assigns it.

## Update status {#update-status}

Where the device stands relative to what it was told to install: [unknown](/help/glossary#unknown), [registered](/help/glossary#registered), [pending](/help/glossary#pending), [in_sync](/help/glossary#in-sync) or [error](/help/glossary#error).

This is a summary of the device, not of any one action. A device with three finished actions and one failed one is in error until something clears it.

## Poll state {#poll-state}

Whether the device is talking to this server on schedule: **responding**, [overdue](/help/glossary#overdue), or **never polled**.

Overdue is computed from the tenant's poll settings, not from the device: it means the server expected to hear from it by now. A device on a deliberately slow schedule can read overdue and be perfectly healthy — see [polling](/help/admin/settings#polling-overdue-time).

## Security token {#security-token}

A per-device secret the device may use to authenticate its polls. It is generated when the device is registered, and it is masked on the page until you reveal it deliberately.

Whether devices must present it is a tenant setting — see [target token authentication](/help/admin/settings#authentication-targettoken-enabled). The alternative is a single tenant-wide gateway token, which is what makes plug-and-play registration possible.

> Revealing the token puts it on screen. Anyone who can see the screen can then use it as that device.

## Assigned and installed {#assigned-and-installed}

**Assigned** is what the device was told to run. **Installed** is what it has reported running. They differ while an update is in flight, and they are the fastest way to see whether a fleet has actually landed.

Both are links: the name goes to the distribution set.

## Registering a new device {#registration}

If the tenant allows a shared gateway token, a device that polls with an identity nobody has seen is created automatically, given a token of its own, and asked to report its attributes. It arrives as [registered](/help/glossary#registered) with nothing assigned.

If it does not, the target must exist before the device can poll successfully.

## The views {#the-views}

## All {#all}

Every device in the tenant, newest first. The unfiltered list.

## Recently seen {#recently-seen}

Devices that have polled most recently, first. Useful for confirming that something you just plugged in is talking.

## Needs attention {#needs-attention}

Devices in [error](/help/glossary#error) or [pending](/help/glossary#pending) — everything that is not settled. The list to work through after a rollout.

## Pending update {#pending-update}

Devices with an update assigned and not yet finished. A device that stays here is not picking its action up — see [troubleshooting](/help/start/troubleshooting#not-picking-up).

## Error {#error}

Devices whose last update attempt failed. Open one and read the action's status history: the messages come from the device.

## Unassigned {#unassigned}

Devices with no distribution set assigned at all. A newly provisioned fleet is entirely here.

## By tag {#by-tag}

Devices grouped by the tags they carry, so you can see what a tag actually covers.

## By group {#by-group}

Devices ordered by their [group path](/help/inventory/target-groups#groups).

## Responding {#responding}

Devices that are polling on schedule. The healthy part of the fleet.

## Not responding {#not-responding}

Devices that have not polled within the expected window. Either they are switched off, or they cannot reach this server — see [poll state](#poll-state).
