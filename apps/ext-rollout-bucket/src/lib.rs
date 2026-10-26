//! hawier rollout strategy: random canary waves over a stable bucket.
//!
//! Every device sits in a bucket in `0..modulo`, and a rollout takes a
//! pseudo-random window of buckets at a time: a small trial wave first, then
//! another window, and so on while each wave succeeds. A failure pauses the
//! rollout with only that wave touched.
//!
//! The bucket is derived from the controller id, not reported by the device.
//! That matters for three reasons: it needs no cooperation from firmware, it
//! is identical on every node without storing anything, and it does not move
//! when a device re-registers. It also sidesteps SCHEMA_NOTES #21 — an
//! attribute would be text, so `attribute.bucket=le=50000` would compare
//! lexicographically and quietly select the wrong devices.
//!
//! The windows are pseudo-random but **deterministic**: they come from the
//! rollout id, so re-planning the same rollout picks the same devices, and
//! two rollouts pick different ones. A canary you cannot reproduce is not a
//! canary, it is a coin toss.
//!
//! Parameters (`params` on the rule):
//!
//! - `modulo` (default 100000) — how finely the fleet is divided.
//! - `sliceSize` (default 1000) — bucket values per wave, so roughly
//!   `sliceSize/modulo` of the fleet each time.
//! - `waves` (default 5) — how many windows before the rest of the fleet.
//! - `finalWave` (default true) — end with a group that sweeps up everything
//!   the windows missed. Without it a rollout only ever reaches a sample.

use hawier_api::ext::{
    points, ExecutionSite, ExtManifest, ExtRequest, ExtResponse, GroupPlan, GroupSelector,
    PlannedGroup, RolloutPlanInput, RolloutStats, RolloutVerdict,
};

/// Strategy names this module answers to.
const KNOWN: [&str; 3] = ["bucket", "canary", "wave"];

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-rollout-bucket".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![
            points::ROLLOUT_PLAN.to_string(),
            points::ROLLOUT_EVALUATE.to_string(),
        ],
        sites: vec![ExecutionSite::Control],
        capabilities: Vec::new(),
        deadline_ms: 2_000,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    match request.point.as_str() {
        points::ROLLOUT_PLAN => match serde_json::from_value::<RolloutPlanInput>(request.payload) {
            Ok(input) if !KNOWN.contains(&input.strategy.as_str()) => ExtResponse::Err {
                // A name this module does not know would otherwise be planned
                // by whatever module happens to be loaded, which is how a
                // typo becomes a rollout divided by the wrong rules.
                message: format!(
                    "unknown strategy '{}'; this module implements {}",
                    input.strategy,
                    KNOWN.join(", ")
                ),
            },
            Ok(input) => encode(plan(&input)),
            Err(error) => ExtResponse::Err {
                message: format!("unusable rollout.plan payload: {error}"),
            },
        },
        points::ROLLOUT_EVALUATE => match serde_json::from_value::<RolloutStats>(request.payload) {
            Ok(stats) => encode(evaluate(&stats)),
            Err(error) => ExtResponse::Err {
                message: format!("unusable rollout.evaluate payload: {error}"),
            },
        },
        other => ExtResponse::Err {
            message: format!("hawier-ext-rollout-bucket does not implement {other}"),
        },
    }
}

fn encode<T: serde::Serialize>(value: T) -> ExtResponse {
    match serde_json::to_value(value) {
        Ok(payload) => ExtResponse::Ok { payload },
        Err(error) => ExtResponse::Err {
            message: format!("cannot encode the answer: {error}"),
        },
    }
}

