# Device simulator

`hawier-device` (crate `apps/client`) speaks DDI the way a device does: it
polls, downloads, verifies the hashes and sends feedback. It is the load
client of `make load` and the fleet of `make screenshots`. Through make:
`make device ARGS="..."`. Behaviour: spec
[`device-simulator`](../../openspec/specs/device-simulator/spec.md).

```bash
# one device, one cycle (poll → download → verify sha1/md5 → feedback)
hawier-device --url http://localhost:8080 --controller-id dev-1 --token <sec_token> --once

# a fleet of 500 devices via gateway token
hawier-device --count 500 --controller-id fleet --gateway-token <key> --poll-override 5s

# fault injection
hawier-device --fail-download ...      # closes updates with failure
hawier-device --deny-confirmation ...  # rejects the confirmation flow
```

| Flag | Meaning |
|---|---|
| `--url` | DDI base URL (default `http://localhost:8080`, env `HAWIER_DEVICE_URL`) |
| `--tenant` | tenant (default `DEFAULT`) |
| `--controller-id` | the device id; with `--count`, the prefix of `{id}-{n}` |
| `--token` / `--gateway-token` | target token or the tenant's gateway token |
| `--poll-override` | ignore the server's polling time, e.g. `5s`, `500ms` |
| `--count` | fleet mode: run N devices |
| `--download-dir` | keep downloads there instead of verify-and-discard |
| `--once` | one poll cycle, then exit |
| `--fail-download` | close every update with failure |
| `--deny-confirmation` | deny confirmation requests |
| `--defer-soft` | download soft updates but wait until the server forces them |
| `--attributes` | controller attributes to report, `k=v,k=v` |
| `--poll-only` | only poll at `--poll-override` and print `POLLSTAT` lines (used by `make load-poll`) |

The simulator builds only the initial poll URL itself; everything after it
comes from the `_links` in server responses. The doctrine and the
geo-polling roadmap are in [API_NOTES.md](../../API_NOTES.md).
