//! Maintenance-window matrix: validation, DDI emission, device behaviour
//! and poll-sleep shortening. Run via `make it`.

use crate::common;

use hawier_api::codes::ActionStatus;
use hawier_api::codes::ActionType;
use hawier_api::ddi::{HandlingType, MaintenanceWindowStatus};
use hawier_client::{CycleOutcome, DdiAuth, DdiClient, Device, DeviceOptions};

/// Fires once, far in the future (Quartz supports a 7th year field):
/// deterministic "window closed, next start exists".
const FUTURE_SCHEDULE: &str = "0 0 0 1 1 ? 2099";
/// Fires every minute with a 10-minute window: always open.
const OPEN_SCHEDULE: &str = "0 * * * * *";

fn device_client(server: &common::TestServer, target: &common::SeededTarget) -> DdiClient {
    DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken(target.token.clone()),
    )
}

#[tokio::test]
#[ignore]
async fn mgmt_rejects_invalid_maintenance_windows() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-mw-bad").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mw-bad", b"x").await;
    let http = common::http();
    let url = format!(
        "{}/rest/v1/targets/{}/assignedDS",
        server.base_url, target.controller_id
    );

    for window in [
        // Garbage cron.
        serde_json::json!({"schedule": "garbage", "duration": "00:10:00", "timezone": "+00:00"}),
        // tz database names are not supported — fixed offsets only.
        serde_json::json!({"schedule": OPEN_SCHEDULE, "duration": "00:10:00", "timezone": "Europe/Berlin"}),
        // Partial triple.
        serde_json::json!({"schedule": OPEN_SCHEDULE}),
        // Schedule that never fires again.
        serde_json::json!({"schedule": "0 0 0 1 1 ? 2001", "duration": "00:10:00", "timezone": "+00:00"}),
    ] {
        let resp = http
            .post(&url)
            .header("Authorization", common::basic("admin", "admin"))
            .json(&serde_json::json!([{ "id": ds.ds_id, "maintenanceWindow": window }]))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "window {window} must be rejected");
    }

    // No action was created by the failed attempts.
    let actions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE target = $1")
        .bind(target.id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(actions, 0);
}

#[tokio::test]
#[ignore]
async fn closed_window_reports_unavailable_and_skips_update() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-mw-closed").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mw-closed", b"payload").await;
    let action_id = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        ActionType::Forced,
        1000,
        Some((FUTURE_SCHEDULE, "01:00:00", "+00:00")),
    )
    .await;

    let client = device_client(&server, &target);
    let base = client.poll().await.unwrap();
    let href = base.links.get("deploymentBase").unwrap().href.clone();
    let deployment = client.deployment_base_by_href(&href).await.unwrap();
    assert_eq!(
        deployment.deployment.maintenance_window,
        Some(MaintenanceWindowStatus::Unavailable)
    );
    assert_eq!(deployment.deployment.update, HandlingType::Skip);
    assert_eq!(deployment.deployment.download, HandlingType::Forced);

    // The simulated device downloads, reports `downloaded`, and waits.
    let mut dev = Device::new(device_client(&server, &target), DeviceOptions::default());
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::WaitingForWindow);
    // Second cycle: still waiting, no duplicated download/feedback.
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::WaitingForWindow);

    let statuses = common::action_statuses(&server, action_id).await;
    let downloaded = statuses
        .iter()
        .filter(|s| **s == ActionStatus::Downloaded.code())
        .count();
    assert_eq!(downloaded, 1, "exactly one DOWNLOADED report: {statuses:?}");

    // Action stays open — the server never closes it while the window is shut.
    let active: Option<bool> = sqlx::query_scalar("SELECT active FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(active, Some(true));
}

