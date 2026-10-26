# Spec Delta

## Purpose

The optional TLS front for a deployed stack. Caddy obtains and renews
certificates by ACME, routes device and operator traffic, and sets the two
forwarded headers that the node relies on to build links and to identify
devices.

## ADDED Requirements

### Requirement: Caddy in front when TLS is enabled
With `hawier_tls: true`:
- the deploy SHALL run Caddy on ports 80 and 443
- Caddy SHALL obtain and renew certificates by ACME, with `hawier_tls_email` optional
- the node and the console SHALL be published only on loopback, whatever publish address is configured
- certificates and the ACME account SHALL live in a dedicated `caddy-data` volume

#### Scenario: Loopback enforced
- **WHEN** TLS is on and `hawier_publish_server` is `0.0.0.0:8080`
- **THEN** the node is published on `127.0.0.1:8080` only

### Requirement: Forwarded headers
Every proxied request SHALL carry `X-Forwarded-Proto` set to the client scheme. Every proxied request SHALL also carry `X-Forwarded-For` overwritten with the client address, never appended to. The node builds all `_links` from the first, and takes the first hop of the second as the device address.

#### Scenario: Spoofed client address
- **WHEN** a device sends `X-Forwarded-For: 6.6.6.6` through the proxy
- **THEN** the node sees the device's real address

#### Scenario: HTTPS links
- **WHEN** a device polls through the proxy over HTTPS
- **THEN** every link in the response starts with `https://`

### Requirement: Routing
If the console and the device API share one name, the proxy SHALL route by path:
- DDI paths `/{tenant}/controller/v1/…`, `/rest/v1` and `/health` go to the node
- everything else goes to the console

If the two names differ, each name SHALL be its own site. Responses on DDI and management paths SHALL NOT be compressed.

#### Scenario: Single name
- **WHEN** one name serves both and a request for `/DEFAULT/controller/v1/dev-1` arrives
- **THEN** it reaches the node, while `/targets` reaches the console

### Requirement: Configuration changes are applied
After each deploy, the proxy SHALL be told to reload its configuration. If the reload fails, the proxy container SHALL be restarted, so that an edited configuration never sits unapplied.

#### Scenario: Only routing changed
- **WHEN** a deploy changes only the proxy configuration
- **THEN** the new routing is live without recreating the container
