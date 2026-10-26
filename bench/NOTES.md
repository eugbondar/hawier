# How to read the measurement history

Two things made the past history unreadable. Both are now accounted for, but
they still need to be remembered.

## 1. Only identical parameters are comparable

`load-history.csv` — rows are comparable with identical `count` **and** `size_mb`.
`poll-history.csv` — with identical `count`, `interval` **and** `duration_s`.

Different parameters measure different things. `count=2000, interval=1s` is a
ceiling of 2000 rps by construction: a saturation test, not a throughput test,
and comparing it with a run at `100ms` is meaningless.

## 2. `avg_ms` / `max_ms` in the poll history are cumulative

The client prints `POLLSTAT` once a second, and the latencies in it are computed **from
the start of the run**, not over the last second. The script takes the last line,
so the CSV gets the average over the whole run and the maximum over the whole run.
Runs of different duration cannot be compared by these two columns.

## 3. The database grew between measurements (fixed)

Until 2026-08-25 `scripts/load.sh` did not clean up after itself: every run left
`COUNT` targets, a DS, a module, an artifact and a rollout behind forever. By the time of
the investigation, 710 abandoned targets from past runs had piled up in dev (1971 in total,
4976 `sp_action_status` rows).

This, not a code regression, explains the main anomaly of the history:

| date | commit | count/size | updates/s |
|---|---|---|---|
| 2026-08-21 | 9633296 | 200/4 | **49.2** |
| 2026-08-22 | b7041ce | 200/4 | **5.2** |
| 2026-08-22 | b7041ce | 200/4 | **11.6** |
| 2026-08-22 | b7041ce | 1000/8 | 123.2 (against 120.7 on 9633296 — parity) |

A 4–9x drop at `200/4` with parity at `1000/8` on the same day is the
signature of a noisy measurement and a growing database, not of code degradation.

Cleanup now hangs on `trap cleanup EXIT INT TERM` and runs on any
outcome: success, `FAIL`, Ctrl-C, `timeout`. `KEEP=1` disables it for post-mortems.
The only case where it will **not** fire is `kill -9`:
SIGKILL cannot be caught, and after it you have to clean up by hand.

The first version of this fix did not work in three cases out of four, and all three
were silent: cleanup was the last line (a failure and a timeout never
reached it); `trap ... EXIT` does not fire when the process dies from a signal (`INT TERM`
are needed); further down the script there were two more `trap ... EXIT`, and traps in bash
do not stack — the last one overwrote the previous ones. Plus `set +e` was not the first
line of the cleanup, and `kill` of an already finished fleet killed the function under `set -e`.
Verified by hand: a successful run and an interruption by signal must both print
`left behind: 0`.

Rows before 2026-08-25 stay in the history as they are, but new measurements
**cannot** be compared with them — the conditions were different.

## 4. The `200/4` configuration does not measure throughput

The rollout engine ticks once every 2 seconds (`rollout.rs`, `TICK`). The install
phase on 200 devices takes the same ~2 seconds, so the result
is determined entirely by where the run landed relative to the tick:

| run | seconds | updates/s |
|---|---|---|
| lucky | 2.0 | ~98 |
| one extra tick | 4.1 | ~49 |
| two extra ticks | 14–17 | ~12–14 |

This is neither noise nor a regression — it is the discreteness of the tick. The historical "regression"
49.2 → 5.2 at `200/4` is of exactly the same nature plus the growing database.
**Conclusion: configurations whose install phase is comparable to 2 seconds are
unfit for comparison.** Measure on a load where the install lasts
at least tens of seconds.

## 5. The test stand measures itself

`load.sh` polls `totalTargetsPerStatus` through the mgmt API while the fleet polls
DDI on the same node. At 1000 devices with a 500 ms interval the node is eaten
by polling: a plain `curl` to mgmt does not respond for minutes, and the script measures not
the update speed but the starvation of its own polling.

The observation is nevertheless real and concerns the product: before
downloading, a device sends `Proceeding`, and if the download does not fit into the cycle, it
sends it again on the next cycle — each attempt is a row in
`sp_action_status`. This makes an amplification loop: saturation → download does not
complete → repeated report → another write → more saturation. One such
run accumulated **2.7 million rows**, every action hit the quota of 800.
On an unloaded node the same path yields **6 rows per action** (verified:
50 devices, `avg 6.0, max 6`), i.e. this is precisely a saturation effect.

