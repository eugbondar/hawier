# Software module fields

## type {#type}

The [software module type](/help/repository/sm-types#software-module-types) — operating system, application, firmware. Which slot the module fills in a distribution set.

## vendor {#vendor}

Who produced it. Free text, recorded and searchable.

## artifactCount {#artifact-count}

How many files the module holds. `artifactCount=gt=0` finds the modules that can actually be installed; zero means a device would be given nothing to download.

## encrypted {#encrypted}

Whether the module's artifacts are stored encrypted.

## locked {#locked}

Whether the module's contents are frozen because it has been part of an assignment or a rollout. See [locking](/help/repository/locking#implicit-lock).
