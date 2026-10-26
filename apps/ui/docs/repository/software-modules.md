# Software modules

> One installable thing, with a name, a version and the files that make it up.

## Software modules {#software-modules}

A module is the unit of software: an operating system image, an application, a firmware blob. It has a name, a version, a vendor and a type, and it holds [artifacts](/help/repository/artifacts#artifacts) — the actual files.

Modules are never assigned to devices directly. They are attached to a [distribution set](/help/repository/distribution-sets#distribution-sets), and the set is what gets assigned. A module can belong to many sets.

## Name and version {#version}

A module is identified by the pair. `agent` version `1.4.0` and `agent` version `1.5.0` are two modules, not two states of one, and both can exist in the repository at the same time.

## Type {#type}

Which slot the module fills — application, operating system, firmware. The type is what a [distribution set type](/help/repository/ds-types#distribution-set-types) refers to when it says a set must contain an operating system.

The type also carries how many modules of that kind one set may hold. See [software module types](/help/repository/sm-types#max-assignments).

## Vendor {#vendor}

Free text: who produced it. Recorded and searchable, used for nothing else.

## Locked {#locked}

A module that has been part of an assignment or a rollout is locked, and its artifacts can no longer be changed. See [locking](/help/repository/locking#implicit-lock).

## Deleting a module {#deleting}

A module that has never been used is removed. A module that a distribution set refers to is marked deleted instead: it disappears from the lists, and the sets that contain it keep working.

## Latest {#latest}

Every module, newest first.

## With artifacts {#with-artifacts}

Modules that hold at least one file — the ones that can actually be installed.

## Without artifacts {#without-artifacts}

Modules with no files yet. A module created and never uploaded to sits here, and a distribution set containing one will hand a device nothing to download.

## Recently changed {#recently-changed}

Modules touched most recently, by modification time rather than creation time.

## By type {#by-type}

Modules grouped by their module type.
