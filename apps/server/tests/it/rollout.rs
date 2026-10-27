//! Rollout engine + node-role integration tests (Phase 4). Run via `make it`.
//!
//! Tests drive the engine deterministically by calling `rollout::tick`
//! instead of relying on the background loop.

use crate::common;

use hawier_api::codes::{ActionType, RolloutGroupStatus, RolloutStatus};
use hawier_client::{CycleOutcome, DdiAuth, DdiClient, Device, DeviceOptions};
use serde_json::{json, Value};
use sqlx::Row;

struct Api {
    http: reqwest::Client,
    base: String,
}

impl Api {
    fn new(base: &str) -> Self {
        Self {
            http: common::http(),
            base: base.to_string(),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .header("Authorization", common::basic("admin", "admin"))
    }

    async fn get(&self, path: &str) -> Value {
        self.req(reqwest::Method::GET, path)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        let resp = self
            .req(reqwest::Method::POST, path)
            .json(body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

async fn tick(server: &common::TestServer) {
    hawier_server::rollout::tick(&server.state).await.unwrap();
}

async fn rollout_status(server: &common::TestServer, id: i64) -> i32 {
    sqlx::query_scalar("SELECT status FROM sp_rollout WHERE id = $1")
        .bind(id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}

async fn run_device_once(
    server: &common::TestServer,
    target: &common::SeededTarget,
) -> CycleOutcome {
    let client = DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken(target.token.clone()),
    );
    let mut device = Device::new(
        client,
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    device.cycle().await.unwrap()
}

/// Seed 4 targets + a DS, create a 2-group rollout via the API.
async fn seed_rollout(server: &common::TestServer, api: &Api) -> (i64, Vec<common::SeededTarget>) {
    let mut targets = Vec::new();
    for n in 0..4 {
        targets.push(common::seed_target(server, "DEFAULT", &format!("ro-dev-{n}")).await);
    }
    let ds =
        common::seed_ds_with_artifact(server, "DEFAULT", "rollout-fw", b"rollout-payload").await;

    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "fleet-update",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==ro-dev-*",
                "amountGroups": 2,
                "successCondition": { "condition": "THRESHOLD", "expression": "100" },
                "errorCondition": { "condition": "THRESHOLD", "expression": "50" },
                "type": "forced"
            }),
        )
        .await;
    assert_eq!(status, 201, "rollout create failed: {rollout}");
    assert_eq!(rollout["status"], "creating");
    (rollout["id"].as_i64().unwrap(), targets)
}

#[tokio::test]
#[ignore]
async fn rollout_full_lifecycle_two_groups() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let (rollout_id, targets) = seed_rollout(&server, &api).await;

    // Tick 1: materialization → READY, 2 groups × 2 targets.
    tick(&server).await;
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Ready.code()
    );
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["totalTargets"], 4);
    let groups = api
        .get(&format!("/rest/v1/rollouts/{rollout_id}/deploygroups"))
        .await;
    assert_eq!(groups["total"], 2);
    for group in groups["content"].as_array().unwrap() {
        assert_eq!(group["status"], "ready");
        assert_eq!(group["totalTargets"], 2);
    }

    // Start → STARTING; tick 2 creates actions and runs group 1.
    let (status, _) = api
        .post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    assert_eq!(status, 200);
    tick(&server).await;
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Running.code()
    );

    let group_rows =
        sqlx::query("SELECT id, status FROM sp_rollout_group WHERE rollout = $1 ORDER BY id")
            .bind(rollout_id)
            .fetch_all(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(
        group_rows[0].get::<i32, _>("status"),
        RolloutGroupStatus::Running.code()
    );
    assert_eq!(
        group_rows[1].get::<i32, _>("status"),
        RolloutGroupStatus::Scheduled.code()
    );

    // Only group-1 devices see a deployment.
    let mut updated = 0;
    for target in &targets {
        if run_device_once(&server, target).await == CycleOutcome::UpdatedSuccessfully {
            updated += 1;
        }
    }
    assert_eq!(updated, 2, "only the first group is active");

    // Tick 3: group 1 hits 100% success → group 2 activates.
    tick(&server).await;
    let group_rows =
        sqlx::query("SELECT status FROM sp_rollout_group WHERE rollout = $1 ORDER BY id")
            .bind(rollout_id)
            .fetch_all(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(
        group_rows[0].get::<i32, _>("status"),
        RolloutGroupStatus::Finished.code()
    );
    assert_eq!(
        group_rows[1].get::<i32, _>("status"),
        RolloutGroupStatus::Running.code()
    );

    // Group 2 devices update; two more ticks settle the rollout.
    for target in &targets {
        run_device_once(&server, target).await;
    }
    tick(&server).await;
    tick(&server).await;
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Finished.code()
    );

    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "finished");
    assert_eq!(detail["totalTargetsPerStatus"]["finished"], 4);
}

