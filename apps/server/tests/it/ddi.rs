//! Client↔DDI integration matrix (Phase 2). Run via `make it`.

use crate::common;

use hawier_api::codes::{ActionStatus, ActionType, TargetUpdateStatus};
use hawier_client::{CycleOutcome, DdiAuth, DdiClient, Device, DeviceOptions};
use sqlx::Row;

fn device(server: &common::TestServer, target: &common::SeededTarget) -> DdiClient {
    DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken(target.token.clone()),
    )
}

#[tokio::test]
#[ignore]
async fn ddi_poll_no_action_returns_polling_sleep_and_no_links() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-idle").await;

    let base = device(&server, &target).poll().await.unwrap();
    assert_eq!(base.config.polling.sleep, "00:05:00");
    assert!(
        base.links.is_empty(),
        "no links expected, got {:?}",
        base.links.keys()
    );

    // Poll touch persisted.
    let last_query: Option<i64> =
        sqlx::query_scalar("SELECT last_target_query FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert!(last_query.is_some());
}

#[tokio::test]
#[ignore]
async fn ddi_wrong_target_token_returns_401() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-badtoken").await;

    let bad = DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken("00000000000000000000000000000000".into()),
    );
    let err = bad.poll().await.unwrap_err();
    assert!(
        format!("{err:#}").contains("401"),
        "expected 401, got: {err}"
    );

    // No auth header at all → 401 too (anonymous disabled).
    let anon = DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id,
        DdiAuth::None,
    );
    let err = anon.poll().await.unwrap_err();
    assert!(format!("{err:#}").contains("401"));
}

#[tokio::test]
#[ignore]
async fn ddi_gateway_token_toggle_and_autoregistration() {
    let server = common::spawn().await;
    hawier_server::tenant::ensure_tenant(&server.state.pool, "DEFAULT")
        .await
        .unwrap();

    let gateway = DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        "dev-pnp",
        DdiAuth::GatewayToken("gw-secret".into()),
    );

    // Disabled (default) → 401.
    let err = gateway.poll().await.unwrap_err();
    assert!(format!("{err:#}").contains("401"));

    // Enable via tenant config → unknown target auto-registers as REGISTERED.
    common::set_tenant_config(
        &server,
        "DEFAULT",
        "authentication.gatewaytoken.enabled",
        "true",
    )
    .await;
    common::set_tenant_config(
        &server,
        "DEFAULT",
        "authentication.gatewaytoken.key",
        "gw-secret",
    )
    .await;
    let base = gateway.poll().await.unwrap();
    assert!(
        base.links.contains_key("configData"),
        "fresh target should be asked for attributes"
    );

    let row = sqlx::query(
        "SELECT update_status, created_by FROM sp_target WHERE controller_id = 'dev-pnp'",
    )
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<i32, _>("update_status"),
        TargetUpdateStatus::Registered.code()
    );
    assert_eq!(
        row.get::<String, _>("created_by"),
        "CONTROLLER_PLUG_AND_PLAY"
    );

    // Wrong gateway token still 401.
    let bad = DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        "dev-pnp",
        DdiAuth::GatewayToken("wrong".into()),
    );
    let err = bad.poll().await.unwrap_err();
    assert!(format!("{err:#}").contains("401"));
}

#[tokio::test]
#[ignore]
async fn ddi_assign_ds_makes_deployment_base_link_appear_and_retrieved_written_once() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-assign").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "firmware-a", b"payload-a").await;

    let client = device(&server, &target);
    assert!(!client
        .poll()
        .await
        .unwrap()
        .links
        .contains_key("deploymentBase"));

    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let base = client.poll().await.unwrap();
    let link = base
        .links
        .get("deploymentBase")
        .expect("deploymentBase link");
    assert!(link.href.contains(&format!("/deploymentBase/{action_id}")));

    // GET twice → exactly one RETRIEVED row.
    let dep = client.deployment_base_by_href(&link.href).await.unwrap();
    assert_eq!(dep.id, action_id.to_string());
    assert_eq!(dep.deployment.chunks.len(), 1);
    assert_eq!(dep.deployment.chunks[0].part, "os");
    assert_eq!(dep.deployment.chunks[0].artifacts.len(), 1);
    assert_eq!(dep.deployment.chunks[0].artifacts[0].hashes.sha1, ds.sha1);
    client.deployment_base_by_href(&link.href).await.unwrap();

    let retrieved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_action_status WHERE action = $1 AND status = $2",
    )
    .bind(action_id)
    .bind(ActionStatus::Retrieved.code())
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(retrieved, 1);
}

