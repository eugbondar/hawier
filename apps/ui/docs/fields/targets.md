# Target fields

> Every field on a device, and what it is telling you.

## controllerId {#controller-id}

The device's own identity, and the only one that matters. Chosen when the device was provisioned; the server never assigns it. See [the controller id](/help/inventory/targets#controller-id).

## group {#group}

The single [group path](/help/inventory/target-groups#groups) this device is in, like `eu/west/gw`. A device is in at most one.

## updateStatus {#update-status}

Where the device stands relative to what it was told to install: [unknown](/help/glossary#unknown), [registered](/help/glossary#registered), [pending](/help/glossary#pending), [in_sync](/help/glossary#in-sync) or [error](/help/glossary#error).

A summary of the device, not of any one action.

## pollState {#poll-state}

Whether the device is calling in on schedule: **responding**, [overdue](/help/glossary#overdue), or **never polled**.

Computed from the tenant's [poll settings](/help/admin/settings#polling-overdue-time), not reported by the device. A device on a deliberately slow schedule can read overdue and be healthy.

## lastControllerRequestAt {#last-controller-request-at}

When the device last called in. The single most useful field on a device: an update that is not happening is almost always a device that is not polling.

## assignedDistributionSet {#assigned-distribution-set}

The [distribution set](/help/repository/distribution-sets#distribution-sets) the device has been told to install, by id. Empty means nothing is assigned — `assignedDistributionSet==null` finds every device nobody has given anything to.

## assignedDistributionSetName {#assigned-distribution-set-name}

The same set by name, as a link. What the device is **supposed** to be running.

## installedDistributionSet {#installed-distribution-set}

The set the device has reported actually running, by id.

## installedDistributionSetName {#installed-distribution-set-name}

The same by name. When this and the assigned set differ, an update is in flight — or stuck.

## installedAt {#installed-at}

When the device last reported a completed installation.

## ipAddress {#ip-address}

Where the device was last seen from, as recorded by the server.

## address {#address}

The address recorded for the device. Usually reported by the device rather than typed.

## securityToken {#security-token}

The device's own authentication secret, masked until deliberately revealed. See [security token](/help/inventory/targets#security-token).

## targetType {#target-type}

The [target type](/help/inventory/target-types#target-types) of the device, by id. Restricts which distribution set types may be installed on it.

## targetTypeName {#target-type-name}

The same type by name.

## autoConfirmActive {#auto-confirm-active}

Whether this device is exempt from the [confirmation gate](/help/deployment/confirmation#auto-confirmation).

## requestAttributes {#request-attributes}

Whether the server is asking the device to report its [attributes](/help/inventory/identity#attributes) on its next poll.

## tag {#tag}

Used in queries rather than shown as a column: `tag==canary` selects every device carrying that [tag](/help/inventory/target-tags#tags).

## attribute {#attribute}

Written with a key: `attribute.hw==rev2`. Facts the device reported about itself.

> Always compared as text, whatever they look like — see [sharp edges](/help/reference/sharp-edges#numeric-attributes).

## metadata {#metadata}

Written with a key: `metadata.owner==logistics`. Your own notes on the device. See [metadata](/help/inventory/identity#metadata).
