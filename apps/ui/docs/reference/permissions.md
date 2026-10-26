# Permissions

> The sixteen permissions a role can carry.

## The permissions {#the-permissions}

Grouped by what they cover. A [role](/help/admin/roles#roles) carries any set of them.

## Devices {#devices}

Read targets :: See devices, their actions and their attributes.
Create targets :: Create device records.
Update targets :: Change devices — tags, types, groups, assignments.
Delete targets :: Remove device records.

These also cover [target filters](/help/deployment/target-filters#target-filters) and [groups](/help/inventory/target-groups#groups).

## Repository {#repository}

Read repository :: See distribution sets, modules, artifacts, tags and types.
Create repository :: Create them and upload artifacts.
Update repository :: Change them — attach modules, invalidate sets, edit tags.
Delete repository :: Remove them.

Anything not covered by another group falls here, which is deliberate: a new kind of object is protected by default rather than open by default.

## Rollouts {#rollouts}

Read rollouts :: See rollouts and their groups.
Create rollouts :: Build new ones.
Update rollouts :: Change them.
Delete rollouts :: Remove them.
Handle rollouts :: **Start, pause, resume, stop, approve, deny and trigger the next group.**

Handling is the one to think about. It is the permission to act on the whole fleet, and it is separate from creating a rollout on purpose: building a plan and executing it can belong to different people.

## System {#system}

Read system :: See tenant configuration, roles, grants, webhooks, retention rules, incidents and the action log.
Update system :: Change all of the above.

Update system includes roles and grants, so it is the permission to change who can do what. Treat it as the administrator permission.

## Everything {#all}

A single permission covering all of the above, for the account that administers the tenant.

## No roles means no restrictions {#no-roles}

While a tenant has no roles at all, none of this applies and every signed-in user can do everything. See [roles](/help/admin/roles#unconfigured).
