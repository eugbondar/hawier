# Spec Delta

## Purpose

Every hawier component reports which source it was built from and with which
profile. This lets a mismatch after a deploy, or a debug build in a
measurement, be seen and not guessed.

## ADDED Requirements

### Requirement: Build stamp
Every binary SHALL embed a build stamp `v{YY.MM.DD}-{sha}`. The binaries are the server, the console and the simulator.
- The sha SHALL come from `GIT_SHA`, else from git (9 hex digits), else `nogit`.
- The date SHALL come from `BUILD_DATE`, else the commit date, else today in UTC.
- Container builds SHALL receive both values as build arguments, so an image built without `.git` carries the same stamp as a local build of that commit.

#### Scenario: Image build
- **WHEN** an image is built with `GIT_SHA=abc123def` and `BUILD_DATE=26.10.26`
- **THEN** the binaries inside report `v26.10.26-abc123def`

### Requirement: Build profile is reported, not configured
The server SHALL report its profile as `debug` when built with debug assertions and as `release` otherwise. It SHALL report the stamp, profile, node id and role in `GET /rest/v1/system/extensions` (`node.build`, `node.profile`, `node.role`) and in `GET /ui/v1/version`. The console SHALL report its own stamp in `GET /api/health`.

#### Scenario: Dev image
- **WHEN** a node built with the default dev profile is asked for its extensions report
- **THEN** `node.profile` is `debug`

### Requirement: Versions are visible side by side
The console settings page SHALL show the console's stamp next to the server's stamp. It SHALL mark a debug build of either with a `debug` pill.

#### Scenario: Mismatch after deploy
- **WHEN** the console was rebuilt but the server was not
- **THEN** `/settings` shows two different stamps