#[tokio::test]
#[ignore]
async fn open_window_reports_available_and_installs() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-mw-open").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mw-open", b"payload").await;
    let action_id = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        ActionType::Forced,
        1000,
        Some((OPEN_SCHEDULE, "00:10:00", "+00:00")),
    )
    .await;

    let client = device_client(&server, &target);
    let base = client.poll().await.unwrap();
    let href = base.links.get("deploymentBase").unwrap().href.clone();
    let deployment = client.deployment_base_by_href(&href).await.unwrap();
    assert_eq!(
        deployment.deployment.maintenance_window,
        Some(MaintenanceWindowStatus::Available)
    );
    assert_eq!(deployment.deployment.update, HandlingType::Forced);

    let mut dev = Device::new(device_client(&server, &target), DeviceOptions::default());
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    let (active, status): (Option<bool>, i32) =
        sqlx::query_as("SELECT active, status FROM sp_action WHERE id = $1")
            .bind(action_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(active, Some(false));
    assert_eq!(status, ActionStatus::Finished.code());
}

#[tokio::test]
#[ignore]
async fn poll_sleep_shortens_ahead_of_next_window() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-mw-sleep").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mw-sleep", b"x").await;
    // 1-second window at each minute boundary: next start is < 60s away,
    // so the shortened sleep (next/3, floor 30s) beats the 5-minute default.
    common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        ActionType::Forced,
        1000,
        Some((OPEN_SCHEDULE, "00:00:01", "+00:00")),
    )
    .await;

    let client = device_client(&server, &target);
    let base = client.poll().await.unwrap();
    let href = base.links.get("deploymentBase").unwrap().href.clone();
    let deployment = client.deployment_base_by_href(&href).await.unwrap();

    let secs = {
        let mut parts = base.config.polling.sleep.split(':');
        let h: u64 = parts.next().unwrap().parse().unwrap();
        let m: u64 = parts.next().unwrap().parse().unwrap();
        let s: u64 = parts.next().unwrap().parse().unwrap();
        h * 3600 + m * 60 + s
    };
    match deployment.deployment.maintenance_window {
        Some(MaintenanceWindowStatus::Unavailable) => {
            assert!(
                (30..=60).contains(&secs),
                "shortened sleep expected, got {} ({secs}s)",
                base.config.polling.sleep
            );
        }
        // The rare case: the window opened between poll and fetch — either
        // value is legitimate, only assert nothing longer than default.
        _ => assert!(secs <= 300, "sleep must never exceed default: {secs}s"),
    }
}

#[tokio::test]
#[ignore]
async fn window_opens_and_waiting_device_installs() {
    use chrono::{Datelike, Timelike};

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-mw-transition").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mw-transition", b"payload").await;

    // A one-shot window opening at a concrete UTC second ~6s from now
    // (7-field Quartz expression with the year — deterministic transition).
    let opens_at = chrono::Utc::now() + chrono::Duration::seconds(6);
    let schedule = format!(
        "{} {} {} {} {} * {}",
        opens_at.second(),
        opens_at.minute(),
        opens_at.hour(),
        opens_at.day(),
        opens_at.month(),
        opens_at.year(),
    );
    let action_id = common::assign_ds_full(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        ActionType::Forced,
        1000,
        Some((&schedule, "00:01:00", "+00:00")),
    )
    .await;

    let mut dev = Device::new(device_client(&server, &target), DeviceOptions::default());
    // Window still closed: download once and wait.
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::WaitingForWindow);
    assert!(
        sqlx::query_scalar::<_, Option<bool>>("SELECT active FROM sp_action WHERE id = $1")
            .bind(action_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap()
            .unwrap_or(false),
        "action stays open while waiting"
    );

    // Let the window open, then the same device applies the update.
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    let (active, status): (Option<bool>, i32) =
        sqlx::query_as("SELECT active, status FROM sp_action WHERE id = $1")
            .bind(action_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(active, Some(false));
    assert_eq!(status, ActionStatus::Finished.code());

    // Exactly one DOWNLOADED report — the wait produced no feedback spam.
    let statuses = common::action_statuses(&server, action_id).await;
    let downloaded = statuses
        .iter()
        .filter(|s| **s == ActionStatus::Downloaded.code())
        .count();
    assert_eq!(downloaded, 1, "one DOWNLOADED report: {statuses:?}");
}