#[tokio::test]
#[ignore]
async fn rollout_error_threshold_pauses() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let (rollout_id, targets) = seed_rollout(&server, &api).await;

    tick(&server).await; // materialize
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await; // start group 1

    // Both group-1 devices fail their update (error threshold is 50%).
    let mut failed = 0;
    for target in &targets {
        let client = DdiClient::new(
            server.base_url.clone(),
            "DEFAULT",
            target.controller_id.clone(),
            DdiAuth::TargetToken(target.token.clone()),
        );
        let mut device = Device::new(
            client,
            DeviceOptions {
                once: true,
                fail_download: true,
                ..Default::default()
            },
        );
        if device.cycle().await.unwrap() == CycleOutcome::UpdateFailed {
            failed += 1;
        }
    }
    assert_eq!(failed, 2);

    tick(&server).await;
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Paused.code()
    );
    let groups = api
        .get(&format!("/rest/v1/rollouts/{rollout_id}/deploygroups"))
        .await;
    assert_eq!(groups["content"][0]["status"], "error");

    // Resume works from mgmt.
    let (status, _) = api
        .post(
            &format!("/rest/v1/rollouts/{rollout_id}/resume"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Running.code()
    );
}

#[tokio::test]
#[ignore]
async fn rollout_approval_workflow() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    common::set_tenant_config(&server, "DEFAULT", "rollout.approval.enabled", "true").await;

    let (rollout_id, _) = seed_rollout(&server, &api).await;
    tick(&server).await;
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::WaitingForApproval.code()
    );

    // Deny.
    let (status, _) = api
        .post(
            &format!("/rest/v1/rollouts/{rollout_id}/deny"),
            &json!({ "remark": "not now" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::ApprovalDenied.code()
    );
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["approvalRemark"], "not now");
    assert_eq!(detail["approveDecidedBy"], "admin");

    // A second rollout gets approved and becomes READY.
    let ds_id: i64 = sqlx::query_scalar("SELECT distribution_set FROM sp_rollout WHERE id = $1")
        .bind(rollout_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    let (_, second) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "fleet-update-2",
                "distributionSetId": ds_id,
                "targetFilterQuery": "controllerid==ro-dev-*",
                "amountGroups": 1
            }),
        )
        .await;
    let second_id = second["id"].as_i64().unwrap();
    tick(&server).await;
    assert_eq!(
        rollout_status(&server, second_id).await,
        RolloutStatus::WaitingForApproval.code()
    );
    let (status, _) = api
        .post(
            &format!("/rest/v1/rollouts/{second_id}/approve"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        rollout_status(&server, second_id).await,
        RolloutStatus::Ready.code()
    );
}