#[tokio::test]
#[ignore]
async fn ddi_full_update_cycle_to_finished() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-cycle").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "firmware-b", b"payload-b").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    let outcome = dev.cycle().await.unwrap();
    assert_eq!(outcome, CycleOutcome::UpdatedSuccessfully);

    // Action closed as FINISHED, target IN_SYNC with installed DS.
    let action = sqlx::query("SELECT status, active FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        action.get::<i32, _>("status"),
        ActionStatus::Finished.code()
    );
    assert!(!action.get::<bool, _>("active"));

    let trow = sqlx::query(
        "SELECT update_status, installed_distribution_set, install_date
         FROM sp_target WHERE id = $1",
    )
    .bind(target.id)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(
        trow.get::<i32, _>("update_status"),
        TargetUpdateStatus::InSync.code()
    );
    assert_eq!(
        trow.get::<Option<i64>, _>("installed_distribution_set"),
        Some(ds.ds_id)
    );
    assert!(trow.get::<Option<i64>, _>("install_date").is_some());

    // Status trail: RUNNING (assign), RETRIEVED, RUNNING (proceeding),
    // DOWNLOAD (server log), DOWNLOADED, FINISHED.
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
        "unexpected status trail: {statuses:?}"
    );
}

#[tokio::test]
#[ignore]
async fn ddi_closed_failure_sets_action_and_target_error() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-fail").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "firmware-c", b"payload-c").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            fail_download: true,
            ..Default::default()
        },
    );
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::UpdateFailed);

    let action = sqlx::query("SELECT status, active FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(action.get::<i32, _>("status"), ActionStatus::Error.code());
    assert!(!action.get::<bool, _>("active"));

    let update_status: i32 =
        sqlx::query_scalar("SELECT update_status FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(update_status, TargetUpdateStatus::Error.code());

    // Feedback on the closed action → 410 Gone.
    let client = device(&server, &target);
    let err = client
        .deployment_feedback(
            &action_id.to_string(),
            &hawier_client::client::feedback(
                hawier_api::ddi::ExecutionStatus::Closed,
                hawier_api::ddi::FinalResult::Success,
                vec![],
            ),
        )
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("410"),
        "expected 410, got {err}"
    );
}

#[tokio::test]
#[ignore]
async fn ddi_download_only_closes_at_downloaded() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-dlonly").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "firmware-d", b"payload-d").await;
    let action_id = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        ActionType::DownloadOnly,
    )
    .await;

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    let action = sqlx::query("SELECT status, active FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        action.get::<i32, _>("status"),
        ActionStatus::Downloaded.code()
    );
    assert!(!action.get::<bool, _>("active"));

    let update_status: i32 =
        sqlx::query_scalar("SELECT update_status FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(update_status, TargetUpdateStatus::InSync.code());
}

#[tokio::test]
#[ignore]
async fn ddi_cancel_flow_and_rejected_cancel() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-cancel").await;
    let ds1 = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-old", b"old").await;
    let ds2 = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-new", b"new").await;

    let first =
        common::assign_ds(&server, "DEFAULT", target.id, ds1.ds_id, ActionType::Forced).await;
    // Second assignment soft-cancels the first.
    let second =
        common::assign_ds(&server, "DEFAULT", target.id, ds2.ds_id, ActionType::Forced).await;

    let first_status: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(first)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(first_status, ActionStatus::Canceling.code());

    // Device sees the cancel first.
    let client = device(&server, &target);
    let base = client.poll().await.unwrap();
    let cancel_link = base.links.get("cancelAction").expect("cancelAction link");
    assert!(cancel_link.href.contains(&format!("/cancelAction/{first}")));

    // Reject the cancel → action resumes RUNNING.
    client
        .cancel_feedback(
            &first.to_string(),
            &hawier_client::client::feedback(
                hawier_api::ddi::ExecutionStatus::Rejected,
                hawier_api::ddi::FinalResult::None,
                vec!["busy".into()],
            ),
        )
        .await
        .unwrap();
    let st: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(first)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(st, ActionStatus::Running.code());

    // Cancel it again (new assignment path) and let the device comply.
    let mut tx = server.state.pool.begin().await.unwrap();
    hawier_server::actions::set_action_status(
        &mut tx,
        first,
        ActionStatus::Canceling,
        true,
        "test",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Canceled);

    let first_row = sqlx::query("SELECT status, active FROM sp_action WHERE id = $1")
        .bind(first)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        first_row.get::<i32, _>("status"),
        ActionStatus::Canceled.code()
    );
    assert!(!first_row.get::<bool, _>("active"));

    // Next cycle picks up the second deployment.
    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );
    let second_status: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(second)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(second_status, ActionStatus::Finished.code());
}

