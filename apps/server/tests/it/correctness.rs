//! Assignment-semantics matrix: implicit lock, autoclose, multi.assignments,
//! weight ordering and housekeeping. Run via `make it`.

use crate::common;

use hawier_api::codes::{ActionStatus, ActionType, TargetUpdateStatus};
use hawier_client::{DdiAuth, DdiClient};
use sqlx::Row;

fn device(server: &common::TestServer, target: &common::SeededTarget) -> DdiClient {
    DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken(target.token.clone()),
    )
}

async fn unlock_ds(server: &common::TestServer, ds: &common::SeededDs) {
    sqlx::query("UPDATE sp_distribution_set SET locked = FALSE WHERE id = $1")
        .bind(ds.ds_id)
        .execute(&server.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sp_software_module SET locked = FALSE WHERE id = $1")
        .bind(ds.sm_id)
        .execute(&server.state.pool)
        .await
        .unwrap();
}

async fn ds_locked(server: &common::TestServer, ds_id: i64) -> bool {
    sqlx::query_scalar("SELECT locked FROM sp_distribution_set WHERE id = $1")
        .bind(ds_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}

async fn sm_locked(server: &common::TestServer, sm_id: i64) -> bool {
    sqlx::query_scalar("SELECT locked FROM sp_software_module WHERE id = $1")
        .bind(sm_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}

async fn action_state(server: &common::TestServer, action_id: i64) -> (bool, i32) {
    let row = sqlx::query("SELECT active, status FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    (
        row.get::<Option<bool>, _>("active").unwrap_or(false),
        row.get("status"),
    )
}

#[tokio::test]
#[ignore]
async fn implicit_lock_locks_ds_and_modules_on_assignment() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-lock").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "lock-fw", b"lock").await;
    unlock_ds(&server, &ds).await;
    assert!(!ds_locked(&server, ds.ds_id).await);

    common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    assert!(ds_locked(&server, ds.ds_id).await, "DS must be locked");
    assert!(sm_locked(&server, ds.sm_id).await, "SM must be locked");
}

#[tokio::test]
#[ignore]
async fn implicit_lock_disabled_leaves_ds_unlocked() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-nolock").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "nolock-fw", b"nolock").await;
    unlock_ds(&server, &ds).await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_IMPLICIT_LOCK_ENABLED,
        "false",
    )
    .await;

    common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    assert!(!ds_locked(&server, ds.ds_id).await);
    assert!(!sm_locked(&server, ds.sm_id).await);
}

#[tokio::test]
#[ignore]
async fn autoclose_closes_superseded_action_without_cancel_handshake() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-autoclose").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "ac-a", b"a").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "ac-b", b"b").await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_ACTIONS_AUTOCLOSE_ENABLED,
        "true",
    )
    .await;

    let first = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
    )
    .await;
    let second = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
    )
    .await;

    // Old action closed as CANCELED right away — no CANCELING handshake.
    let (active, status) = action_state(&server, first).await;
    assert!(!active);
    assert_eq!(status, ActionStatus::Canceled.code());

    // The device sees only the new deployment, never a cancelAction.
    let base = device(&server, &target).poll().await.unwrap();
    assert!(!base.links.contains_key("cancelAction"));
    let href = &base.links.get("deploymentBase").unwrap().href;
    assert!(href.contains(&format!("/deploymentBase/{second}")));
}

#[tokio::test]
#[ignore]
async fn default_supersede_uses_canceling_handshake() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-softcancel").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "sc-a", b"a").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "sc-b", b"b").await;

    let first = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
    )
    .await;
    common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
    )
    .await;

    let (active, status) = action_state(&server, first).await;
    assert!(active, "soft-canceled action stays active until confirmed");
    assert_eq!(status, ActionStatus::Canceling.code());
}

#[tokio::test]
#[ignore]
async fn multi_assignments_keep_both_actions_active_weight_wins() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-multi").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "multi-a", b"a").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "multi-b", b"b").await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        "true",
    )
    .await;

    let low = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
        500,
        None,
    )
    .await;
    let high = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
        700,
        None,
    )
    .await;

    // Nothing superseded: both actions still active.
    assert!(action_state(&server, low).await.0);
    assert!(action_state(&server, high).await.0);

    // DDI serves the heavier action even though it is newer.
    let base = device(&server, &target).poll().await.unwrap();
    let href = &base.links.get("deploymentBase").unwrap().href;
    assert!(
        href.contains(&format!("/deploymentBase/{high}")),
        "expected weight-700 action {high}, got {href}"
    );
}

#[tokio::test]
#[ignore]
async fn equal_weight_serves_oldest_action_first() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-eqweight").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "eq-a", b"a").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "eq-b", b"b").await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        "true",
    )
    .await;

    let first = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
        600,
        None,
    )
    .await;
    common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
        600,
        None,
    )
    .await;

    let base = device(&server, &target).poll().await.unwrap();
    let href = &base.links.get("deploymentBase").unwrap().href;
    assert!(
        href.contains(&format!("/deploymentBase/{first}")),
        "equal weight → oldest id first, got {href}"
    );
}

