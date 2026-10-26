# Incidents

> Every failure gets a number, and the number leads to what happened.

## Incidents {#incidents}

When a request fails, the answer carries a decimal number. That number is an incident, and it is recorded here with the request that produced it.

It is a diagnostic record, not an alert queue: nothing is assigned, acknowledged or closed. It is how you turn "it said something went wrong" into "at 14:02 on node 3, this request answered 500 with this message".

## Incident numbers {#incident-numbers}

The number is a plain decimal you can read out over a phone. Encoded in it are the moment it happened, which node handled the request, and a counter within that millisecond — which is what makes it unique across a cluster without any coordination.

The panel on an incident page decodes those parts, and [identifiers](/help/reference/ids#encoding) explains why they are shaped that way. Which [node](/help/admin/nodes#which-node) produced it is part of the record.

To look one up, paste it into the incidents list. Every request carries its number in a response header, whether it failed or not.

## What gets recorded {#what-is-recorded}

Server faults are always recorded. Rejected requests from the console and the API are recorded too. Rejected requests from **devices** are not: a fleet with a misconfigured agent would otherwise fill the journal with the same rejection a million times. Those still get a number — they are just not kept.

## Kind {#kind}

Which side the failure came from: a device, the API, this console, or the server itself. Console noise is separated out on the overview for exactly this reason — a rendering slip here is not the same news as a device being turned away.

## Important {#important}

Server faults from devices and the API — the failures that mean something is broken rather than someone mistyping a query.

## All {#all}

Every recorded incident.

## Device incidents {#ddi}

Failures on the device-facing side.

## API incidents {#mgmt}

Failures on the management API — usually another system integrating with this one.

## Console incidents {#console}

Failures inside this console. Rarely urgent, but a run of them is worth reporting.

## Server faults {#server-faults}

Everything that answered with a server error, whatever the source.
