# Errors and limits

> What a failure tells you, and the ceilings you can run into.

## When something fails {#failures}

A failed request answers with a category, a message and an [incident number](/help/admin/incidents#incident-numbers). The console shows the message where the data would have been and leaves the rest of the page alone — the toolbar, the filters and the navigation stay usable.

## The categories {#categories}

Not found :: The object does not exist, or was deleted.
Invalid parameter :: Something in the request was wrong — usually a query or a sort order.
Unauthorized :: Not signed in, or the session expired.
Forbidden :: Signed in, but the [role](/help/admin/roles#roles) does not carry the permission.
Already exists :: A name that must be unique is taken.
Not cancelable :: The action has already closed. Nothing to cancel.
Not compatible :: The device's [target type](/help/inventory/target-types#compatible-types) does not accept the set's type, or the device has no type and the tenant [requires one](/help/admin/settings#target-type-required). Nothing in the request was assigned.
Quota exceeded :: A limit was reached — see below.
Invalid query :: The query did not parse, or named a field that does not exist. The message says where.
Service unavailable :: A part of the system is missing or down. Usually a missing [extension](/help/admin/extensions#no-extension-no-fallback).
Internal :: A fault in the server. Always recorded as an incident.

## Limits {#limits}

Status reports per action :: An action holds a bounded number of status rows — eight hundred. Repeated identical reports are collapsed rather than stored again, which is what normally keeps a device well under it. A device in a loop reporting **different** things can exhaust it, and its later reports are then refused.
Rows per page :: A list request may ask for at most five hundred rows. The console's page size tops out at a hundred.
Artifact size :: Uploads are capped by the node's configuration, a gigabyte by default.
Rollout groups :: Between 1 and 500.

## Reporting a problem {#reporting}

Quote the incident number. It identifies the exact request, the node and the millisecond, and it is the difference between a report that can be investigated and one that cannot.