**Fixed (`ad72117`):** a repeated status that carries no new information
does not write a row — it is counted on the existing one (`repeats` in the API, `×N` on
the action page). Details in API_NOTES §5.

**Roles separated (`5a32f60`):** the fleet is directed to a sub node via
`DDI_URL`, progress is polled from the control node. Otherwise measurement and load
fight over one queue: `curl` to mgmt does not respond for minutes, and the script measures
the starvation of its own polling, not the update speed.

    DDI_URL=http://host:8081 URL=http://host:8080 COUNT=1000 SIZE_MB=8 make load

## 6. What the separated test stand showed (2026-08-25)

Separating the roles made `1000/8` observable — and the first thing it showed
turned out not to be what I had been thinking about for two hours straight.

### The real cause: the node ran out of file descriptors

The sub node was answering **500 "No file descriptors available (os error 24)"** —
154 thousand times per run. The container ran with docker's default soft limit
of **1024** descriptors against a hard limit of 524288 (`ulimits` in compose
were simply not set), and every artifact download costs a socket plus an open
file. After that the node returned 500 **for everything, polling included**, and a
device reads 500 as a failed cycle and returns immediately. That is, the node's
own exhaustion was the source of that very avalanche of repeats.
There is no leak: with no load the node holds 40 descriptors.

With `nofile: 65536` the same run gives **1000/1000 in 6.1 s** (164 updates/s,
~10.5 Gbit/s of payload) and **not a single** warning from the fleet.

**The moral for all measurements before `0c56462`:** they were made on an
artificially broken node. The numbers "973/1000" and "983/1000" are not the difference
between client versions, they are noise within one and the same failure. They
cannot be compared with each other.

### Three fixes made along the way

They are right in themselves, but they were not what held the tail:

1. **Recording of repeats** (`ad72117`) — a repeated status does not write a row.
   The only measurement that descriptor exhaustion does not disturb:
   rows per 1000 actions were 2.7 million (every action hit the quota of
   800), now 6887 — 6.9 per action. During an avalanche of repeats this is the difference
   between a working database and a dead one.
2. **Retry without a pause** (`189078c`) — after a cycle error the device slept
   50 ms. Exponential backoff with jitter: 92k repeated reports per
   run → 10.8k. The backoff ceiling is the polling interval, and the test stand starts
   devices with `--poll-override 500ms`, so in the benchmark it cannot
   grow above 500 ms by design.
3. **Hung connection** (`04b229b`) — the client had no timeouts at all,
   and a device that caught a silent connection never polled the
   server again. The hypothesis "this is the tail" **was not confirmed**:
   with timeouts the run gave 970/1000 against 983/1000 without them, i.e.
   within the noise of the same failure.

### Checking 503 on a deliberately broken node (2026-08-27)

The sub node's limit was temporarily returned to 1024 to reproduce the same failure
and look at the new behavior (`befeac2`):

| | 500, client does not listen | 503 + Retry-After, client listens |
|---|---|---|
| node responses | 154k × `500` | **82 406 × `503`, `500` — zero** |
| fleet errors | none carries a hint | **23 284, every single one with a delay** |
| finished within 600 s | 969–982 of 1000 | **790 of 1000** |

The mechanism works end to end: the node correctly classifies the
exhaustion, the device sees the delay and honors it. But **politeness costs
completions**: on a node that genuinely lacks a resource, a fleet
waiting 3–7 seconds finishes fewer devices in the same window than a fleet
that hammers. This is an honest tradeoff, not a win: 503 protects the node
(and everything else it serves), it does not speed up the update. A resource
shortage is cured by a resource — with `nofile 65536` the same configuration gives
1000/1000 in 6.1 s.

**Open tuning question:** 3–7 s were chosen at random, while `EMFILE`
clears in milliseconds. A delay of about a second would most likely have brought back
most of those 180 devices while staying two orders of magnitude gentler than
hammering. Verified by the same run with a different value in
`error::retry_after_seconds`.

