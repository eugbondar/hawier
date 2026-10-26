//! Automatic matrix of mixed client<->server cases (completeness of
//! coverage of "different cases" not covered by the targeted suites). `make it`.

use crate::common;

use hawier_api::codes::{ActionStatus, ActionType, RolloutGroupStatus};
use hawier_client::{CycleOutcome, DdiAuth, DdiClient, Device, DeviceOptions};
use serde_json::{json, Value};

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

    async fn send(&self, method: reqwest::Method, path: &str, body: &Value) -> (u16, Value) {
        let resp = self.req(method, path).json(body).send().await.unwrap();
        (
            resp.status().as_u16(),
            resp.json().await.unwrap_or(Value::Null),
        )
    }

    async fn upload_artifact(&self, sm_id: i64, filename: &str, content: Vec<u8>) -> Value {
        let form = reqwest::multipart::Form::new().part(
            "file",
            reqwest::multipart::Part::bytes(content).file_name(filename.to_string()),
        );
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/rest/v1/softwaremodules/{sm_id}/artifacts"),
            )
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "artifact upload failed");
        resp.json().await.unwrap()
    }
}

fn ddi(server: &common::TestServer, target: &common::SeededTarget) -> DdiClient {
    DdiClient::new(
        server.base_url.clone(),
        "DEFAULT",
        target.controller_id.clone(),
        DdiAuth::TargetToken(target.token.clone()),
    )
}

async fn device_once(
    server: &common::TestServer,
    target: &common::SeededTarget,
    options: DeviceOptions,
) -> CycleOutcome {
    let mut device = Device::new(
        ddi(server, target),
        DeviceOptions {
            once: true,
            ..options
        },
    );
    device.cycle().await.unwrap()
}

async fn tick(server: &common::TestServer) {
    hawier_server::rollout::tick(&server.state).await.unwrap();
}

// ---------------------------------------------------------------------------
// 1. Mixed fleet: some succeed, some fail, some stay silent
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn mixed_fleet_rollout_partial_failures() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);

    let mut targets = Vec::new();
    for n in 0..6 {
        targets.push(common::seed_target(&server, "DEFAULT", &format!("mix-{n}")).await);
    }
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "mix-fw", b"mix-payload").await;

    let (status, rollout) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/rollouts",
            &json!({
                "name": "mixed-fleet",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==mix-*",
                "amountGroups": 1,
                "successCondition": { "condition": "THRESHOLD", "expression": "100" },
                "errorCondition": { "condition": "THRESHOLD", "expression": "50" }
            }),
        )
        .await;
    assert_eq!(status, 201);
    let rollout_id = rollout["id"].as_i64().unwrap();

    tick(&server).await; // materialize
    api.send(
        reqwest::Method::POST,
        &format!("/rest/v1/rollouts/{rollout_id}/start"),
        &json!({}),
    )
    .await;
    tick(&server).await; // activate group

    // 3 succeed, 2 fail, the sixth device stays silent.
    for target in &targets[0..3] {
        assert_eq!(
            device_once(&server, target, DeviceOptions::default()).await,
            CycleOutcome::UpdatedSuccessfully
        );
    }
    for target in &targets[3..5] {
        assert_eq!(
            device_once(
                &server,
                target,
                DeviceOptions {
                    fail_download: true,
                    ..Default::default()
                }
            )
            .await,
            CycleOutcome::UpdateFailed
        );
    }

    // 2/6 = 33% errors < 50%: the group keeps working, rollout RUNNING.
    tick(&server).await;
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "running");
    assert_eq!(detail["totalTargetsPerStatus"]["finished"], 3);
    assert_eq!(detail["totalTargetsPerStatus"]["error"], 2);
    assert_eq!(detail["totalTargetsPerStatus"]["running"], 1);

    // The silent one fails too -> 3/6 = 50% >= threshold: group ERROR, rollout PAUSED.
    assert_eq!(
        device_once(
            &server,
            &targets[5],
            DeviceOptions {
                fail_download: true,
                ..Default::default()
            }
        )
        .await,
        CycleOutcome::UpdateFailed
    );
    tick(&server).await;
    let detail = api.get(&format!("/rest/v1/rollouts/{rollout_id}")).await;
    assert_eq!(detail["status"], "paused");
    let group_status: i32 =
        sqlx::query_scalar("SELECT status FROM sp_rollout_group WHERE rollout = $1")
            .bind(rollout_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(group_status, RolloutGroupStatus::Error.code());
}

