# console-visual-language Specification

## Purpose

The visual and interaction rules that every console page shares: shape,
colour tokens, time display, identifiers, layering and focus, hiding, error
pages, and resilient client-side script.

## Requirements

### Requirement: Rectangular style and tokenised colour
The console SHALL use a strictly rectangular style, so any `border-radius` SHALL be 0. Every colour SHALL be a CSS custom property declared on `:root` in the main stylesheet, and no colour literal SHALL appear outside that block. Every referenced CSS variable SHALL be defined.

#### Scenario: Stylesheet audit
- **WHEN** the stylesheet is checked
- **THEN** it contains no non-zero border-radius, no hex colour outside `:root`, and no undefined `var(--…)`

### Requirement: One time format
Every timestamp SHALL be shown as `HH:MM:SS dd.mm.yy` in the viewer's time zone. The exact instant SHALL be available on hover. Object pages SHALL also show it beside the formatted time. No raw epoch number SHALL appear as visible text.

#### Scenario: Times are human
- **WHEN** any page is crawled
- **THEN** no text node contains a raw epoch number and every time element carries its exact value

### Requirement: Decimal identifiers, no UUIDs
Every identifier shown to an operator SHALL be a decimal number, so it can be selected with a double click and read out. No UUID SHALL appear in the interface. The only random token is the session cookie, which is a secret and not an identifier.

#### Scenario: No UUIDs
- **WHEN** every page is crawled after creating a saved view
- **THEN** no UUID-shaped string appears

### Requirement: Layers, focus and hiding
Pop-up layers SHALL cast a shadow and sit on a declared layer. Pop-up layers include menus, pickers and search results. Buttons and links SHALL have a visible keyboard focus. An element hidden with the `hidden` attribute SHALL be fully hidden, so it cannot sit above content and capture clicks.

#### Scenario: Hidden menu does not eat clicks
- **WHEN** the bulk context menu is closed
- **THEN** it is not displayed and clicks reach the table beneath

### Requirement: No stray log panels
Logs of operator actions SHALL appear only in the Action Log resource, and no other page SHALL carry a log panel.

#### Scenario: Object page
- **WHEN** a target page is rendered
- **THEN** it contains no action log panel

### Requirement: A broken page is an error
A page that fails to render SHALL answer 500 and SHALL never answer 200 with partial content.

#### Scenario: Template failure
- **WHEN** rendering a page fails
- **THEN** the response status is 500

### Requirement: Client script degrades piecewise
Client-side initialisers SHALL run independently, so that one failing widget does not disable the others on the page.

#### Scenario: One widget throws
- **WHEN** one initialiser throws an exception
- **THEN** the remaining initialisers on the page still run
