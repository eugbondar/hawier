# Spec Delta

## Purpose

`hawier-device` is a DDI device simulator and load client. It is both the
integration-test driver and the load tool, and it is the reference client for
the `_links` doctrine.

## ADDED Requirements

### Requirement: Command-line surface
The simulator SHALL accept the following flags:

| Flag | Default | Meaning |
|---|---|---|
| `--url` | `HAWIER_DEVICE_URL` env, else `http://localhost:8080` | server address |
| `--tenant` | `DEFAULT` | tenant |
| `--controller-id` | `hawier-device-1` | controller id |
| `--token` | none | sent as `Authorization: TargetToken`; preferred over `--gateway-token` when both are given |
| `--gateway-token` | none | sent as `Authorization: GatewayToken` |
| `--poll-override` | none | poll interval with a required unit: `ms`, `s`, `m` or `h` |
| `--count N` | 1 | number of devices |
| `--download-dir` | none | keep downloaded files there; without it they are verified and discarded |
| `--once` | off | run one cycle |
| `--fail-download` | off | fault injection |
| `--deny-confirmation` | off | fault injection |
| `--defer-soft` | off | fault injection |
| `--attributes k=v,...` | none | attributes to report |
| `--poll-only` | off | benchmark mode |

With neither token given, requests SHALL be anonymous. With `--count N` greater than 1, the devices SHALL use ids `{controller-id}-0` through `{controller-id}-(N-1)`.

#### Scenario: Gateway fleet
- **WHEN** `hawier-device --count 3 --controller-id fleet --gateway-token K` runs against a server with gateway tokens enabled
- **THEN** targets `fleet-0`, `fleet-1` and `fleet-2` register themselves on their first poll

#### Scenario: Single cycle
- **WHEN** `--once` is given
- **THEN** the simulator performs one cycle and exits non-zero if the cycle failed

### Requirement: Follow links, build only the poll URL
The simulator SHALL construct only the initial poll URL `{url}/{tenant}/controller/v1/{id}`. Every other request SHALL follow an href received in `_links`. Feedback SHALL be posted to `{href without query and trailing slash}/feedback`, where href is the one through which the resource was fetched.

#### Scenario: Feedback derived from the fetched href
- **WHEN** the deployment was fetched from `.../deploymentBase/7?c=123`
- **THEN** its feedback is posted to `.../deploymentBase/7/feedback`

### Requirement: Update cycle
Each cycle SHALL:
1. Poll.
2. Once per process, report configuration data with mode `merge` when the poll offers `configData` or attributes were given. The report SHALL be `{"hawier.client": <version>}` when no attributes were given.
3. Act on exactly one link, in priority order cancelAction, confirmationBase, deploymentBase. With none of these, the cycle is idle.

For a deployment the simulator SHALL:
1. Report `proceeding`.
2. Download every artifact of every chunk through `download-http`.
3. Verify sha1, and md5 when the server provides one.
4. Report `downloaded`, with a per-file detail.
5. Report `closed` with `success`, unless one of these holds:
   - `update` is `skip`, or the maintenance window is unavailable: the simulator SHALL NOT close the action, and it SHALL NOT re-download the action while waiting
   - `--defer-soft` is set and `update` is `attempt`: the simulator SHALL defer
   - `--fail-download` is set: the simulator SHALL report `closed` with `failure`

For cancellation it SHALL report `canceled`. For confirmation it SHALL report `confirmed`, or `denied` with `--deny-confirmation`.

#### Scenario: Healthy cycle
- **WHEN** a device with an assigned set runs one cycle
- **THEN** the server records the action FINISHED and the target IN_SYNC

#### Scenario: Injected failure
- **WHEN** the device runs with `--fail-download`
- **THEN** the action and the target end in ERROR

#### Scenario: Closed window does not spam
- **WHEN** a device has downloaded an action whose window is unavailable and cycles again
- **THEN** it sends no further feedback until the window opens

### Requirement: Polite pacing and backoff
After an idle, waiting or deferred cycle, the simulator SHALL sleep for `--poll-override` if given, else for the server's `polling.sleep`. After a cycle that did work, it SHALL continue after 50 ms.

On error it SHALL wait as follows:
- after a 503, the `Retry-After` delay-seconds, capped at 300 s
- otherwise, exponential backoff starting at 200 ms, capped at the poll interval, with jitter of ±25% derived from the controller id, so that devices which failed together return apart

Connect timeout SHALL be 10 s and each read SHALL time out after 30 s.

#### Scenario: Server asks to wait
- **WHEN** the server answers 503 with `Retry-After: 2` for 3 seconds
- **THEN** the device sends at most 3 requests in that time

#### Scenario: Silent server
- **WHEN** the server accepts the connection and never answers
- **THEN** the device fails the request within the read timeout and does not hang

### Requirement: Poll-only benchmark mode
With `--poll-only`, N devices SHALL only poll the controller base:
- every `--poll-override`, default 1 s
- with a deterministic staggered start
- printing a `POLLSTAT elapsed= total= errors= rps= avg_ms= max_ms=` line every second

#### Scenario: Poll statistics
- **WHEN** `--poll-only --count 100 --poll-override 100ms` runs for 5 seconds
- **THEN** about five POLLSTAT lines are printed with cumulative totals