#[tokio::test]
#[ignore]
async fn rollout_trigger_next_group_forces_advance() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let (rollout_id, _) = seed_rollout(&server, &api).await;

    tick(&server).await;
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;

    // No device finished anything, but the operator forces group 2.
    let (status, _) = api
        .post(
            &format!("/rest/v1/rollouts/{rollout_id}/triggerNextGroup"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200);
    let statuses: Vec<i32> =
        sqlx::query_scalar("SELECT status FROM sp_rollout_group WHERE rollout = $1 ORDER BY id")
            .bind(rollout_id)
            .fetch_all(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(
        statuses,
        vec![
            RolloutGroupStatus::Running.code(),
            RolloutGroupStatus::Running.code()
        ]
    );

    // Nothing left → 409.
    let (status, _) = api
        .post(
            &format!("/rest/v1/rollouts/{rollout_id}/triggerNextGroup"),
            &json!({}),
        )
        .await;
    assert_eq!(status, 409);
}

#[tokio::test]
#[ignore]
async fn target_filter_auto_assign_picks_up_matching_target() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);

    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "auto-fw", b"auto").await;
    let (status, filter) = api
        .post(
            "/rest/v1/targetfilters",
            &json!({ "name": "auto-lab", "query": "controllerid==auto-*" }),
        )
        .await;
    assert_eq!(status, 201);
    let filter_id = filter["id"].as_i64().unwrap();
    let (status, _) = api
        .post(
            &format!("/rest/v1/targetfilters/{filter_id}/autoAssignDS"),
            &json!({ "id": ds.ds_id, "type": "forced" }),
        )
        .await;
    assert_eq!(status, 200);

    // New matching target appears; the auto-assign tick picks it up.
    let target = common::seed_target(&server, "DEFAULT", "auto-dev-1").await;
    let outsider = common::seed_target(&server, "DEFAULT", "other-dev-1").await;
    hawier_server::rollout::auto_assign_tick(&server.state)
        .await
        .unwrap();

    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT assigned_distribution_set FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(assigned, Some(ds.ds_id));
    let outsider_assigned: Option<i64> =
        sqlx::query_scalar("SELECT assigned_distribution_set FROM sp_target WHERE id = $1")
            .bind(outsider.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(outsider_assigned, None);

    // Idempotent: a second tick creates no second action.
    hawier_server::rollout::auto_assign_tick(&server.state)
        .await
        .unwrap();
    let actions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE target = $1")
        .bind(target.id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(actions, 1);

    // And the device can complete the auto-assigned update.
    assert_eq!(
        run_device_once(&server, &target).await,
        CycleOutcome::UpdatedSuccessfully
    );
}

#[tokio::test]
#[ignore]
async fn failover_control_node_dies_sub_keeps_devices_updating() {
    // Control node owns the database; a sub node attaches to the same DB.
    let mut server = common::spawn().await;
    let api = Api::new(&server.base_url);
    api.post("/rest/v1/targets", &json!([{ "controllerId": "sub-dev" }]))
        .await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "sub-fw", b"sub").await;

    let token: String =
        sqlx::query_scalar("SELECT sec_token FROM sp_target WHERE controller_id = 'sub-dev'")
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    let target_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_target WHERE controller_id = 'sub-dev'")
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    common::assign_ds(&server, "DEFAULT", target_id, ds.ds_id, ActionType::Forced).await;

    // Boot a sub node against the same database.
    let mut sub_config = common::test_config(&server.db_url);
    sub_config.role = hawier_server::config::Role::Sub;
    let sub_state = hawier_server::init(sub_config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sub_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, hawier_server::app(sub_state))
            .await
            .ok();
    });

    // Mgmt reads work on the sub…
    let sub_api = Api::new(&sub_url);
    let listing = sub_api.get("/rest/v1/targets").await;
    assert_eq!(listing["total"], 1);

    // …writes are rejected…
    let (status, error) = sub_api
        .post("/rest/v1/targets", &json!([{ "controllerId": "nope" }]))
        .await;
    assert_eq!(status, 405);
    assert!(error["errorCode"].as_str().unwrap().contains("readOnly"));

    // FAILOVER: the control node dies for real.
    let control_url = server.base_url.clone();
    server.kill().await;
    let control_dead = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap()
        .get(format!("{control_url}/health"))
        .send()
        .await;
    assert!(control_dead.is_err(), "control node must be unreachable");

    // The device still completes a full update cycle through the sub node
    // (poll → deploymentBase → download + hash verify → feedback → FINISHED).
    let client = DdiClient::new(
        sub_url.clone(),
        "DEFAULT",
        "sub-dev",
        DdiAuth::TargetToken(token.clone()),
    );
    let mut device = Device::new(
        client,
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        device.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    // State is durable in Postgres, not in the dead node.
    let status: i32 = sqlx::query_scalar(
        "SELECT status FROM sp_action WHERE target = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(target_id)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(status, hawier_api::codes::ActionStatus::Finished.code());

    // Follow-up polls against the sub keep working (steady state, no links).
    let client = DdiClient::new(sub_url, "DEFAULT", "sub-dev", DdiAuth::TargetToken(token));
    let base = client.poll().await.unwrap();
    assert!(base.links.contains_key("installedBase"));
}

/// rollout-engine spec, "Late-joining device": a dynamic rollout keeps
/// absorbing matching targets after its static groups are done.
#[tokio::test]
#[ignore]
async fn dynamic_rollout_absorbs_a_late_device() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let first = common::seed_target(&server, "DEFAULT", "late-a").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "late-fw", b"late").await;
    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "late-fleet",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==late-*",
                "amountGroups": 1,
                "dynamic": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;
    assert_eq!(
        run_device_once(&server, &first).await,
        CycleOutcome::UpdatedSuccessfully
    );
    tick(&server).await;

    let late = common::seed_target(&server, "DEFAULT", "late-b").await;
    tick(&server).await;
    tick(&server).await;

    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Running.code()
    );
    let dynamic: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sp_rollout_group WHERE rollout = $1 AND is_dynamic = TRUE",
    )
    .bind(rollout_id)
    .fetch_all(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(dynamic, vec!["late-fleet-dynamic-1"]);
    assert_eq!(
        run_device_once(&server, &late).await,
        CycleOutcome::UpdatedSuccessfully
    );
}