#[tokio::test]
#[ignore]
async fn mgmt_repeat_assignment_only_allowed_with_multi_assignments() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-repeat").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "repeat-fw", b"r").await;
    let http = common::http();
    let url = format!(
        "{}/rest/v1/targets/{}/assignedDS",
        server.base_url, target.controller_id
    );
    let body = serde_json::json!([{ "id": ds.ds_id }]);

    // Default mode: second identical assignment is a no-op.
    for expected_assigned in [1, 0] {
        let resp: serde_json::Value = http
            .post(&url)
            .header("Authorization", common::basic("admin", "admin"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["assigned"], expected_assigned, "resp: {resp}");
    }

    // Multi-assignment mode: the same DS can be assigned again.
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        "true",
    )
    .await;
    let resp: serde_json::Value = http
        .post(&url)
        .header("Authorization", common::basic("admin", "admin"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["assigned"], 1, "resp: {resp}");
}

#[tokio::test]
#[ignore]
async fn housekeeping_deletes_expired_actions_and_sessions() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-cleanup").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "hk-a", b"a").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "hk-b", b"b").await;

    // A canceled, stale action and a fresh running one.
    let stale = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
    )
    .await;
    sqlx::query(
        "UPDATE sp_action SET active = FALSE, status = $1, last_modified_at = 1000 WHERE id = $2",
    )
    .bind(ActionStatus::Canceled.code())
    .bind(stale)
    .execute(&server.state.pool)
    .await
    .unwrap();
    let fresh = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
    )
    .await;

    // An expired UI session.
    let session_id = server.state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_session (id, token, tenant, username, secret, created_at, expires_at)
         VALUES ($1, $2, 'DEFAULT', 'admin', 's', now(), now() - interval '1 hour')",
    )
    .bind(session_id)
    .bind(hawier_server::db::model::generate_sec_token())
    .execute(&server.state.pool)
    .await
    .unwrap();

    // Cleanup disabled by default: nothing happens.
    hawier_server::housekeeping::tick(&server.state)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE id = $1")
        .bind(stale)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "cleanup must be off without config");

    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_ACTION_CLEANUP_EXPIRY,
        "1000",
    )
    .await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_ACTION_CLEANUP_STATUS,
        "CANCELED,ERROR",
    )
    .await;
    hawier_server::housekeeping::tick(&server.state)
        .await
        .unwrap();

    let stale_left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE id = $1")
        .bind(stale)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(stale_left, 0, "stale CANCELED action must be deleted");
    let fresh_left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE id = $1")
        .bind(fresh)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(fresh_left, 1, "fresh RUNNING action must survive");
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hawier_session WHERE id = $1")
        .bind(session_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0, "expired session must be swept");
}

#[tokio::test]
#[ignore]
async fn polite_device_defers_soft_update_until_forced() {
    use hawier_client::{CycleOutcome, Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-defer").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "defer-fw", b"soft").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Soft).await;

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            defer_soft: true,
            ..DeviceOptions::default()
        },
    );
    // Downloads and waits — twice, without duplicated feedback.
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Deferred);
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Deferred);
    let statuses = common::action_statuses(&server, action_id).await;
    let downloaded = statuses
        .iter()
        .filter(|s| **s == ActionStatus::Downloaded.code())
        .count();
    assert_eq!(downloaded, 1, "one DOWNLOADED report: {statuses:?}");
    assert!(action_state(&server, action_id).await.0, "still active");

    // Operator escalates to forced via mgmt — the device now installs.
    let response = common::http()
        .put(format!(
            "{}/rest/v1/targets/{}/actions/{action_id}",
            server.base_url, target.controller_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&serde_json::json!({ "forceType": "forced" }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());

    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );
    let (active, status) = action_state(&server, action_id).await;
    assert!(!active);
    assert_eq!(status, ActionStatus::Finished.code());
}