#[tokio::test]
#[ignore]
async fn ddi_confirmation_flow_confirm_deny_and_auto_confirm() {
    let server = common::spawn().await;
    common::set_tenant_config(&server, "DEFAULT", "user.confirmation.flow.enabled", "true").await;

    // --- deny then confirm ---
    let target = common::seed_target(&server, "DEFAULT", "dev-confirm").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-conf", b"conf").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let st: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(st, ActionStatus::WaitForConfirmation.code());

    // Deny keeps it gated.
    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            deny_confirmation: true,
            ..Default::default()
        },
    );
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Denied);
    let st: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(st, ActionStatus::WaitForConfirmation.code());

    // Confirm → RUNNING, then the update itself.
    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(dev.cycle().await.unwrap(), CycleOutcome::Confirmed);
    let st: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(st, ActionStatus::Running.code());

    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    // --- auto-confirm skips the gate ---
    let target2 = common::seed_target(&server, "DEFAULT", "dev-autoconf").await;
    let client2 = device(&server, &target2);
    client2
        .activate_auto_confirm(Some("tester".into()))
        .await
        .unwrap();
    let action2 =
        common::assign_ds(&server, "DEFAULT", target2.id, ds.ds_id, ActionType::Forced).await;
    let st: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(action2)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        st,
        ActionStatus::Running.code(),
        "auto-confirmed action starts RUNNING"
    );
}