// ---------------------------------------------------------------------------
// Success starts the next group; settling finishes it (rollout-engine spec,
// "Evaluation and advance"). hawkBit 1.0's semantics.
// ---------------------------------------------------------------------------

/// Create `<prefix>-fleet` over the targets `<prefix>-*`, with explicit
/// thresholds.
async fn create_rollout(
    api: &Api,
    prefix: &str,
    ds_id: i64,
    groups: i64,
    success: &str,
    error: &str,
    dynamic: bool,
) -> i64 {
    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": format!("{prefix}-fleet"),
                "distributionSetId": ds_id,
                "targetFilterQuery": format!("controllerid=={prefix}-*"),
                "amountGroups": groups,
                "successCondition": { "condition": "THRESHOLD", "expression": success },
                "errorCondition": { "condition": "THRESHOLD", "expression": error },
                "type": "forced",
                "dynamic": dynamic,
            }),
        )
        .await;
    assert_eq!(status, 201, "rollout create failed: {rollout}");
    rollout["id"].as_i64().unwrap()
}

/// Group statuses in id order.
async fn group_statuses(server: &common::TestServer, rollout_id: i64) -> Vec<i32> {
    sqlx::query_scalar("SELECT status FROM sp_rollout_group WHERE rollout = $1 ORDER BY id")
        .bind(rollout_id)
        .fetch_all(&server.state.pool)
        .await
        .unwrap()
}

