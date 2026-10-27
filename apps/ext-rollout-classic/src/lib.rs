//! hawier rollout strategy: hawkBit's classic percentage rollout.
//!
//! This is the behaviour hawier had built in, moved out unchanged so that
//! "how a rollout is divided" is a decision like any other. A hardened
//! deployment simply does not load this module, and then the only rollout
//! logic in the system is the one its operators chose (EXTENSIONS.md §5).
//!
//! What it does, and does not do:
//!
//! - each group takes a percentage of what earlier groups **left**, the last
//!   one taking the remainder — hawkBit's own arithmetic;
//! - a group meets its success condition when the share of its targets that
//!   installed successfully reaches its success threshold. That starts the
//!   next group and nothing else: the group is finished when none of its
//!   actions is still active, which the engine sees for itself, as hawkBit
//!   does. A rollout with a device that never polls stays running;
//! - a group has failed when the share that errored reaches its error
//!   threshold, and that pauses the rollout;
//! - the per-group `targetFilterQuery` is **ignored**, exactly as before.
//!   That is a real gap (API_NOTES §7) and it is left here rather than
//!   quietly fixed, because "classic" means the semantics people already
//!   have. A strategy that honours it is a different module.

use hawier_api::ext::{
    points, ExecutionSite, ExtManifest, ExtRequest, ExtResponse, GroupPlan, GroupSelector,
    PlannedGroup, RolloutPlanInput, RolloutStats, RolloutVerdict,
};

/// Strategy names this module answers to.
const KNOWN: [&str; 4] = ["", "default", "classic", "percentage"];

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-rollout-classic".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![
            points::ROLLOUT_PLAN.to_string(),
            points::ROLLOUT_EVALUATE.to_string(),
        ],
        // The rollout engine is the control node's.
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
            message: format!("hawier-ext-rollout-classic does not implement {other}"),
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

/// Percentage of the remaining targets, last group takes the rest.
fn plan(input: &RolloutPlanInput) -> GroupPlan {
    let count = input.groups.len();
    let groups = input
        .groups
        .iter()
        .enumerate()
        .map(|(index, definition)| PlannedGroup {
            id: Some(definition.id.clone()),
            name: definition.name.clone(),
            selector: if index + 1 == count {
                // The last group takes whatever rounding left behind, so no
                // target is silently dropped from the rollout.
                GroupSelector::Rest
            } else {
                GroupSelector::Share {
                    percentage: definition.target_percentage.unwrap_or(100.0),
                }
            },
            success_threshold: definition.success_threshold,
            error_threshold: definition.error_threshold,
        })
        .collect();
    GroupPlan { groups }
}

/// Thresholds as percentages of the group, which is what
/// `successCondition: THRESHOLD` has always meant here.
fn evaluate(stats: &RolloutStats) -> RolloutVerdict {
    let mut verdict = RolloutVerdict::default();
    for group in &stats.groups {
        if group.total == 0 {
            // An empty group has succeeded; otherwise a rollout with one
            // would never advance past it.
            verdict.succeeded.push(group.id.clone());
            continue;
        }
        if let Some(threshold) = group.error_threshold {
            let share = group.errors as f64 * 100.0 / group.total as f64;
            if group.errors > 0 && share >= threshold {
                verdict.errored.push(group.id.clone());
                verdict.pause = Some("error threshold".to_string());
                continue;
            }
        }
        let share = group.finished as f64 * 100.0 / group.total as f64;
        if share >= group.success_threshold {
            verdict.succeeded.push(group.id.clone());
        }
    }
    verdict
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;
    use hawier_api::ext::{GroupDefinition, GroupStats};

    fn definition(id: &str, percentage: Option<f64>) -> GroupDefinition {
        GroupDefinition {
            id: id.to_string(),
            name: format!("group-{id}"),
            target_percentage: percentage,
            target_filter: String::new(),
            success_threshold: 100.0,
            error_threshold: Some(50.0),
        }
    }

    fn input(groups: Vec<GroupDefinition>) -> RolloutPlanInput {
        RolloutPlanInput {
            rollout: "1".to_string(),
            name: "fleet".to_string(),
            strategy: "classic".to_string(),
            params: serde_json::json!({}),
            total_targets: 100,
            groups,
        }
    }

    fn stats(groups: Vec<GroupStats>) -> RolloutStats {
        RolloutStats {
            rollout: "1".to_string(),
            strategy: "classic".to_string(),
            params: serde_json::json!({}),
            dynamic: false,
            groups,
        }
    }

    fn group(id: &str, total: i64, finished: i64, errors: i64) -> GroupStats {
        GroupStats {
            id: id.to_string(),
            name: format!("group-{id}"),
            total,
            finished,
            errors,
            success_threshold: 100.0,
            error_threshold: Some(50.0),
        }
    }

    #[test]
    fn every_group_but_the_last_takes_its_share() {
        let plan = plan(&input(vec![
            definition("1", Some(50.0)),
            definition("2", Some(50.0)),
        ]));
        assert!(matches!(
            plan.groups[0].selector,
            GroupSelector::Share { percentage } if percentage == 50.0
        ));
        // The last one takes the remainder, so rounding never drops a target.
        assert!(matches!(plan.groups[1].selector, GroupSelector::Rest));
    }

    #[test]
    fn a_single_group_takes_everything() {
        let plan = plan(&input(vec![definition("1", Some(10.0))]));
        assert_eq!(plan.groups.len(), 1);
        assert!(matches!(plan.groups[0].selector, GroupSelector::Rest));
    }

    #[test]
    fn a_group_succeeds_at_its_success_threshold() {
        let verdict = evaluate(&stats(vec![group("1", 10, 10, 0)]));
        assert_eq!(verdict.succeeded, vec!["1".to_string()]);
        assert!(verdict.pause.is_none());
    }

    #[test]
    fn short_of_the_threshold_it_keeps_running() {
        let verdict = evaluate(&stats(vec![group("1", 10, 9, 0)]));
        assert!(verdict.succeeded.is_empty());
        assert!(verdict.errored.is_empty());
    }

    #[test]
    fn the_error_threshold_pauses_the_rollout() {
        let verdict = evaluate(&stats(vec![group("1", 10, 0, 5)]));
        assert_eq!(verdict.errored, vec!["1".to_string()]);
        assert_eq!(verdict.pause.as_deref(), Some("error threshold"));
    }

    #[test]
    fn a_single_error_below_the_threshold_does_not_pause() {
        let verdict = evaluate(&stats(vec![group("1", 10, 0, 1)]));
        assert!(verdict.errored.is_empty());
        assert!(verdict.pause.is_none());
    }

    #[test]
    fn an_empty_group_succeeds_rather_than_stuck() {
        let verdict = evaluate(&stats(vec![group("1", 0, 0, 0)]));
        assert_eq!(verdict.succeeded, vec!["1".to_string()]);
    }

    #[test]
    fn a_threshold_met_with_stragglers_is_success_and_nothing_else() {
        // Half done, half still running: enough to start the next group.
        // Whether this one is finished is not the module's to say.
        let mut half = group("1", 10, 5, 0);
        half.success_threshold = 50.0;
        let verdict = evaluate(&stats(vec![half]));
        assert_eq!(verdict.succeeded, vec!["1".to_string()]);
        assert!(verdict.errored.is_empty());
        assert!(verdict.pause.is_none());
    }
}
