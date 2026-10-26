# Grants

> Which user holds which role.

## Grants {#grants}

A grant connects a username to a [role](/help/admin/roles#roles). A user with several grants holds the union of their permissions.

A user with no grant at all, in a tenant that has roles, can do nothing beyond signing in.

## Usernames {#usernames}

A grant names a user as they authenticate. It does not create an account — accounts come from wherever authentication is configured. Granting a role to a name that never signs in does nothing and reports no error.

## Removing a grant {#removing}

Takes effect on the user's next request. It does not end their session.

## All {#all}

Every grant in the tenant.
