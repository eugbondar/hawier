# The distribution set form

> Name, version, type. The modules are attached afterwards.

## The form {#the-form}

Creates the empty bundle. It has no modules yet and is therefore [incomplete](/help/repository/distribution-sets#complete) — attach them on the set's page afterwards.

## Name {#name}

What the release is called. Together with the version, this is how the set is identified.

## Version {#version}

The version of this set. A set is identified by name and version together, so `platform` 1.4 and `platform` 1.5 are two sets, not two states of one.

## Type {#type}

Which [distribution set type](/help/repository/ds-types#distribution-set-types) this is. The type decides which module types the set must contain before it counts as complete, and which devices it may be installed on.

It cannot be changed once modules are attached, so choose it before you fill the set.

## Description {#description}

Free text. What is in this release and what changed.

## Required migration step {#required-migration-step}

Marks the set as a version that must not be skipped over. It is recorded and handed to the device; what the device does with it is up to its agent.

Leave it off unless your agent implements it.
