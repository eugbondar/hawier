//! hawier retention extension: which artifacts a rule may delete.
//!
//! The host narrows the field (RSQL plus a minimum age) and hands over the
//! candidates; this decides. It answers with ids and nothing else — the host
//! checks every id came from the list it offered, verifies no other row still
//! references the same content, and does the deleting.
//!
//! Runs as wasm with no file system and no network: it cannot touch an
//! artifact even if it wanted to. Deciding is all it does.
//!
//! The default policy is "keep releases, keep anything in use, keep the
//! newest few dev builds per module". Knobs come from the rule's `params`:
//!
//! - `keepPerModule` (default 3) — newest dev builds to keep per module.
//! - `releasePattern` (default `-dev`,`-snapshot`,`+build`) — a version
//!   containing any of these is a dev build; everything else is a release.
//! - `deleteAssigned` (default false) — allow deleting something a device
//!   was once told to install.

use std::collections::HashMap;

use hawier_api::ext::{
    points, ExecutionSite, ExtManifest, ExtRequest, ExtResponse, RetentionCandidate, RetentionPlan,
    RetentionRequest,
};

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-retention".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::RETENTION_SELECT.to_string()],
        // Rules fire from the control node's schedule.
        sites: vec![ExecutionSite::Control],
        // None: it returns a list of ids.
        capabilities: Vec::new(),
        deadline_ms: 2_000,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::RETENTION_SELECT {
        return ExtResponse::Err {
            message: format!("hawier-ext-retention does not implement {}", request.point),
        };
    }
    let plan = match serde_json::from_value::<RetentionRequest>(request.payload) {
        Ok(plan) => plan,
        Err(error) => {
            return ExtResponse::Err {
                message: format!("unusable retention.select payload: {error}"),
            }
        }
    };
    if plan.scope != "artifact" {
        return ExtResponse::Err {
            message: format!(
                "this extension only understands the artifact scope, not '{}'",
                plan.scope
            ),
        };
    }
    match serde_json::to_value(select(&plan)) {
        Ok(payload) => ExtResponse::Ok { payload },
        Err(error) => ExtResponse::Err {
            message: format!("cannot encode the plan: {error}"),
        },
    }
}

const DEFAULT_MARKERS: [&str; 3] = ["-dev", "-snapshot", "+build"];

fn markers(params: &serde_json::Value) -> Vec<String> {
    params
        .get("releasePattern")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|v| v.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|list| !list.is_empty())
        .unwrap_or_else(|| DEFAULT_MARKERS.iter().map(|m| m.to_string()).collect())
}

/// A version carrying a pre-release marker is a developer build; anything
/// else is treated as a release and never deleted by this policy.
fn is_dev_build(version: &str, markers: &[String]) -> bool {
    let version = version.to_ascii_lowercase();
    markers.iter().any(|marker| version.contains(marker))
}

