# Incident fields

## incident {#incident}

The number itself: a decimal you can read out over a phone. It encodes when it happened, which node handled it and a sequence within that millisecond. See [incident numbers](/help/admin/incidents#incident-numbers).

## kind {#kind}

Which side the failure came from: a device, the management API, this console, or the server itself.

## occurredAt {#occurred-at}

When it happened.

## status {#status}

The response code the request answered with. `status=ge=500` finds server faults.

## method {#method}

The request method.

## path {#path}

What was being asked for. Usually enough on its own to see which device or object was involved.

## errorCode {#error-code}

The category of the failure — see [error categories](/help/reference/errors#categories).

## message {#message}

What the server said. The first thing to read.

## node {#node}

Which node handled the request. A run of incidents from one node and not the others narrows the problem considerably.

## sequence {#sequence}

The counter within the millisecond that makes the number unique. Diagnostic only.