// ---------------------------------------------------------------------------
// 2. DS of two modules with several artifacts (+legacy part names)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn multi_module_ds_with_multiple_artifacts() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let target = common::seed_target(&server, "DEFAULT", "multi-dev").await;

    // os module + application module, two artifacts in each.
    let (_, sms) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/softwaremodules",
            &json!([
                { "name": "core", "version": "2.0", "type": "os" },
                { "name": "apps", "version": "2.0", "type": "application" }
            ]),
        )
        .await;
    let os_sm = sms[0]["id"].as_i64().unwrap();
    let app_sm = sms[1]["id"].as_i64().unwrap();
    for (sm, name) in [
        (os_sm, "kernel.img"),
        (os_sm, "rootfs.img"),
        (app_sm, "app-a.bin"),
        (app_sm, "app-b.bin"),
    ] {
        api.upload_artifact(sm, name, format!("payload-{name}").into_bytes())
            .await;
    }

    let (status, ds) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/distributionsets",
            &json!([{ "name": "multi-bundle", "version": "2.0", "type": "os_app",
                      "modules": [{ "id": os_sm }, { "id": app_sm }] }]),
        )
        .await;
    assert_eq!(status, 201);
    assert_eq!(ds[0]["complete"], json!(true));
    let ds_id = ds[0]["id"].as_i64().unwrap();

    common::assign_ds(&server, "DEFAULT", target.id, ds_id, ActionType::Forced).await;

    // deploymentBase: two chunks, the application one has legacy part name = bApp.
    let client = ddi(&server, &target);
    let base = client.poll().await.unwrap();
    let dep = client
        .deployment_base_by_href(&base.links["deploymentBase"].href)
        .await
        .unwrap();
    let mut parts: Vec<&str> = dep
        .deployment
        .chunks
        .iter()
        .map(|c| c.part.as_str())
        .collect();
    parts.sort_unstable();
    assert_eq!(parts, ["bApp", "os"]);
    let total_artifacts: usize = dep
        .deployment
        .chunks
        .iter()
        .map(|c| c.artifacts.len())
        .sum();
    assert_eq!(total_artifacts, 4);

    // The device downloads all 4, checks the hashes and finishes the update.
    assert_eq!(
        device_once(&server, &target, DeviceOptions::default()).await,
        CycleOutcome::UpdatedSuccessfully
    );
    let installed = api.get("/rest/v1/targets/multi-dev/installedDS").await;
    assert_eq!(installed["id"], json!(ds_id));
}

// ---------------------------------------------------------------------------
// 3. TIMEFORCED: attempt before the deadline, forced after it
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn timeforced_switches_to_forced_after_deadline() {
    let server = common::spawn().await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "tf-fw", b"tf").await;
    let now = hawier_server::state::now_millis();

    for (suffix, forced_time, expected) in [
        ("past", now - 60_000, hawier_api::ddi::HandlingType::Forced),
        (
            "future",
            now + 3_600_000,
            hawier_api::ddi::HandlingType::Attempt,
        ),
    ] {
        let target = common::seed_target(&server, "DEFAULT", &format!("tf-dev-{suffix}")).await;
        hawier_server::deploy_logic::assign(
            &server.state.pool,
            &server.state.incidents,
            "DEFAULT",
            &hawier_server::deploy_logic::AssignRequest {
                target_id: target.id,
                ds_id: ds.ds_id,
                action_type: ActionType::TimeForced,
                forced_time: Some(forced_time),
                weight: hawier_server::deploy_logic::DEFAULT_WEIGHT,
                initiated_by: "test".into(),
                confirmation_required: None,
                maintenance_window: None,
            },
        )
        .await
        .unwrap();

        let client = ddi(&server, &target);
        let base = client.poll().await.unwrap();
        let dep = client
            .deployment_base_by_href(&base.links["deploymentBase"].href)
            .await
            .unwrap();
        assert_eq!(dep.deployment.update, expected, "case {suffix}");
    }
}