fn select(request: &RetentionRequest) -> RetentionPlan {
    let markers = markers(&request.params);
    let keep_per_module = request
        .params
        .get("keepPerModule")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as usize;
    let delete_assigned = request
        .params
        .get("deleteAssigned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Group what is eligible by module, so "keep the newest few" means the
    // newest few *of that module* rather than of the whole fleet's history.
    let mut by_module: HashMap<&str, Vec<&RetentionCandidate>> = HashMap::new();
    let mut skipped_release = 0usize;
    let mut skipped_in_use = 0usize;

    for candidate in &request.candidates {
        if !is_dev_build(&candidate.module_version, &markers) {
            skipped_release += 1;
            continue;
        }
        // Something a distribution set still offers is not spare.
        if candidate.in_distribution_set || (candidate.ever_assigned && !delete_assigned) {
            skipped_in_use += 1;
            continue;
        }
        by_module
            .entry(candidate.software_module.as_str())
            .or_default()
            .push(candidate);
    }

    let mut delete = Vec::new();
    for (_, mut group) in by_module {
        // Newest first, so the tail is what ages out.
        group.sort_by_key(|candidate| std::cmp::Reverse(candidate.created_at));
        for candidate in group.into_iter().skip(keep_per_module) {
            delete.push(candidate.id.clone());
        }
    }
    // Stable output: the same input must produce the same plan, or a dry run
    // tells you nothing about the real one.
    delete.sort();

    RetentionPlan {
        reason: format!(
            "{} to delete; kept {} release artifacts, {} still in use, newest {} per module",
            delete.len(),
            skipped_release,
            skipped_in_use,
            keep_per_module
        ),
        delete,
    }
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, module: &str, version: &str, created_at: i64) -> RetentionCandidate {
        RetentionCandidate {
            id: id.to_string(),
            sha1: format!("{id:0>40}"),
            filename: format!("{id}.bin"),
            size: 1024,
            created_at,
            software_module: module.to_string(),
            module_name: "fw".to_string(),
            module_version: version.to_string(),
            in_distribution_set: false,
            ever_assigned: false,
        }
    }

    fn request(candidates: Vec<RetentionCandidate>, params: serde_json::Value) -> RetentionRequest {
        RetentionRequest {
            scope: "artifact".to_string(),
            rule: "dev-builds".to_string(),
            params,
            now: 1_000_000,
            candidates,
        }
    }

    #[test]
    fn releases_are_never_deleted() {
        let plan = select(&request(
            vec![
                candidate("1", "m1", "1.0.0", 1),
                candidate("2", "m1", "1.0.1", 2),
                candidate("3", "m1", "1.0.2", 3),
                candidate("4", "m1", "1.0.3", 4),
            ],
            serde_json::json!({ "keepPerModule": 0 }),
        ));
        assert!(
            plan.delete.is_empty(),
            "a release has no pre-release marker: {plan:?}"
        );
    }

    #[test]
    fn the_newest_dev_builds_per_module_survive() {
        let plan = select(&request(
            vec![
                candidate("1", "m1", "1.0.0-dev", 10),
                candidate("2", "m1", "1.0.1-dev", 20),
                candidate("3", "m1", "1.0.2-dev", 30),
                candidate("4", "m1", "1.0.3-dev", 40),
            ],
            serde_json::json!({ "keepPerModule": 2 }),
        ));
        // Newest two (40, 30) stay; the older two go.
        assert_eq!(plan.delete, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn keeping_is_counted_per_module_not_overall() {
        let plan = select(&request(
            vec![
                candidate("1", "m1", "1.0.0-dev", 10),
                candidate("2", "m1", "1.0.1-dev", 20),
                candidate("3", "m2", "2.0.0-dev", 30),
                candidate("4", "m2", "2.0.1-dev", 40),
            ],
            serde_json::json!({ "keepPerModule": 1 }),
        ));
        // One per module survives, so the older of each pair goes.
        assert_eq!(plan.delete, vec!["1".to_string(), "3".to_string()]);
    }

    #[test]
    fn anything_a_distribution_set_offers_is_kept() {
        let mut in_ds = candidate("1", "m1", "1.0.0-dev", 10);
        in_ds.in_distribution_set = true;
        let plan = select(&request(
            vec![in_ds, candidate("2", "m1", "1.0.1-dev", 20)],
            serde_json::json!({ "keepPerModule": 0 }),
        ));
        assert_eq!(plan.delete, vec!["2".to_string()]);
    }

    #[test]
    fn something_a_device_was_told_to_install_is_kept_by_default() {
        let mut assigned = candidate("1", "m1", "1.0.0-dev", 10);
        assigned.ever_assigned = true;
        let plan = select(&request(
            vec![assigned.clone()],
            serde_json::json!({ "keepPerModule": 0 }),
        ));
        assert!(plan.delete.is_empty(), "kept unless explicitly allowed");

        let plan = select(&request(
            vec![assigned],
            serde_json::json!({ "keepPerModule": 0, "deleteAssigned": true }),
        ));
        assert_eq!(plan.delete, vec!["1".to_string()]);
    }

    #[test]
    fn a_custom_marker_list_replaces_the_default() {
        let plan = select(&request(
            vec![candidate("1", "m1", "1.0.0~nightly", 10)],
            serde_json::json!({ "keepPerModule": 0, "releasePattern": ["~nightly"] }),
        ));
        assert_eq!(plan.delete, vec!["1".to_string()]);
    }

    #[test]
    fn the_plan_is_stable_for_the_same_input() {
        let build = || {
            select(&request(
                vec![
                    candidate("9", "m1", "1.0.0-dev", 10),
                    candidate("3", "m2", "2.0.0-dev", 20),
                    candidate("5", "m3", "3.0.0-dev", 30),
                ],
                serde_json::json!({ "keepPerModule": 0 }),
            ))
        };
        // A dry run has to predict the real one, and HashMap iteration does
        // not promise an order.
        assert_eq!(build().delete, build().delete);
        assert_eq!(build().delete, vec!["3", "5", "9"]);
    }
}
