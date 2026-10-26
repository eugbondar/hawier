# Spec Delta

## Purpose

The in-console manual and the "what is this?" mode. Everything an operator can
point at in the console is explained, and missing documentation is a test
failure, not a gap found later.

## ADDED Requirements

### Requirement: Built-in manual
The console SHALL serve a manual compiled into its binary from markdown chapters. It SHALL serve:
- `/help`: a table of contents with `?q=` search
- `/help/{topic}`: one chapter on one page, with sections separated by headings rather than boxes, and an anchor per heading
- `/help/glossary`: the status glossary

Each chapter SHALL start with a note that it was written by AI. The manual SHALL NOT contain the word `hawkBit`.

#### Scenario: Chapter is one page
- **WHEN** any help topic is rendered
- **THEN** it is a single panel listing every section anchor of the chapter

### Requirement: What-is-this mode
Every page SHALL carry help keys of the form `topic#anchor`, plus a page-level key. The `?` control in the top bar, or Shift+F1, SHALL arm help mode. The next click SHALL show the help for the nearest element with a key, taken from `/help/panel?key=`. Ctrl/Cmd SHALL keep the mode armed, and Esc SHALL disarm it. An unknown key SHALL get a plain "nothing written" answer, not an error. Without JavaScript, `?` SHALL be a link to `/help`.

#### Scenario: Click anywhere
- **WHEN** help mode is armed and the operator clicks a column header
- **THEN** the panel shows the manual section for that column

#### Scenario: Every key resolves
- **WHEN** all help keys on all pages are collected
- **THEN** each resolves to an existing section or to a declared fallback

### Requirement: Every declared thing is documented
Every declared item SHALL have a manual section whose anchor is derived from its name:
- resource, system view, column, form field, bulk operation
- every value of a status enum the server can report

A new item without its section SHALL fail the documentation audit.

#### Scenario: Undocumented column
- **WHEN** a column is added to a resource without a section for it in the manual
- **THEN** the documentation audit fails, naming the column

### Requirement: One colour table for status words
The mapping from a status word to its colour class (`ok`, `warn`, `error`, `neutral`, `attention`) SHALL come from one table. List badges, object page pills and the glossary SHALL use it. Spelling variants of one word SHALL map identically; `in_sync`, `IN-SYNC` and `InSync` are examples. Every status value the server can report SHALL have an entry.

#### Scenario: Same word, same colour
- **WHEN** `paused` appears in a rollout list badge and on a rollout page pill
- **THEN** both carry the class `warn`