// ---------------------------------------------------------------------------
// 4. Cancel "in flight": the device has already picked up the deployment
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn cancel_mid_flight_device_complies() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let target = common::seed_target(&server, "DEFAULT", "cancel-dev").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "cf-fw", b"cf").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    // The device "saw" the deployment (RETRIEVED) but hasn't reported yet.
    let client = ddi(&server, &target);
    let base = client.poll().await.unwrap();
    client
        .deployment_base_by_href(&base.links["deploymentBase"].href)
        .await
        .unwrap();

    // The operator cancels the action.
    let resp = api
        .req(
            reqwest::Method::DELETE,
            &format!("/rest/v1/targets/cancel-dev/actions/{action_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // The device's next cycle obeys the cancel.
    assert_eq!(
        device_once(&server, &target, DeviceOptions::default()).await,
        CycleOutcome::Canceled
    );

    let action = api
        .get(&format!("/rest/v1/targets/cancel-dev/actions/{action_id}"))
        .await;
    assert_eq!(action["status"], "finished");
    assert_eq!(action["detailStatus"], "canceled");
    assert_eq!(action["type"], "cancel");
    // The link to the set carries its name and id, not just an href: the
    // console has no other source for them, and printed an empty
    // "Distribution set" row on every action page until it did.
    let link = &action["_links"]["distributionset"];
    assert!(
        link["name"].as_str().is_some_and(|name| name.contains(':')),
        "name:version in the link: {link}"
    );
    assert!(link["id"].is_i64(), "id in the link: {link}");

    // Status chain: RUNNING -> RETRIEVED -> CANCELING -> CANCELED.
    let statuses = common::action_statuses(&server, action_id).await;
    assert_eq!(
        statuses,
        vec![
            ActionStatus::Running.code(),
            ActionStatus::Retrieved.code(),
            ActionStatus::Canceling.code(),
            ActionStatus::Canceled.code(),
        ]
    );
}

// ---------------------------------------------------------------------------
// 5. Attributes: replace/merge, filtering and auto-assign by attribute
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn attributes_roundtrip_and_attribute_auto_assign() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let target = common::seed_target(&server, "DEFAULT", "attr-dev").await;
    let bystander = common::seed_target(&server, "DEFAULT", "attr-other").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "attr-fw", b"attr").await;

    let client = ddi(&server, &target);
    let put = |pairs: Vec<(&str, &str)>, mode| {
        let data = pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        hawier_api::ddi::DdiConfigData { data, mode }
    };
    client
        .put_config_data(&put(
            vec![("hw", "rev1"), ("region", "eu")],
            Some(hawier_api::ddi::UpdateMode::Replace),
        ))
        .await
        .unwrap();
    client
        .put_config_data(&put(
            vec![("hw", "rev2")],
            Some(hawier_api::ddi::UpdateMode::Merge),
        ))
        .await
        .unwrap();

    // Attribute map via mgmt.
    let attributes = api.get("/rest/v1/targets/attr-dev/attributes").await;
    assert_eq!(attributes["hw"], "rev2");
    assert_eq!(attributes["region"], "eu");

    // RSQL on the attribute finds exactly this device.
    let by_attr = api.get("/rest/v1/targets?q=attribute.hw==rev2").await;
    assert_eq!(by_attr["total"], 1);
    assert_eq!(by_attr["content"][0]["controllerId"], "attr-dev");

    // Auto-assign by the attribute filter assigns the DS to it only.
    let (status, filter) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targetfilters",
            &json!({ "name": "rev2-fleet", "query": "attribute.hw==rev2" }),
        )
        .await;
    assert_eq!(status, 201);
    let filter_id = filter["id"].as_i64().unwrap();
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/targetfilters/{filter_id}/autoAssignDS"),
            &json!({ "id": ds.ds_id }),
        )
        .await;
    assert_eq!(status, 200);
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
    let bystander_assigned: Option<i64> =
        sqlx::query_scalar("SELECT assigned_distribution_set FROM sp_target WHERE id = $1")
            .bind(bystander.id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(bystander_assigned, None);

    // And the device carries the auto-assigned update through to the end.
    assert_eq!(
        device_once(&server, &target, DeviceOptions::default()).await,
        CycleOutcome::UpdatedSuccessfully
    );
}

// ---------------------------------------------------------------------------
// 6. Offline installedBase report is visible in mgmt
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn offline_installed_base_visible_in_mgmt() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let target = common::seed_target(&server, "DEFAULT", "offline-dev").await;
    common::seed_ds_with_artifact(&server, "DEFAULT", "preinstalled", b"pre").await;

    let resp = common::http()
        .put(format!(
            "{}/DEFAULT/controller/v1/{}/installedBase",
            server.base_url, target.controller_id
        ))
        .header("Authorization", format!("TargetToken {}", target.token))
        .json(&json!({ "name": "preinstalled", "version": "1.0.0" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // mgmt sees the installed DS and the closed offline action.
    let installed = api.get("/rest/v1/targets/offline-dev/installedDS").await;
    assert_eq!(installed["name"], "preinstalled");
    let target_json = api.get("/rest/v1/targets/offline-dev").await;
    assert_eq!(target_json["updateStatus"], "in_sync");
    let actions = api.get("/rest/v1/targets/offline-dev/actions").await;
    assert_eq!(actions["total"], 1);
    assert_eq!(actions["content"][0]["status"], "finished");
    assert_eq!(actions["content"][0]["detailStatus"], "finished");
}

// ---------------------------------------------------------------------------
// 7. Device "logs" (details from feedback) are readable through mgmt
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn device_report_messages_visible_in_mgmt() {
    let server = common::spawn().await;
    let api = Api::new(&server.base_url);
    let target = common::seed_target(&server, "DEFAULT", "log-dev").await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "log-fw", b"log-payload").await;
    let action_id =
        common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    assert_eq!(
        device_once(&server, &target, DeviceOptions::default()).await,
        CycleOutcome::UpdatedSuccessfully
    );

    // Full history: newest first, the device's messages in place.
    let history = api
        .get(&format!(
            "/rest/v1/targets/log-dev/actions/{action_id}/status"
        ))
        .await;
    let entries = history["content"].as_array().unwrap();
    assert_eq!(entries[0]["type"], "finished", "newest entry first");
    let all_messages: Vec<String> = entries
        .iter()
        .flat_map(|e| e["messages"].as_array().unwrap().iter())
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    assert!(
        all_messages.iter().any(|m| m.contains("hashes verified")),
        "device download report must be present: {all_messages:?}"
    );
    assert!(all_messages
        .iter()
        .any(|m| m.contains("Simulated update applied successfully")));

    // History pagination.
    let page = api
        .get(&format!(
            "/rest/v1/targets/log-dev/actions/{action_id}/status?limit=2&offset=0"
        ))
        .await;
    assert_eq!(page["size"], 2);
    assert!(page["total"].as_i64().unwrap() >= 5);
}
