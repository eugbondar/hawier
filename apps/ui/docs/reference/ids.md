# Identifiers

> Every identifier an operator sees is a decimal number or a name. Nothing else.

## What you see {#what-you-see}

Objects are identified either by a name you chose — a controller id, a distribution set name and version, a tag — or by a plain decimal number.

There are no long hexadecimal strings anywhere in this interface, deliberately. A number can be double-clicked to select, read out over a phone and typed back in without error. That is what an identifier is for.

## Numbers you can look up {#looking-up}

An id from a list can be pasted into that list's filter panel as `id==...` to find the row again. An [incident number](/help/admin/incidents#incident-numbers) can be pasted into the incidents list.

## What the numbers encode {#encoding}

Some numbers — incidents in particular — are not counters. They encode the moment they were created, which node created them, and a sequence within that millisecond, which is how separate nodes can hand out unique numbers without asking each other.

The incident page decodes those parts for you.

## The one secret {#the-one-secret}

A session cookie is random and long, and it is not an identifier: it is a secret. So is a device's [security token](/help/inventory/targets#security-token) and a webhook's [signing secret](/help/admin/webhooks#secret). Those are masked until you reveal them deliberately.