### What remains

503 with `Retry-After` instead of 500 is done (API_NOTES §5): descriptor
exhaustion and a DB pool timeout are now "busy", not "broken", and the device
honors the header. But the node answers this way having already hit the resource: there is no
limiter of concurrent downloads, and it still cannot ask the whole fleet
to poll less often — `polling.sleep` remains a tenant constant.

### Baseline (the first honest one in the whole history of the file)

`2026-08-25, 0c56462, 1000/8` — the median of three runs: **6.1 s, 164.7
updates/s, 1317 MB/s (~10.5 Gbit/s)**. The spread between runs is 164.5 /
164.7 / 164.8, i.e. 0.2%; status rows exactly 7.0 per action, repeats
zero, fleet complaints zero. Conditions: control+sub on one 32-core machine,
`nofile 65536`, the fleet in a single process on the same host, load average at start
11–14 (created by the fleet itself), no foreign load.

A caveat that makes this a baseline rather than "throughput":
6.1 s is **three ticks** of the rollout engine (§4). The resolution is still
coarse, there are just three divisions now instead of one. And 10.5 Gbit/s goes through
loopback, so the upper bound here is not the network. A real throughput
measurement needs a fleet on a separate machine and an install phase of
tens of seconds.

### The fleet could run on yesterday's binary

`make load` built `hawier-device`, while `bash scripts/load.sh` did not, and picked up
whatever lay in `target/release`. The script is called directly when working interactively,
so a run that was checking a fresh client change calmly measured the client
from an hour ago and showed that the change "changed nothing". The build
moved inside the script: both paths yield the same binary.

Hence one more correction to the evening of 2026-08-25: client-side comparisons
(backoff, timeouts) were made with runs for which it is **not proven** that the fleet
ran the built code. The numbers "92k → 10.8k reports" and "983 against 970"
cannot be attributed to those fixes. The only thing measured reliably is the
server side (status rows) and the baseline after `0c56462`.

### How to catch this next time

Both traces were thrown away, and it cost hours:

- `DEVICE_LOG=/tmp/fleet.log` — the fleet's output used to go to `/dev/null`,
  and a stuck device left nothing at all.
- `docker logs deploy-hawier-sub-1 | grep -i error` — the server had been
  writing the cause in plain text all along; I just did not look there.
- compare the fleet's build time with the time of the change: `ls -l target/release/hawier-device`
  against `git log -1 --format=%cd -- apps/client`. The script now does this itself,
  but the habit is useful for any test stand with its own binary.

## How to measure honestly

1. Do not build or run tests in parallel: `cargo build` on 32
   cores eats the machine and spoils the measurement. **The machine is shared** — other
   projects run on it too. Before starting, check not only the load average but also
   network load: the run of 2026-08-25 coincided with someone else's `iperf3 -u` through
   a wireguard proxy and gave `1/1000` instead of `983/1000` — while
   fleet registration took 17 s instead of the usual 2–4 s, i.e. the machine was
   already busy before the start. This is a disqualification of the measurement, not a result:

       pgrep -af "iperf3|loadgen|wireproxy"  # empty — OK to measure
2. Record the conditions: `uptime` (load average), fleet size before the run,
   what else is running on the machine.
3. Three runs per configuration, the median goes into the CSV, the spread goes here.
4. The regression threshold is 20% at the **same** parameters (README, CLAUDE.md).

## Measurements

| date | commit | configuration | result | conditions |
|---|---|---|---|---|
| 2026-08-25 | c24388e | 200/4 ×3 | 14.2 / 11.6 / 97.6 upd/s | **not recorded in the CSV**: the spread is tick discreteness (see §4), not measurement |
| 2026-08-25 | c24388e | 1000/8 ×3 | did not finish within 15 min | node eaten by polling (see §5); baseline not established |
| 2026-08-25 | c24388e | 50/1 | 6 status rows per action | control: on an unloaded node there is no spam |

**The baseline on this machine is not established, and nothing is recorded in `load-history.csv`
for 2026-08-25, deliberately.** Recording 14.2 would plant a "sevenfold regression"
for the next agent where there is none; picking the convenient 97.6 would be the
same thing the other way round.