fn number(params: &serde_json::Value, key: &str, default: i64) -> i64 {
    params.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/// A small, stable mixer. Not a hash for security — for picking a window
/// that is the same every time this rollout is planned and different from
/// the next rollout's.
fn mix(seed: u64, round: u64) -> u64 {
    let mut value = seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(round.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn plan(input: &RolloutPlanInput) -> GroupPlan {
    let modulo = number(&input.params, "modulo", 100_000).max(2);
    let slice = number(&input.params, "sliceSize", 1_000).clamp(1, modulo);
    let waves = number(&input.params, "waves", 5).clamp(1, 100);
    let final_wave = input
        .params
        .get("finalWave")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // The rollout id seeds the windows, so the same rollout always picks the
    // same devices and a different rollout picks different ones.
    let seed = input.rollout.parse::<u64>().unwrap_or_else(|_| {
        input
            .rollout
            .bytes()
            .fold(1469598103934665603u64, |acc, b| {
                (acc ^ b as u64).wrapping_mul(1099511628211)
            })
    });

    // Thresholds come from the declared groups where the operator gave any,
    // so a strategy change does not silently loosen a rollout's conditions.
    let default_success = input
        .groups
        .first()
        .map(|g| g.success_threshold)
        .unwrap_or(100.0);
    let default_error = input.groups.first().and_then(|g| g.error_threshold);

    let mut groups = Vec::new();
    for wave in 0..waves {
        // Windows never wrap: pick a start that leaves room for the slice, so
        // one wave is one contiguous range and the SQL stays a BETWEEN.
        let span = (modulo - slice).max(1) as u64;
        let from = (mix(seed, wave as u64) % span) as i64;
        groups.push(PlannedGroup {
            // Invented, not declared: a bucket rollout decides its own waves.
            id: None,
            name: format!("wave-{}", wave + 1),
            selector: GroupSelector::Bucket {
                from,
                to: from + slice,
                modulo,
            },
            success_threshold: default_success,
            error_threshold: default_error,
        });
    }
    if final_wave {
        groups.push(PlannedGroup {
            id: None,
            name: "rest".to_string(),
            selector: GroupSelector::Rest,
            success_threshold: default_success,
            error_threshold: default_error,
        });
    }
    GroupPlan { groups }
}

/// Same arithmetic as the classic strategy: a wave that meets its success
/// threshold is done, one that meets its error threshold stops everything.
/// The difference between the two strategies is how the fleet is divided,
/// not what counts as success.
fn evaluate(stats: &RolloutStats) -> RolloutVerdict {
    let mut verdict = RolloutVerdict::default();
    for group in &stats.groups {
        if group.total == 0 {
            verdict.finished.push(group.id.clone());
            continue;
        }
        if let Some(threshold) = group.error_threshold {
            let share = group.errors as f64 * 100.0 / group.total as f64;
            if group.errors > 0 && share >= threshold {
                verdict.errored.push(group.id.clone());
                verdict.pause = Some(format!("wave '{}' failed its error threshold", group.name));
                continue;
            }
        }
        let share = group.finished as f64 * 100.0 / group.total as f64;
        if share >= group.success_threshold {
            verdict.finished.push(group.id.clone());
        }
    }
    verdict
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;
    use hawier_api::ext::{GroupDefinition, GroupStats};

    fn input(rollout: &str, params: serde_json::Value) -> RolloutPlanInput {
        RolloutPlanInput {
            rollout: rollout.to_string(),
            name: "canary".to_string(),
            strategy: "bucket".to_string(),
            params,
            total_targets: 10_000,
            groups: vec![GroupDefinition {
                id: "1".to_string(),
                name: "declared".to_string(),
                target_percentage: None,
                target_filter: String::new(),
                success_threshold: 100.0,
                error_threshold: Some(10.0),
            }],
        }
    }

    fn windows(plan: &GroupPlan) -> Vec<(i64, i64)> {
        plan.groups
            .iter()
            .filter_map(|group| match group.selector {
                GroupSelector::Bucket { from, to, .. } => Some((from, to)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_same_rollout_always_picks_the_same_windows() {
        // A canary you cannot reproduce is a coin toss.
        let a = plan(&input("42", serde_json::json!({})));
        let b = plan(&input("42", serde_json::json!({})));
        assert_eq!(windows(&a), windows(&b));
    }

    #[test]
    fn a_different_rollout_picks_different_windows() {
        let a = plan(&input("42", serde_json::json!({})));
        let b = plan(&input("43", serde_json::json!({})));
        assert_ne!(windows(&a), windows(&b));
    }

    #[test]
    fn every_window_fits_inside_the_modulo() {
        let plan = plan(&input(
            "7",
            serde_json::json!({ "modulo": 1000, "sliceSize": 100, "waves": 20 }),
        ));
        for (from, to) in windows(&plan) {
            assert!(from >= 0, "window starts inside the range");
            assert!(to <= 1000, "and ends inside it: {from}..{to}");
            assert_eq!(to - from, 100, "each wave is one slice wide");
        }
    }

    #[test]
    fn a_final_sweep_catches_what_the_windows_missed() {
        let with_sweep = plan(&input("7", serde_json::json!({ "waves": 2 })));
        assert_eq!(with_sweep.groups.len(), 3);
        assert!(matches!(
            with_sweep.groups.last().unwrap().selector,
            GroupSelector::Rest
        ));

        // Without it a rollout only ever reaches a sample of the fleet.
        let sample = plan(&input(
            "7",
            serde_json::json!({ "waves": 2, "finalWave": false }),
        ));
        assert_eq!(sample.groups.len(), 2);
        assert!(sample
            .groups
            .iter()
            .all(|g| matches!(g.selector, GroupSelector::Bucket { .. })));
    }

    #[test]
    fn thresholds_come_from_the_declared_group() {
        let plan = plan(&input("7", serde_json::json!({ "waves": 1 })));
        assert_eq!(plan.groups[0].success_threshold, 100.0);
        assert_eq!(plan.groups[0].error_threshold, Some(10.0));
    }

    #[test]
    fn a_failing_wave_pauses_and_names_itself() {
        let verdict = evaluate(&RolloutStats {
            rollout: "1".to_string(),
            strategy: "bucket".to_string(),
            params: serde_json::json!({}),
            dynamic: false,
            groups: vec![GroupStats {
                id: "9".to_string(),
                name: "wave-1".to_string(),
                total: 100,
                finished: 0,
                errors: 20,
                success_threshold: 100.0,
                error_threshold: Some(10.0),
            }],
        });
        assert_eq!(verdict.errored, vec!["9".to_string()]);
        assert!(verdict.pause.unwrap().contains("wave-1"));
    }
}
