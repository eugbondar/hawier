# The role form

> A name and a list of permissions.

## The form {#the-form}

Creates a named set of permissions. It affects nobody until someone is [granted](/help/admin/grants#grants) it.

> Creating the **first** role in a tenant switches authorisation on for everybody. Before that, every signed-in user can do everything. Make sure your own account has a grant before you create the first role, or you may lock yourself out.

## Name {#name}

What the role is called. Name it after the job — `fleet-operator`, `release-manager`, `read-only`.

## Permissions {#permissions}

The permissions this role carries, as a list — see [the permission reference](/help/reference/permissions#the-permissions). Names are validated when you save, so a typo is refused rather than silently granting nothing.

Two worth thinking about separately: **handling** rollouts is the permission to start, pause and stop them across the whole fleet, which is not the same as being allowed to create one. And the system permissions cover tenant configuration, roles and grants — that is the permission to change who can do what.

## Description {#description}

Free text. Who this role is for and why it exists.
