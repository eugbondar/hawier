# Settings

> Who you are, which server you are talking to, which build each half is running, and the tenant configuration.

## System settings {#system-settings}

The first panel names the tenant and user you are signed in as, and the two addresses this console talks to: the Management API and the console state API.

## Build {#build}

Each half of the system prints its own build string, `vYY.MM.DD-` followed by the commit it was built from. After a deployment the two should match. A mismatch here is the answer to "why does the console not know about that field" — it is running an older build than the server, or the other way round.

A **debug** marker next to a build means that half was compiled without optimisation. It behaves identically and runs several times slower, so it answers "why is this sluggish" — and it means any timing you take from it is meaningless.

## Tenant configuration {#tenant-configuration}

Settings that change how the server behaves for this tenant. A row marked **(default)** is not set for the tenant and is using the server default; saving a value makes it the tenant's own, and **Reset** puts it back to the default.

Each key is documented in [tenant configuration](/help/admin/settings#tenant-configuration).
