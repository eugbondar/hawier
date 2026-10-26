# Spec Delta

## Purpose

The end-to-end load measurements (`make load`, `make load-poll`) against a
running hawier. Each run produces a result row in the benchmark history. The
tool refuses measurements that would be misleading and cleans up after itself.

## ADDED Requirements

### Requirement: End-to-end update benchmark
`make load` SHALL measure update throughput against a running node at `URL`, with defaults:

| Parameter | Default |
|---|---|
| `URL` | `:8080` |
| `COUNT` | 200, at least 2 |
| `SIZE_MB` | 4 |
| `TIMEOUT_S` | 600 |

The run SHALL:
1. Create a module with one artifact, a distribution set, and a rollout over a fleet of simulated devices.
2. Wait until every device has finished.
3. Print updates per second and throughput.

`DDI_URL` SHALL point the fleet at a different node than the one used for management.

#### Scenario: Successful run
- **WHEN** `COUNT=1000 SIZE_MB=8 make load` completes against a release node
- **THEN** it prints updates/s and Gbit/s and appends one row to the history

### Requirement: Refuse to measure a debug build
Before measuring, the benchmark SHALL ask each node it uses for `node.profile`. It SHALL refuse with a failure message and record nothing if any node reports `debug`. `PROFILE_CHECK=0` SHALL skip the check.

#### Scenario: Debug control node
- **WHEN** `make load` runs against a dev-profile node
- **THEN** it fails, suggests `CARGO_PROFILE=release make scale`, and writes no history row

### Requirement: History rows
A successful update run SHALL append `date,git_rev,count,size_mb,seconds,updates_per_s,mb_per_s` to `bench/load-history.csv`. A poll run SHALL append `date,git_rev,count,interval,duration_s,total,errors,avg_rps,avg_ms,max_ms` to `bench/poll-history.csv`. A missing file SHALL be created with its header.

#### Scenario: New history file
- **WHEN** the history file does not exist and a run succeeds
- **THEN** the file is created with a header and one data row

### Requirement: Cleanup on every exit
On normal exit, error, interrupt or termination, the benchmark SHALL stop its fleet and delete what it created on the server: the rollout, the `$PREFIX-*` targets, the distribution set and the module. It SHALL then report how many targets were left behind. `KEEP=1` SHALL keep the server-side objects.

#### Scenario: Interrupted run
- **WHEN** a run is interrupted with Ctrl-C mid-rollout
- **THEN** cleanup runs and reports `left behind: 0`

### Requirement: Poll benchmark
`make load-poll` SHALL register `COUNT` devices and run the simulator in poll-only mode for `DURATION_S`. It SHALL record the final poll statistics.

#### Scenario: Poll run
- **WHEN** `make load-poll` completes
- **THEN** one row with the total polls, errors, average rate and latency is appended to the poll history