/// Controller ids per group, groups in id order.
async fn members_per_group(server: &common::TestServer, rollout_id: i64) -> Vec<Vec<String>> {
    let rows = sqlx::query(
        "SELECT g.id, t.controller_id FROM sp_rollout_target_group l
         JOIN sp_rollout_group g ON g.id = l.rollout_group
         JOIN sp_target t ON t.id = l.target
         WHERE g.rollout = $1 ORDER BY g.id, t.controller_id",
    )
    .bind(rollout_id)
    .fetch_all(&server.state.pool)
    .await
    .unwrap();
    let mut groups: Vec<(i64, Vec<String>)> = Vec::new();
    for row in &rows {
        let id: i64 = row.get("id");
        if groups.last().map(|(g, _)| *g) != Some(id) {
            groups.push((id, Vec::new()));
        }
        groups.last_mut().unwrap().1.push(row.get("controller_id"));
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// The report this was found by: 42 targets, 3 groups, a 50% success
/// threshold, 6 devices that never poll. The rollout went FINISHED with 6
/// actions still running and every group "finished".
#[tokio::test]
#[ignore]
async fn devices_that_never_poll_keep_the_rollout_running() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let mut targets = Vec::new();
    for n in 0..42 {
        targets.push(common::seed_target(&server, "DEFAULT", &format!("half-{n:02}")).await);
    }
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "half-fw", b"half").await;
    let rollout_id = create_rollout(&api, "half", ds.ds_id, 3, "50", "50", false).await;
    tick(&server).await; // materialize
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await; // group 1 runs

    // Two silent devices in each group, so every group is past 50% but none
    // has settled.
    let groups = members_per_group(&server, rollout_id).await;
    assert_eq!(groups.len(), 3);
    let silent: Vec<String> = groups.iter().flat_map(|g| g[..2].to_vec()).collect();
    assert_eq!(silent.len(), 6);

    for _ in 0..4 {
        for target in targets
            .iter()
            .filter(|t| !silent.contains(&t.controller_id))
        {
            run_device_once(&server, target).await;
        }
        tick(&server).await;
    }

    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "running", "{detail}");
    assert_eq!(detail["totalTargetsPerStatus"]["finished"], 36, "{detail}");
    assert_eq!(detail["totalTargetsPerStatus"]["running"], 6, "{detail}");
    // All three past their threshold, all three still waiting for someone.
    assert_eq!(
        group_statuses(&server, rollout_id).await,
        vec![RolloutGroupStatus::Running.code(); 3]
    );

    // The stragglers turn up: now everything settles.
    for target in targets.iter().filter(|t| silent.contains(&t.controller_id)) {
        assert_eq!(
            run_device_once(&server, target).await,
            CycleOutcome::UpdatedSuccessfully
        );
    }
    tick(&server).await;
    assert_eq!(
        group_statuses(&server, rollout_id).await,
        vec![RolloutGroupStatus::Finished.code(); 3]
    );
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "finished", "{detail}");
    assert_eq!(detail["totalTargetsPerStatus"]["finished"], 42, "{detail}");
}

/// A group with nothing left to wait for is finished even below its success
/// threshold; otherwise the rollout would never move on.
#[tokio::test]
#[ignore]
async fn a_settled_group_finishes_below_its_threshold() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let mut targets = Vec::new();
    for n in 0..4 {
        targets.push(common::seed_target(&server, "DEFAULT", &format!("settle-{n}")).await);
    }
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "settle-fw", b"settle").await;
    let rollout_id = create_rollout(&api, "settle", ds.ds_id, 2, "100", "80", false).await;
    tick(&server).await;
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;

    // Group 1: one success, one failure. 50% is short of the success
    // threshold and short of the error threshold.
    let groups = members_per_group(&server, rollout_id).await;
    let target = |id: &str| targets.iter().find(|t| t.controller_id == id).unwrap();
    assert_eq!(
        run_device_once(&server, target(&groups[0][0])).await,
        CycleOutcome::UpdatedSuccessfully
    );
    let failing = target(&groups[0][1]);
    let mut device = Device::new(
        DdiClient::new(
            server.base_url.clone(),
            "DEFAULT",
            failing.controller_id.clone(),
            DdiAuth::TargetToken(failing.token.clone()),
        ),
        DeviceOptions {
            once: true,
            fail_download: true,
            ..Default::default()
        },
    );
    assert_eq!(device.cycle().await.unwrap(), CycleOutcome::UpdateFailed);

    tick(&server).await;
    assert_eq!(
        group_statuses(&server, rollout_id).await,
        vec![
            RolloutGroupStatus::Finished.code(),
            RolloutGroupStatus::Running.code()
        ]
    );
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Running.code()
    );
}