#[tokio::test]
#[ignore]
async fn multi_assignments_device_completes_both_actions() {
    use hawier_client::{CycleOutcome, Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-multi-e2e").await;
    let ds_a = common::seed_ds_with_artifact(&server, "DEFAULT", "me2e-a", b"aaa").await;
    let ds_b = common::seed_ds_with_artifact(&server, "DEFAULT", "me2e-b", b"bbb").await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        hawier_server::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        "true",
    )
    .await;

    let low = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_a.ds_id,
        ActionType::Forced,
        500,
        None,
    )
    .await;
    let high = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds_b.ds_id,
        ActionType::Forced,
        700,
        None,
    )
    .await;

    let target_row = |server: &common::TestServer| {
        let pool = server.state.pool.clone();
        async move {
            sqlx::query(
                "SELECT installed_distribution_set, update_status FROM sp_target WHERE id = $1",
            )
            .bind(target.id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    let mut dev = Device::new(device(&server, &target), DeviceOptions::default());

    // First cycle installs the heavier (newer) action B.
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );
    let row = target_row(&server).await;
    assert_eq!(
        row.get::<Option<i64>, _>("installed_distribution_set"),
        Some(ds_b.ds_id),
        "B installed first"
    );
    assert_eq!(
        row.get::<i32, _>("update_status"),
        TargetUpdateStatus::Pending.code(),
        "A still active → target stays PENDING"
    );
    let (b_active, b_status) = action_state(&server, high).await;
    assert!(!b_active);
    assert_eq!(b_status, ActionStatus::Finished.code());
    assert!(action_state(&server, low).await.0, "A untouched");

    // Second cycle drains the remaining action A.
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );
    let row = target_row(&server).await;
    assert_eq!(
        row.get::<Option<i64>, _>("installed_distribution_set"),
        Some(ds_a.ds_id),
        "A installed last"
    );
    assert_eq!(
        row.get::<i32, _>("update_status"),
        TargetUpdateStatus::InSync.code(),
        "nothing active → IN_SYNC"
    );
    let (a_active, a_status) = action_state(&server, low).await;
    assert!(!a_active);
    assert_eq!(a_status, ActionStatus::Finished.code());

    // Nothing left for the device.
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Idle);
}

