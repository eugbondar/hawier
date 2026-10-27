# Tasks

## 1. Verdict contract

- [x] 1.1 Rename `RolloutVerdict.finished` to `succeeded` in `libs/api/src/ext.rs` with `#[serde(alias = "finished")]`, and say in its doc comment that it starts the next group and does not finish anything; verify `cargo build` of the workspace
- [x] 1.2 Update `apps/ext-rollout-classic` and `apps/ext-rollout-bucket` to the new field and the module doc comments to the new meaning; add a classic unit test for "threshold met with stragglers" and verify `make test`

## 2. Engine

- [x] 2.1 In `apps/server/src/rollout.rs`, apply succeeded groups by starting the SCHEDULED successor in id order (or growing a dynamic rollout when there is none) instead of setting them FINISHED; verify by the integration tests below
- [x] 2.2 Mark RUNNING groups FINISHED when none of their actions is in a non-terminal status (DOWNLOADED terminal only for download-only rollouts), inside the claimed transaction, before the existing next-group/finish logic; verify `rollout_full_lifecycle_two_groups` still passes

## 3. Tests

- [x] 3.1 Add an integration test reproducing the report: 42 targets, 3 groups, THRESHOLD 50, 6 devices never poll → the rollout and the groups holding them stay RUNNING, `totalTargetsPerStatus` 36 finished / 6 running; after those 6 update, everything is FINISHED; verify with `cargo test -p hawier-server --test it -- --ignored rollout::`
- [x] 3.2 Add integration tests for a group settling below its success threshold (FINISHED, rollout moves on) and for a dynamic rollout growing while its last group has a straggler; verify as in 3.1

## 4. Manual

- [x] 4.1 Update `apps/ui/docs/deployment/rollout-groups.md` (success threshold, choosing the numbers, the `finished` state) and `apps/ui/docs/glossary.md` (finished) to the new meaning; verify `make ui-audit`

## 5. Verification and commit

- [x] 5.1 `make lint`, `make test`, `make it` green
- [x] 5.2 `make load` against a release node, compared with the last rows of `bench/load-history.csv` at the same parameters (no degradation above 20%)
- [x] 5.3 Sync the delta specs into `openspec/specs/`, archive the change, and commit the slice on master