/// A device of the last group that never polls must not keep newcomers out
/// of a dynamic rollout: the last group's success is what lets them in.
#[tokio::test]
#[ignore]
async fn a_dynamic_rollout_grows_past_a_straggler() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let first = common::seed_target(&server, "DEFAULT", "strag-a").await;
    let _silent = common::seed_target(&server, "DEFAULT", "strag-b").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "strag-fw", b"strag").await;
    let rollout_id = create_rollout(&api, "strag", ds.ds_id, 1, "50", "50", true).await;
    tick(&server).await;
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;
    assert_eq!(
        run_device_once(&server, &first).await,
        CycleOutcome::UpdatedSuccessfully
    );
    tick(&server).await;

    let late = common::seed_target(&server, "DEFAULT", "strag-c").await;
    tick(&server).await;

    let dynamic: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sp_rollout_group WHERE rollout = $1 AND is_dynamic = TRUE",
    )
    .bind(rollout_id)
    .fetch_all(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(dynamic, vec!["strag-fleet-dynamic-1"]);
    assert_eq!(
        run_device_once(&server, &late).await,
        CycleOutcome::UpdatedSuccessfully
    );
    // The static group still waits for strag-b.
    assert_eq!(
        group_statuses(&server, rollout_id).await[0],
        RolloutGroupStatus::Running.code()
    );
    assert_eq!(
        rollout_status(&server, rollout_id).await,
        RolloutStatus::Running.code()
    );
}

// ---------------------------------------------------------------------------
// Target type compatibility (rollout-engine spec)
// ---------------------------------------------------------------------------

async fn group_members(server: &common::TestServer, rollout_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT t.controller_id FROM sp_rollout_target_group l
         JOIN sp_rollout_group g ON g.id = l.rollout_group
         JOIN sp_target t ON t.id = l.target
         WHERE g.rollout = $1 ORDER BY t.controller_id",
    )
    .bind(rollout_id)
    .fetch_all(&server.state.pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore]
async fn rollout_takes_only_compatible_targets() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let rev2 = common::seed_revision(&server, "rev2").await;
    let one = common::seed_target(&server, "DEFAULT", "mix-rev1").await;
    let two = common::seed_target(&server, "DEFAULT", "mix-rev2").await;
    common::set_target_type(&server, one.id, rev1.target_type).await;
    common::set_target_type(&server, two.id, rev2.target_type).await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mix-fw", b"mix").await;
    common::set_ds_type(&server, ds.ds_id, rev1.ds_type).await;

    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "mixed-fleet",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==mix-*",
                "amountGroups": 1,
            }),
        )
        .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;

    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "ready", "{detail}");
    assert_eq!(detail["totalTargets"], 1);
    assert_eq!(group_members(&server, rollout_id).await, vec!["mix-rev1"]);

    // Running it never offers the set to the rev2 board.
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;
    assert_eq!(common::action_count(&server, two.id).await, 0);
    assert_eq!(common::action_count(&server, one.id).await, 1);
}

#[tokio::test]
#[ignore]
async fn dynamic_rollout_does_not_grow_into_incompatible_targets() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let rev2 = common::seed_revision(&server, "rev2").await;
    let first = common::seed_target(&server, "DEFAULT", "dyn-a").await;
    common::set_target_type(&server, first.id, rev1.target_type).await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "dyn-fw", b"dyn").await;
    common::set_ds_type(&server, ds.ds_id, rev1.ds_type).await;

    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "dyn-fleet",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==dyn-*",
                "amountGroups": 1,
                "dynamic": true,
            }),
        )
        .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;
    api.post(&format!("/rest/v1/rollouts/{rollout_id}/start"), &json!({}))
        .await;
    tick(&server).await;
    assert_eq!(
        run_device_once(&server, &first).await,
        CycleOutcome::UpdatedSuccessfully
    );
    tick(&server).await;

    // Two late arrivals match the filter; only the rev1 one may join.
    let late_rev1 = common::seed_target(&server, "DEFAULT", "dyn-b").await;
    let late_rev2 = common::seed_target(&server, "DEFAULT", "dyn-c").await;
    common::set_target_type(&server, late_rev1.id, rev1.target_type).await;
    common::set_target_type(&server, late_rev2.id, rev2.target_type).await;
    tick(&server).await;
    tick(&server).await;

    assert_eq!(
        group_members(&server, rollout_id).await,
        vec!["dyn-a", "dyn-b"]
    );
    assert_eq!(common::action_count(&server, late_rev2.id).await, 0);
}

