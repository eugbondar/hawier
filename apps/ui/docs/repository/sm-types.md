# Software module types

> The kinds of module that exist: operating system, application, firmware.

## Software module types {#software-module-types}

A module type is a slot. Every [software module](/help/repository/software-modules#software-modules) has one, and [distribution set types](/help/repository/ds-types#distribution-set-types) are written in terms of them.

## The key {#key}

The short, stable identifier — `os`, `application`, `firmware`. The display name is for people and may be reworded; the key is what devices and other systems see, and changing it changes what they see.

Devices are given the key as the **part** of each chunk they download, so an agent that switches on it depends on the key staying put.

## Max assignments {#max-assignments}

How many modules of this type one distribution set may contain.

One means a set holds at most one module of that kind — the usual choice for an operating system. A larger number lets a set carry several, which is what you want for applications.

## Min artifacts {#min-artifacts}

How many files a module of this type must hold before it counts as usable.

## Deleting {#deleting}

A type still referred to by modules or by distribution set types is marked deleted rather than removed, so nothing that already depends on it breaks.

## All {#all}

Every software module type in the tenant.