#[tokio::test]
#[ignore]
async fn ddi_config_data_merge_replace_remove() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-attrs").await;
    let client = device(&server, &target);

    let attrs = |pairs: &[(&str, &str)], mode| hawier_api::ddi::DdiConfigData {
        data: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        mode,
    };

    client
        .put_config_data(&attrs(&[("hw", "rev1"), ("os", "linux")], None))
        .await
        .unwrap();
    client
        .put_config_data(&attrs(
            &[("os", "linux-6.1")],
            Some(hawier_api::ddi::UpdateMode::Merge),
        ))
        .await
        .unwrap();

    let fetch = || async {
        let rows = sqlx::query(
            "SELECT attribute_key, attribute_value FROM sp_target_attributes
             WHERE target = $1 ORDER BY attribute_key",
        )
        .bind(target.id)
        .fetch_all(&server.state.pool)
        .await
        .unwrap();
        rows.iter()
            .map(|r| {
                (
                    r.get::<String, _>("attribute_key"),
                    r.get::<Option<String>, _>("attribute_value")
                        .unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        fetch().await,
        vec![
            ("hw".into(), "rev1".into()),
            ("os".into(), "linux-6.1".into())
        ]
    );

    client
        .put_config_data(&attrs(
            &[("only", "this")],
            Some(hawier_api::ddi::UpdateMode::Replace),
        ))
        .await
        .unwrap();
    assert_eq!(fetch().await, vec![("only".into(), "this".into())]);

    client
        .put_config_data(&attrs(
            &[("only", "")],
            Some(hawier_api::ddi::UpdateMode::Remove),
        ))
        .await
        .unwrap();
    assert!(fetch().await.is_empty());

    // Attributes were delivered → server stops asking for them.
    let flag: bool =
        sqlx::query_scalar("SELECT request_controller_attributes FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert!(!flag);
}

#[tokio::test]
#[ignore]
async fn ddi_artifact_etag_if_match_if_none_match_and_range() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-dl").await;
    let content = b"0123456789abcdef";
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-range", content).await;
    common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let url = format!(
        "{}/DEFAULT/controller/v1/{}/softwaremodules/{}/artifacts/{}",
        server.base_url, target.controller_id, ds.sm_id, ds.filename
    );
    let http = common::http();
    let auth = ("Authorization", format!("TargetToken {}", target.token));

    // Plain download.
    let resp = http.get(&url).header(auth.0, &auth.1).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["etag"].to_str().unwrap(),
        format!("\"{}\"", ds.sha1)
    );
    assert_eq!(resp.headers()["accept-ranges"].to_str().unwrap(), "bytes");
    assert_eq!(resp.bytes().await.unwrap().as_ref(), content);

    // If-Match mismatch → 412.
    let resp = http
        .get(&url)
        .header(auth.0, &auth.1)
        .header("If-Match", "\"deadbeef\"")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 412);

    // If-None-Match matching etag → 304.
    let resp = http
        .get(&url)
        .header(auth.0, &auth.1)
        .header("If-None-Match", format!("\"{}\"", ds.sha1))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 304);

    // Range → 206 with the slice.
    let resp = http
        .get(&url)
        .header(auth.0, &auth.1)
        .header("Range", "bytes=4-7")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(
        resp.headers()["content-range"].to_str().unwrap(),
        format!("bytes 4-7/{}", content.len())
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), &content[4..=7]);

    // Multi-range → full 200 (documented divergence-tolerant behaviour).
    let resp = http
        .get(&url)
        .header(auth.0, &auth.1)
        .header("Range", "bytes=0-1,4-5")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // MD5SUM companion.
    let resp = http
        .get(format!("{url}.MD5SUM"))
        .header(auth.0, &auth.1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body.trim(), format!("{}  {}", ds.md5, ds.filename));

    // Unassigned module → 404 (authorization by deployment).
    let stranger = common::seed_target(&server, "DEFAULT", "dev-stranger").await;
    let resp = http
        .get(url.replace(&target.controller_id, &stranger.controller_id))
        .header("Authorization", format!("TargetToken {}", stranger.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
#[ignore]
async fn ddi_installed_base_get_and_offline_put() {
    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-installed").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-inst", b"inst").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    // Complete the update.
    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );

    // Poll now advertises installedBase; GET returns the finished action.
    let client = device(&server, &target);
    let base = client.poll().await.unwrap();
    let link = base.links.get("installedBase").expect("installedBase link");
    assert!(link.href.contains(&format!("/installedBase/{action_id}")));
    let installed = client.installed_base(&action_id.to_string()).await.unwrap();
    assert_eq!(installed.id, action_id.to_string());
    assert_eq!(installed.deployment.chunks.len(), 1);

    // Offline PUT with another DS.
    let ds2 = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-offline", b"off").await;
    let http = common::http();
    let resp = http
        .put(format!(
            "{}/DEFAULT/controller/v1/{}/installedBase",
            server.base_url, target.controller_id
        ))
        .header("Authorization", format!("TargetToken {}", target.token))
        .json(&serde_json::json!({"name": "fw-offline", "version": "1.0.0"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let installed_ds: Option<i64> =
        sqlx::query_scalar("SELECT installed_distribution_set FROM sp_target WHERE id = $1")
            .bind(target.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(installed_ds, Some(ds2.ds_id));
}

#[tokio::test]
#[ignore]
async fn action_status_quota_blocks_device_spam_not_operator() {
    use hawier_api::ddi::{ExecutionStatus, FinalResult};
    use hawier_client::client::feedback;

    let server = common::spawn().await;
    let target = common::seed_target(&server, "DEFAULT", "dev-quota").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "quota-fw", b"q").await;
    let action_id = common::assign_ds(
        &server,
        "DEFAULT",
        target.id,
        ds.ds_id,
        hawier_api::codes::ActionType::Forced,
    )
    .await;

    // Fill the action up to the 800-status quota (1 RUNNING row exists).
    sqlx::query(
        "INSERT INTO sp_action_status
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, target_occurred_at, status, action)
         SELECT 1, 'spam-seed', 1, 'spam-seed', 1, 'DEFAULT', 1, $1, $2
         FROM generate_series(1, 799)",
    )
    .bind(ActionStatus::Running.code())
    .bind(action_id)
    .execute(&server.state.pool)
    .await
    .unwrap();

    // The spamming device now gets 403 Quota…
    let client = device(&server, &target);
    let base = client.poll().await.unwrap();
    let href = base.links.get("deploymentBase").unwrap().href.clone();
    let err = client
        .post_feedback_by_href(
            &href,
            &feedback(
                ExecutionStatus::Proceeding,
                FinalResult::None,
                vec!["spam".into()],
            ),
        )
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("403"),
        "expected 403 quota, got {err}"
    );

    // …the action is still open…
    let active: Option<bool> = sqlx::query_scalar("SELECT active FROM sp_action WHERE id = $1")
        .bind(action_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(active, Some(true));

    // …and the operator can still force-quit it: the quota guards against
    // device spam, never against the server closing its own actions.
    let response = common::http()
        .delete(format!(
            "{}/rest/v1/targets/{}/actions/{action_id}?force=true",
            server.base_url, target.controller_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "force-quit must not be quota-blocked: {}",
        response.status()
    );
    let (active, status): (Option<bool>, i32) =
        sqlx::query_as("SELECT active, status FROM sp_action WHERE id = $1")
            .bind(action_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(active, Some(false));
    assert_eq!(status, ActionStatus::Canceled.code());
}

/// A device whose connection opens and then goes silent must give up, not
/// hang. Without a read timeout the poll loop stops forever: the device
/// never polls again, and the operator is left with an action that stays
/// "running" while nothing is running. In a 1000-device benchmark 17 froze
/// exactly this way on their first download.
#[tokio::test]
#[ignore]
async fn a_silent_server_does_not_hang_the_device_forever() {
    use hawier_client::client::READ_TIMEOUT;
    use hawier_client::{DdiAuth, DdiClient};

    // Accepts the connection, reads the request, answers nothing, ever.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    let client = DdiClient::new(
        format!("http://{addr}"),
        "DEFAULT",
        "silent-dev".to_string(),
        DdiAuth::TargetToken("token".to_string()),
    );
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(READ_TIMEOUT * 2, client.poll()).await;

    let result = outcome.expect("the client must time out on its own, not hang");
    assert!(result.is_err(), "a silent server is an error, not a poll");
    assert!(
        started.elapsed() >= READ_TIMEOUT,
        "and it must not give up before the read timeout: {:?}",
        started.elapsed()
    );
}

/// A node that is out of capacity answers `503` with `Retry-After`, and that
/// header is its only lever against a fleet. A device that ignores it keeps
/// the overload alive that the header exists to end — so the obedience is
/// worth a test of its own.
#[tokio::test]
#[ignore]
async fn a_device_obeys_retry_after_instead_of_hammering() {
    use axum::http::{header::RETRY_AFTER, StatusCode};
    use hawier_client::{DdiAuth, DdiClient, Device, DeviceOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = axum::Router::new().fallback(axum::routing::any(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(RETRY_AFTER, "2")],
                "out of capacity",
            )
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The device is told to poll every 50ms, which is also the ceiling of
    // its own backoff — so without obeying the header it would come back
    // ~20 times a second, exactly the behaviour that saturates a node.
    let mut device = Device::new(
        DdiClient::new(
            format!("http://{addr}"),
            "DEFAULT",
            "busy-dev".to_string(),
            DdiAuth::TargetToken("token".to_string()),
        ),
        DeviceOptions {
            poll_override: Some(std::time::Duration::from_millis(50)),
            ..DeviceOptions::default()
        },
    );
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), device.run()).await;

    let hits = hits.load(Ordering::Relaxed);
    assert!(
        hits <= 3,
        "three seconds of 'retry in 2s' must cost about two requests, not {hits}"
    );
    assert!(hits >= 1, "the device must still be trying");
}

/// hawkBit asks a device for its attributes when the target was provisioned
/// through the management API, and again after every finished update.
/// SWUpdate's suricatta only sends them when asked, so without either a real
/// device never says what it runs.
#[tokio::test]
#[ignore]
async fn attributes_are_requested_after_provisioning_and_after_an_update() {
    let server = common::spawn().await;
    let created: serde_json::Value = common::http()
        .post(format!("{}/rest/v1/targets", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&serde_json::json!([{ "controllerId": "dev-provisioned" }]))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provisioned = common::SeededTarget {
        id: sqlx::query_scalar("SELECT id FROM sp_target WHERE controller_id = 'dev-provisioned'")
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        controller_id: "dev-provisioned".into(),
        token: created[0]["securityToken"].as_str().unwrap().into(),
    };
    let base = device(&server, &provisioned).poll().await.unwrap();
    assert!(
        base.links.contains_key("configData"),
        "a target provisioned through mgmt should be asked for attributes"
    );

    // A target that has nothing outstanding is asked again once an update
    // finishes.
    let target = common::seed_target(&server, "DEFAULT", "dev-asked-again").await;
    let base = device(&server, &target).poll().await.unwrap();
    assert!(!base.links.contains_key("configData"));
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-asked", b"asked").await;
    common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;
    let mut dev = Device::new(
        device(&server, &target),
        DeviceOptions {
            once: true,
            ..Default::default()
        },
    );
    assert_eq!(
        dev.cycle().await.unwrap(),
        CycleOutcome::UpdatedSuccessfully
    );
    let base = device(&server, &target).poll().await.unwrap();
    assert!(
        base.links.contains_key("configData"),
        "a finished update should ask for attributes again"
    );
}