#[tokio::test]
#[ignore]
async fn auto_assignment_skips_incompatible_targets() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let rev2 = common::seed_revision(&server, "rev2").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "auto-typed", b"x").await;
    common::set_ds_type(&server, ds.ds_id, rev1.ds_type).await;
    let (_, filter) = api
        .post(
            "/rest/v1/targetfilters",
            &json!({ "name": "typed", "query": "controllerid==typed-*" }),
        )
        .await;
    let filter_id = filter["id"].as_i64().unwrap();
    let (status, _) = api
        .post(
            &format!("/rest/v1/targetfilters/{filter_id}/autoAssignDS"),
            &json!({ "id": ds.ds_id, "type": "forced" }),
        )
        .await;
    assert_eq!(status, 200);

    let good = common::seed_target(&server, "DEFAULT", "typed-rev1").await;
    let bad = common::seed_target(&server, "DEFAULT", "typed-rev2").await;
    common::set_target_type(&server, good.id, rev1.target_type).await;
    common::set_target_type(&server, bad.id, rev2.target_type).await;

    hawier_server::rollout::auto_assign_tick(&server.state)
        .await
        .expect("an incompatible target must not fail the tick");
    hawier_server::rollout::auto_assign_tick(&server.state)
        .await
        .unwrap();
    assert_eq!(common::action_count(&server, good.id).await, 1);
    assert_eq!(common::action_count(&server, bad.id).await, 0);
}

#[tokio::test]
#[ignore]
async fn a_hardened_tenant_rolls_out_and_auto_assigns_only_to_typed_targets() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let rev1 = common::seed_revision(&server, "rev1").await;
    common::set_tenant_config(&server, "DEFAULT", "target.type.required", "true").await;
    let typed = common::seed_target(&server, "DEFAULT", "hard-typed").await;
    let untyped = common::seed_target(&server, "DEFAULT", "hard-untyped").await;
    common::set_target_type(&server, typed.id, rev1.target_type).await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "hard-fw", b"hard").await;
    common::set_ds_type(&server, ds.ds_id, rev1.ds_type).await;

    let (status, rollout) = api
        .post(
            "/rest/v1/rollouts",
            &json!({
                "name": "hardened",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==hard-*",
                "amountGroups": 1,
            }),
        )
        .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["totalTargets"], 1, "{detail}");
    assert_eq!(group_members(&server, rollout_id).await, vec!["hard-typed"]);

    let ds2 = common::seed_ds_with_artifact(&server, "DEFAULT", "hard-auto", b"auto").await;
    common::set_ds_type(&server, ds2.ds_id, rev1.ds_type).await;
    let (_, filter) = api
        .post(
            "/rest/v1/targetfilters",
            &json!({ "name": "hard", "query": "controllerid==hard-*" }),
        )
        .await;
    let filter_id = filter["id"].as_i64().unwrap();
    api.post(
        &format!("/rest/v1/targetfilters/{filter_id}/autoAssignDS"),
        &json!({ "id": ds2.ds_id, "type": "forced" }),
    )
    .await;
    hawier_server::rollout::auto_assign_tick(&server.state)
        .await
        .expect("an untyped target must not fail the tick");
    assert_eq!(common::action_count(&server, untyped.id).await, 0);
    assert_eq!(common::action_count(&server, typed.id).await, 1);
}
