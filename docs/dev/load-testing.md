# Load runs

Behaviour of both targets: spec
[`load-benchmark`](../../openspec/specs/load-benchmark/spec.md). How to read
the history and what in it not to trust: [bench/NOTES.md](../../bench/NOTES.md).

## `make load` — updates

A real end-to-end run against a running hawier (`make up` or `make run`;
for a measurement that counts, `CARGO_PROFILE=release make scale`): it
creates an artifact and a rollout to a fleet of simulators, waits for
FINISHED on all of them and prints updates/s and Gbit/s.

```bash
COUNT=1000 SIZE_MB=8 make load
```

| Variable | Default | Meaning |
|---|---|---|
| `URL` | `http://localhost:8080` | the node Mgmt goes through |
| `DDI_URL` | `= URL` | where the fleet connects (e.g. a sub node on `:8081`) |
| `COUNT` | `200` (at least 2) | fleet size |
| `SIZE_MB` | `4` | artifact size |
| `TIMEOUT_S` | `600` | how long to wait for FINISHED on all |
| `USER_PASS` | `admin:admin` | Mgmt account |
| `GATEWAY_KEY` | `load-gateway-key` | gateway key the fleet registers with |
| `DEVICE_BIN` | `target/release/hawier-device` | the simulator (built automatically by default) |
| `DEVICE_LOG` | `/dev/null` | where to write the fleet log |
| `HISTORY` | `bench/load-history.csv` | where to append the row |
| `PROFILE_CHECK` | `1` | `0` — don't check that the nodes are release (the run then does not count for history) |
| `KEEP` | `0` | `1` — don't delete what was created on the server |

Each run appends a row to `bench/load-history.csv` (the history lives in
the repository) and removes the fleet it created.

## `make load-poll` — polling

Takes `URL`, `DDI_URL`, `COUNT` (2000), `INTERVAL` (`1s`), `DURATION_S`
(60), `USER_PASS`, `GATEWAY_KEY`, `HISTORY` (`bench/poll-history.csv`) and
`KEEP`.

```bash
COUNT=5000 INTERVAL=500ms make load-poll
```

## Regression rule

Run `make load` before a release and after changes to the hot path (DDI,
artifact delivery, the rollout engine) and compare with the history. A drop
in updates/s or MB/s of more than 20% at the same parameters is a
regression, to be dealt with before the change lands. Single-node
measurements are in [ARCHITECTURE.md](../../ARCHITECTURE.md).
