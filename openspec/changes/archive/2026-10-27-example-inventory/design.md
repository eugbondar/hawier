# Design

## One place to edit

The tracked `deploy/ansible/inventory.yml` is the place a new user edits. A
separate untracked "real" inventory with a tracked example would add a copy
step before the first deploy and leave the user editing a file that does not
exist in a fresh clone. Instead the tracked file carries a working entry whose
only wrong value is the address.

The maintainer's own hosts move to `deploy/ansible/inventory.local.yml`,
ignored by git. make passes it as a second `-i` when it exists; ansible merges
the two, so a host in either file is deployed, listed and diagnosed the same
way.

## Dirty tree

The dirty check exists because the version (`vYY.MM.DD-<sha>`) must describe
what is shipped. The images are built from a context that excludes `deploy/`,
so the inventory cannot change them; excluding it from the check keeps the
guarantee and lets the user deploy right after editing the address. The local
inventory is untracked and never counted.

## Placeholder refusal

The check is the first task of preflight and is an `assert`, which runs on the
controller: an unedited `server` entry fails in a second with a sentence that
names the file, instead of an ssh timeout against a name that does not exist.
Only documentation addresses are refused (RFC 2606 `example.com/net/org` and
their subdomains, RFC 5737 `192.0.2.0/24`, `198.51.100.0/24`,
`203.0.113.0/24`): they can never be a real host, so the check cannot refuse a
real one.

## Resolved divergence

Item 29 of the baseline catalog (a deploy log tracked by git because
`.gitignore` missed `*.log.ansible`) is resolved: the pattern is ignored and
the file is removed from the index.
