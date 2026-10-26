# Roles

> Named sets of permissions. Nobody is restricted until at least one exists.

## Roles {#roles}

A role is a name and a list of permissions. Users are connected to roles by [grants](/help/admin/grants#grants).

## A tenant with no roles allows everything {#unconfigured}

> While a tenant has **no roles at all**, permission checks do not apply and every signed-in user can do everything. Authorisation is opt-in: creating the first role is what switches enforcement on.

That is a deliberate choice — a fresh tenant is usable before it is configured — but it means "we have not set up roles yet" and "everyone is an administrator" are the same state. Check the [roles](/roles) list before assuming the tenant is locked down.

## Permissions {#permissions}

Permissions are grouped by what they cover — see [the permission reference](/help/reference/permissions#the-permissions).

Devices :: reading, updating, creating and deleting targets, filters and groups.
Repository :: reading, updating, creating and deleting distribution sets, modules, artifacts, tags and types.
Rollouts :: reading, creating, updating, deleting — and **handling**, which is the one that covers starting, pausing, approving and stopping.
System :: reading and updating tenant configuration, roles, webhooks, retention rules, incidents and the action log.

Rollout **handling** is worth separating: a person who may look at rollouts and a person who may start one across the whole fleet are different people.

## Who you are {#who-you-are}

The foot of the navigation column shows the tenant and the user you are signed in as. Everything you do is recorded against that name in the [action log](/help/admin/action-log#the-action-log).

Signing out ends the session. Sessions are held by a cookie and nothing else about your identity is kept in the browser.

## All {#all}

Every role in the tenant.

## Unheld {#unheld}

Roles nobody holds. Either a mistake, or a role someone defined and never granted — and either way it is not protecting anything.
