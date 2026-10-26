# The type form

> A name, a stable key, and for module types a limit.

## The form {#the-form}

Creates a target type, a distribution set type or a software module type, depending on which list you started from.

## Name {#name}

The display name. For people, and it may be reworded later.

## Key {#key}

The short, stable identifier — `os`, `application`, `gateway-v2`. Unlike the name it is what other systems and devices see, so **pick it carefully and do not change it**: changing a module type's key changes what devices are told about the modules they download.

Lower case, no spaces, is the usual convention.

## Description {#description}

Free text. What this type covers.

## Colour {#colour}

A colour for the type, so it stands out in a list. Cosmetic.

## Max assignments (SM types) {#max-assignments}

Software module types only, and ignored for the others.

How many modules of this type one distribution set may hold. One is right for an operating system — a set should not contain two. A larger number lets a set carry several, which is what you want for applications.