#[tokio::test]
#[ignore]
async fn force_quit_under_device_feet() {
    use hawier_api::ddi::{ExecutionStatus, FinalResult};
    use hawier_client::client::feedback;
    use hawier_client::{CycleOutcome, Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-forcequit").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "fq-fw", b"fq").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    // Device starts working on the deployment by hand: follows the link,
    // reports "proceeding" — and then the operator force-quits the action.
    let client = device(&server, &target);
    let base = client.poll().await.unwrap();
    let href = base.links.get("deploymentBase").unwrap().href.clone();
    client.deployment_base_by_href(&href).await.unwrap();
    client
        .post_feedback_by_href(
            &href,
            &feedback(
                ExecutionStatus::Proceeding,
                FinalResult::None,
                vec!["installing...".into()],
            ),
        )
        .await
        .unwrap();

    let response = common::http()
        .delete(format!(
            "{}/rest/v1/targets/{}/actions/{action_id}?force=true",
            server.base_url, target.controller_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let (active, status) = action_state(&server, action_id).await;
    assert!(!active);
    assert_eq!(status, ActionStatus::Canceled.code());

    // The device's next poll is clean and the loop survives.
    let mut dev = Device::new(device(&server, &target), DeviceOptions::default());
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Idle);

    // A late feedback on the stale href bounces with 410 Gone.
    let err = client
        .post_feedback_by_href(
            &href,
            &feedback(
                ExecutionStatus::Closed,
                FinalResult::Success,
                vec!["too late".into()],
            ),
        )
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("410"),
        "expected 410, got {err}"
    );
}

/// A plain soft update is *not* an update the device may sit on: hawkBit's
/// `attempt` means "install when convenient", and convenient is now unless
/// the device says otherwise. Only `--defer-soft` postpones it, and that
/// path is covered above — this pins the default.
#[tokio::test]
#[ignore]
async fn soft_update_installs_by_default() {
    use hawier_client::{CycleOutcome, Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-soft").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "soft-fw", b"soft-bytes").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Soft).await;

    let mut dev = Device::new(device(&server, &target), DeviceOptions::default());
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully,
        "an attempt update installs on the first cycle"
    );

    let statuses = common::action_statuses(&server, action_id).await;
    assert_eq!(
        statuses.last().copied(),
        Some(ActionStatus::Finished.code()),
        "the action closes: {statuses:?}"
    );
    assert!(
        statuses.contains(&ActionStatus::Retrieved.code())
            && statuses.contains(&ActionStatus::Downloaded.code()),
        "the device fetched and downloaded before closing: {statuses:?}"
    );
    let (active, status) = action_state(&server, action_id).await;
    assert!(!active);
    assert_eq!(status, ActionStatus::Finished.code());

    let update_status: i32 =
        sqlx::query_scalar("SELECT update_status FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(
        update_status,
        TargetUpdateStatus::InSync.code(),
        "the fleet view must show the device as in sync"
    );
}

/// Two processes booted from the same firmware image share a controller id —
/// a cloning accident, but one that happens. This pins what the server does:
/// both are handed the same deployment, and exactly one close is accepted.
/// The loser gets 410 Gone rather than a second, contradictory history.
#[tokio::test]
#[ignore]
async fn two_devices_share_one_controller_id() {
    use hawier_client::{Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-twin").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "twin-fw", b"twin").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let mut first = Device::new(device(&server, &target), DeviceOptions::default());
    let mut second = Device::new(device(&server, &target), DeviceOptions::default());
    let (left, right) = tokio::join!(first.cycle(), second.cycle());

    let winners = [&left, &right].iter().filter(|r| r.is_ok()).count();
    assert!(
        winners >= 1,
        "at least one twin must complete: {left:?} / {right:?}"
    );

    // Whatever the interleaving, the action closes once and stays closed.
    let (active, status) = action_state(&server, action_id).await;
    assert!(!active, "the action is finished exactly once");
    assert_eq!(status, ActionStatus::Finished.code());

    let statuses = common::action_statuses(&server, action_id).await;
    let finished = statuses
        .iter()
        .filter(|status| **status == ActionStatus::Finished.code())
        .count();
    assert_eq!(
        finished, 1,
        "one close means one history — a second would make the device's \
         state unknowable: {statuses:?}"
    );
    let retrieved = statuses
        .iter()
        .filter(|status| **status == ActionStatus::Retrieved.code())
        .count();
    assert_eq!(
        retrieved, 1,
        "the RETRIEVED marker is written once per action, even when two \
         pollers race for it: {statuses:?}"
    );
}

/// A device that keeps reporting the same state must not keep growing the
/// action's history. Under load this used to close an amplification loop:
/// saturation → download misses its window → another `Proceeding` → another
/// row → more work. One benchmark run wrote 2.7M rows that way and pushed
/// every action into the 800-row quota (`bench/NOTES.md` §5).
#[tokio::test]
#[ignore]
async fn repeated_proceeding_is_counted_not_appended() {
    use hawier_api::ddi::{ExecutionStatus, FinalResult};
    use hawier_client::client::feedback;

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-repeat").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "repeat-fw", b"fw").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let client = device(&server, &target);
    let same = || {
        feedback(
            ExecutionStatus::Proceeding,
            FinalResult::None,
            vec!["downloading".to_string()],
        )
    };

    client
        .deployment_feedback(&action_id.to_string(), &same())
        .await
        .unwrap();
    let after_first = common::action_statuses(&server, action_id).await;

    for _ in 0..20 {
        client
            .deployment_feedback(&action_id.to_string(), &same())
            .await
            .unwrap();
    }
    let after_repeats = common::action_statuses(&server, action_id).await;
    assert_eq!(
        after_first, after_repeats,
        "20 identical reports must not add rows: {after_repeats:?}"
    );

    // The operator still has to see that the device kept retrying.
    let row = sqlx::query(
        "SELECT optlock_revision, created_at, last_modified_at FROM sp_action_status
         WHERE action = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(action_id)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<Option<i64>, _>("optlock_revision"),
        Some(21),
        "the repeat counter rides in optlock_revision (1 insert + 20 repeats)"
    );
    assert!(
        row.get::<i64, _>("last_modified_at") >= row.get::<i64, _>("created_at"),
        "the last repeat is timestamped"
    );

    // ...and the mgmt API has to surface it, or the console cannot.
    let statuses: serde_json::Value = common::http()
        .get(format!(
            "{}/rest/v1/targets/{}/actions/{action_id}/status",
            server.base_url, target.controller_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let latest = &statuses["content"][0];
    assert_eq!(latest["repeats"], 20, "repeats in the JSON: {statuses}");
    assert!(latest["lastReportedAt"].is_i64(), "{statuses}");

    // New information still gets its own row: a different message means the
    // device is saying something else, however similar the status code.
    client
        .deployment_feedback(
            &action_id.to_string(),
            &feedback(
                ExecutionStatus::Proceeding,
                FinalResult::None,
                vec!["downloading, attempt 3".to_string()],
            ),
        )
        .await
        .unwrap();
    let after_new_message = common::action_statuses(&server, action_id).await;
    assert_eq!(
        after_new_message.len(),
        after_repeats.len() + 1,
        "a changed message is new information: {after_new_message:?}"
    );
}

/// The collapse must not eat the normal chain: one plain update still walks
/// through every distinct state, in order.
#[tokio::test]
#[ignore]
async fn one_update_still_writes_the_whole_chain() {
    use hawier_client::{CycleOutcome, Device, DeviceOptions};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-chain").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "chain-fw", b"fw").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let mut dev = Device::new(device(&server, &target), DeviceOptions::default());
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    let statuses = common::action_statuses(&server, action_id).await;
    assert_eq!(
        statuses,
        vec![
            ActionStatus::Running.code(),
            ActionStatus::Retrieved.code(),
            ActionStatus::Running.code(),
            ActionStatus::Download.code(),
            ActionStatus::Downloaded.code(),
            ActionStatus::Finished.code(),
        ],
        "the six-row chain of a healthy update: {statuses:?}"
    );
}
