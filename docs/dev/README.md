# hawier — developer and operator reference

How to build, run, measure and deploy hawier. What the system guarantees is
in the specs, [`openspec/specs/`](../../openspec/specs/); these pages say how
to do things and link to the spec for the rest. How the console is used is
in its built-in manual (`/help` in the console, sources in
[`apps/ui/docs/`](../../apps/ui/docs/)).

| Page | What it covers |
|---|---|
| [make.md](make.md) | every make target, the development loop, image builds |
| [configuration.md](configuration.md) | server and console environment variables, extension parameters |
| [simulator.md](simulator.md) | `hawier-device`: one device, a fleet, fault injection |
| [load-testing.md](load-testing.md) | `make load` / `make load-poll` and the regression rule |
| [e2e.md](e2e.md) | `make e2e`: hawier against SWUpdate/suricatta in QEMU |
| [deployment.md](deployment.md) | `make deploy-<host>`, TLS, the pre-production checklist, scaling |
| [extensions.md](extensions.md) | the wasm modules and how to run a node with them |
| [hawkbit-compatibility.md](hawkbit-compatibility.md) | the shared database schema and the Flyway history |

## Repository layout

| What | Where | Binary | Port |
|---|---|---|---|
| Server (Mgmt `/rest/v1`, DDI `/{tenant}/controller/v1`, `/ui/v1`) | `apps/server` | `hawier` | 8080 |
| Web console (server-side rendered) | `apps/ui` | `hawier-ui` | 8088 |
| Device simulator / load client | `apps/client` | `hawier-device` | — |
| First-party wasm extensions | `apps/ext-*` | `hawier_ext_*.wasm` | — |
| Shared DTOs and enum codes | `libs/api` | — | — |

The module map, the topology and the trade-offs are in
[ARCHITECTURE.md](../../ARCHITECTURE.md). The backlog lives in
[API_NOTES.md](../../API_NOTES.md), [SCHEMA_NOTES.md](../../SCHEMA_NOTES.md)
and the roadmap parts of [EXTENSIONS.md](../../EXTENSIONS.md); how to read
the load-run history is [bench/NOTES.md](../../bench/NOTES.md).
