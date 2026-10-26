# The target form

> Creating a device record by hand.

## The form {#the-form}

Creates the record before the device exists. It is inert until the real device polls with the same controller id.

If the tenant allows plug-and-play registration, you usually do not need this at all — the device creates itself on its first poll.

## Controller ID {#controller-id}

The device's identity, and the only field that must match reality. The real device will present this exact string when it polls; anything else creates a second, separate record.

Chosen by whoever provisions the device — a serial number, a MAC address, an asset tag. See [the controller id](/help/inventory/targets#controller-id).

## Name {#name}

A readable label. Free text, and it may be changed at any time — the identity is the controller id, not this.

## Description {#description}

Free text. Where it is, what it does, who owns it.

## Security token {#security-token}

The device's own authentication secret. Leave it empty and one is generated.

Fill it in only when the device has already been provisioned with a specific token and cannot be changed. See [security token](/help/inventory/targets#security-token).

## Address {#address}

Where the device was last seen from. Normally reported by the device itself, not typed here.
