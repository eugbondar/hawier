//! Mgmt API integration matrix (Phase 3). Run via `make it`.

use crate::common;

use serde_json::{json, Value};
use sqlx::Row;

struct Api {
    http: reqwest::Client,
    base: String,
}

impl Api {
    fn new(server: &common::TestServer) -> Self {
        Self {
            http: common::http(),
            base: server.base_url.clone(),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .header("Authorization", common::basic("admin", "admin"))
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let resp = self.req(reqwest::Method::GET, path).send().await.unwrap();
        let status = resp.status().as_u16();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: &Value) -> (u16, Value) {
        let resp = self.req(method, path).json(body).send().await.unwrap();
        let status = resp.status().as_u16();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }
}

#[tokio::test]
#[ignore]
async fn mgmt_target_crud_array_create_rsql_pagination() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    // Array create (hawier-ui depends on this shape).
    let (status, created) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets",
            &json!([
                { "controllerId": "t-alpha", "name": "Alpha" },
                { "controllerId": "t-beta" },
                { "controllerId": "x-gamma", "description": "third" }
            ]),
        )
        .await;
    assert_eq!(status, 201);
    let created = created.as_array().unwrap();
    assert_eq!(created.len(), 3);
    assert_eq!(created[0]["controllerId"], "t-alpha");
    assert_eq!(
        created[1]["name"], "t-beta",
        "name defaults to controllerId"
    );
    assert!(created[0]["securityToken"].as_str().unwrap().len() == 32);

    // Duplicate → 409.
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets",
            &json!([{ "controllerId": "t-alpha" }]),
        )
        .await;
    assert_eq!(status, 409);

    // RSQL wildcard + pagination.
    let (status, page) = api
        .get("/rest/v1/targets?q=controllerid==t-*&limit=1&offset=0")
        .await;
    assert_eq!(status, 200);
    assert_eq!(page["total"], 2);
    assert_eq!(page["size"], 1);
    assert_eq!(page["content"].as_array().unwrap().len(), 1);

    // limit > 500 → 400.
    let (status, _) = api.get("/rest/v1/targets?limit=501").await;
    assert_eq!(status, 400);

    // Invalid RSQL → 400 with hawkbit-style error body.
    let (status, error) = api.get("/rest/v1/targets?q=bogusfield==1").await;
    assert_eq!(status, 400);
    assert!(error["errorCode"].as_str().unwrap().contains("rsql"));

    // Update + single GET.
    let (status, updated) = api
        .send(
            reqwest::Method::PUT,
            "/rest/v1/targets/t-alpha",
            &json!({ "name": "Alpha2", "group": "lab/rack1" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(updated["name"], "Alpha2");
    assert_eq!(updated["group"], "lab/rack1");

    // Sort DESC by name.
    let (_, sorted) = api.get("/rest/v1/targets?sort=name:DESC").await;
    let names: Vec<&str> = sorted["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut expected = names.clone();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(names, expected);

    // Delete.
    let resp = api
        .req(reqwest::Method::DELETE, "/rest/v1/targets/x-gamma")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let (status, _) = api.get("/rest/v1/targets/x-gamma").await;
    assert_eq!(status, 404);
}

#[tokio::test]
#[ignore]
async fn mgmt_sm_multipart_upload_hash_verify_and_download() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    let (status, sm) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/softwaremodules",
            &json!([{ "name": "fw", "version": "2.0.0", "type": "os", "vendor": "acme" }]),
        )
        .await;
    assert_eq!(status, 201);
    let sm_id = sm[0]["id"].as_i64().unwrap();

    let content = b"artifact-bytes-123".to_vec();
    let sha1 = {
        use sha1::Digest;
        hex::encode(sha1::Sha1::digest(&content))
    };

    // Upload with a correct sha1sum param.
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(content.clone()).file_name("fw.bin"),
    );
    let resp = api
        .req(
            reqwest::Method::POST,
            &format!("/rest/v1/softwaremodules/{sm_id}/artifacts?sha1sum={sha1}"),
        )
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let artifact: Value = resp.json().await.unwrap();
    assert_eq!(artifact["hashes"]["sha1"], json!(sha1));
    assert_eq!(artifact["size"], json!(content.len()));
    assert_eq!(artifact["providedFilename"], "fw.bin");
    let artifact_id = artifact["id"].as_i64().unwrap();

    // Upload with a wrong hash param → 400.
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(b"other".to_vec()).file_name("bad.bin"),
    );
    let resp = api
        .req(
            reqwest::Method::POST,
            &format!("/rest/v1/softwaremodules/{sm_id}/artifacts?md5sum=deadbeef"),
        )
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Download roundtrip.
    let resp = api
        .req(
            reqwest::Method::GET,
            &format!("/rest/v1/softwaremodules/{sm_id}/artifacts/{artifact_id}/download"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("accept-ranges")
            .map(|v| v.to_str().unwrap()),
        Some("bytes"),
        "the endpoint has to advertise that it can resume"
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), content.as_slice());

    // A resumed download: a multi-gigabyte image over a flaky link must not
    // start from zero because the connection dropped.
    let resp = api
        .req(
            reqwest::Method::GET,
            &format!("/rest/v1/softwaremodules/{sm_id}/artifacts/{artifact_id}/download"),
        )
        .header("Range", "bytes=4-8")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206, "a ranged request must not answer 200");
    assert_eq!(
        resp.headers()
            .get("content-range")
            .map(|v| v.to_str().unwrap()),
        Some(format!("bytes 4-8/{}", content.len()).as_str())
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), &content[4..=8]);

    // Names the server could not serve back everywhere are refused before a
    // byte is stored: non-ASCII (DDI answered 500 for it) and a slash (DDI
    // addresses the artifact by its name as one path segment).
    for name in ["fw-\u{fc}.bin", "a/b.bin", "   "] {
        let form = reqwest::multipart::Form::new().part(
            "file",
            reqwest::multipart::Part::bytes(b"refused".to_vec()).file_name(name),
        );
        let resp = api
            .req(
                reqwest::Method::POST,
                &format!("/rest/v1/softwaremodules/{sm_id}/artifacts"),
            )
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{name:?} must be refused");
        let body: Value = resp.json().await.unwrap();
        assert_eq!(
            body["errorCode"], "hawkbit.server.error.repo.invalidParameter",
            "{name:?}"
        );
    }
    let (status, artifacts) = api
        .send(
            reqwest::Method::GET,
            &format!("/rest/v1/softwaremodules/{sm_id}/artifacts"),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        artifacts.as_array().map(Vec::len),
        Some(1),
        "a refused upload must store nothing: {artifacts}"
    );
}

#[tokio::test]
#[ignore]
async fn mgmt_ds_create_assign_sm_assign_target_roundtrip() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    let (_, sm) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/softwaremodules",
            &json!([{ "name": "core", "version": "1.0", "type": "os" }]),
        )
        .await;
    let sm_id = sm[0]["id"].as_i64().unwrap();

    let (status, ds) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/distributionsets",
            &json!([{ "name": "bundle", "version": "1.0", "type": "os",
                      "modules": [{ "id": sm_id }] }]),
        )
        .await;
    assert_eq!(status, 201);
    let ds_id = ds[0]["id"].as_i64().unwrap();
    assert_eq!(
        ds[0]["complete"],
        json!(true),
        "os DS with os module is complete"
    );
    assert_eq!(ds[0]["modules"][0]["id"], json!(sm_id));

    // Target + assignment.
    api.send(
        reqwest::Method::POST,
        "/rest/v1/targets",
        &json!([{ "controllerId": "ds-target" }]),
    )
    .await;
    let (status, result) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/ds-target/assignedDS",
            &json!({ "id": ds_id }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(result["assigned"], 1);
    assert_eq!(result["total"], 1);

    // Re-assign → alreadyAssigned.
    let (_, result) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/ds-target/assignedDS",
            &json!({ "id": ds_id }),
        )
        .await;
    assert_eq!(result["alreadyAssigned"], 1);
    assert_eq!(result["assigned"], 0);

    // Visible in assignedTargets + target's assignedDS.
    let (_, assigned) = api
        .get(&format!(
            "/rest/v1/distributionsets/{ds_id}/assignedTargets"
        ))
        .await;
    assert_eq!(assigned["total"], 1);
    assert_eq!(assigned["content"][0]["controllerId"], "ds-target");

    let (status, ds_of_target) = api.get("/rest/v1/targets/ds-target/assignedDS").await;
    assert_eq!(status, 200);
    assert_eq!(ds_of_target["id"], json!(ds_id));

    // Target action list shows the running update.
    let (_, target_actions) = api.get("/rest/v1/targets/ds-target/actions").await;
    assert_eq!(target_actions["total"], 1);
    assert_eq!(target_actions["content"][0]["status"], "pending");
    assert_eq!(target_actions["content"][0]["detailStatus"], "running");

    // Global actions list too.
    let (_, actions) = api.get("/rest/v1/actions?q=detailstatus==running").await;
    assert_eq!(actions["total"], 1);
}

#[tokio::test]
#[ignore]
async fn mgmt_tags_types_crud_and_assignment() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    api.send(
        reqwest::Method::POST,
        "/rest/v1/targets",
        &json!([{ "controllerId": "tagged-1" }, { "controllerId": "tagged-2" }]),
    )
    .await;

    // Tag CRUD (array create).
    let (status, tags) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targettags",
            &json!([{ "name": "eu-fleet", "colour": "#00ff00" }]),
        )
        .await;
    assert_eq!(status, 201);
    let tag_id = tags[0]["id"].as_i64().unwrap();

    // Assign both targets, then verify.
    let resp = api
        .req(
            reqwest::Method::POST,
            &format!("/rest/v1/targettags/{tag_id}/assigned/tagged-1"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let (assign_status, assigned) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/targettags/{tag_id}/assigned"),
            &json!(["tagged-2"]),
        )
        .await;
    assert_eq!(assign_status, 200, "batch tag assign failed: {assigned}");
    let (_, listing) = api
        .get(&format!("/rest/v1/targettags/{tag_id}/assigned"))
        .await;
    assert_eq!(listing["total"], 2);

    // Targets filterable by tag.
    let (_, by_tag) = api.get("/rest/v1/targets?q=tag==eu-fleet").await;
    assert_eq!(by_tag["total"], 2);

    // Target type + compatible DS types.
    let (status, types) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targettypes",
            &json!([{ "name": "camera", "key": "cam" }]),
        )
        .await;
    assert_eq!(status, 201);
    let type_id = types[0]["id"].as_i64().unwrap();

    let (_, ds_types) = api.get("/rest/v1/distributionsettypes").await;
    let os_type_id = ds_types["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["key"] == "os")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let resp = api
        .req(
            reqwest::Method::POST,
            &format!("/rest/v1/targettypes/{type_id}/compatibledistributionsettypes"),
        )
        .json(&json!([{ "id": os_type_id }]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let (_, compatible) = api
        .get(&format!(
            "/rest/v1/targettypes/{type_id}/compatibledistributionsettypes"
        ))
        .await;
    assert_eq!(compatible.as_array().unwrap().len(), 1);

    // Assign type to target via mgmt.
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/tagged-1/targettype",
            &json!({ "id": type_id }),
        )
        .await;
    assert_eq!(status, 200);
    let (_, target) = api.get("/rest/v1/targets/tagged-1").await;
    assert_eq!(target["targetType"], json!(type_id));
    assert_eq!(target["targetTypeName"], "camera");
}

/// Seed a target that last spoke `silent_ms` ago, and hand back its id.
async fn target_silent_for(server: &common::TestServer, name: &str, silent_ms: Option<i64>) {
    let last: Option<i64> = match silent_ms {
        Some(ms) => {
            let now: i64 = sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT")
                .fetch_one(&server.state.pool)
                .await
                .unwrap();
            Some(now - ms)
        }
        None => None,
    };
    sqlx::query(
        "INSERT INTO sp_target
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, controller_id, sec_token, request_controller_attributes,
          update_status, last_target_query)
         VALUES (1, 'it', 1, 'it', 1, 'DEFAULT', $1, $1, $2, FALSE, 1, $3)",
    )
    .bind(name)
    .bind(format!("token-{name}"))
    .bind(last)
    .execute(&server.state.pool)
    .await
    .unwrap();
}

/// What the list says about the fixture target.
async fn list_poll_state(api: &Api) -> String {
    let (status, page) = api.get("/rest/v1/targets?q=controllerId==poll-7min").await;
    assert_eq!(status, 200, "{page}");
    page["content"][0]["pollState"]
        .as_str()
        .expect("pollState is a compact key")
        .to_string()
}

/// What the card says about the same target. `None` means it never polled,
/// which is when hawkBit omits `pollStatus` altogether.
async fn card_overdue(api: &Api) -> Option<bool> {
    let (status, target) = api.get("/rest/v1/targets/poll-7min").await;
    assert_eq!(status, 200, "{target}");
    target["pollStatus"]["overdue"].as_bool()
}

/// Liveness follows the tenant's polling configuration — both halves of it.
///
/// The middle step is the regression pin: on the code this replaced,
/// `pollingOverdueTime` was declared, defaulted and served over
/// `/rest/v1/system/configs` while nothing read it, so a device seven minutes
/// silent under a one-minute poll time was overdue no matter how much grace
/// the tenant had configured. That step fails on the old formula.
#[tokio::test]
#[ignore]
async fn poll_state_follows_the_tenant_polling_configuration() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    target_silent_for(&server, "poll-7min", Some(7 * 60_000)).await;

    // The page and the predicate must mean the same thing by "unset".
    let (_, configs) = api.get("/rest/v1/system/configs").await;
    assert_eq!(configs["pollingTime"]["value"], "00:05:00");
    assert_eq!(configs["pollingOverdueTime"]["value"], "00:05:00");

    // Default window is 5 + 5 = ten minutes, so seven is still fine.
    assert_eq!(list_poll_state(&api).await, "responding");
    assert_eq!(card_overdue(&api).await, Some(false));

    // A short poll time with a long grace: eleven minutes of window.
    for (key, value) in [
        ("pollingTime", "00:01:00"),
        ("pollingOverdueTime", "00:10:00"),
    ] {
        let (status, _) = api
            .send(
                reqwest::Method::PUT,
                &format!("/rest/v1/system/configs/{key}"),
                &json!({ "value": value }),
            )
            .await;
        assert_eq!(status, 200);
    }
    assert_eq!(
        list_poll_state(&api).await,
        "responding",
        "pollingOverdueTime is part of the window, so seven minutes of silence \
         is inside an eleven-minute one"
    );
    assert_eq!(card_overdue(&api).await, Some(false));

    // Now shrink the grace: two minutes of window, seven of silence.
    let (status, _) = api
        .send(
            reqwest::Method::PUT,
            "/rest/v1/system/configs/pollingOverdueTime",
            &json!({ "value": "00:01:00" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(list_poll_state(&api).await, "overdue");
    assert_eq!(
        card_overdue(&api).await,
        Some(true),
        "the card reads the same column the list filtered on"
    );

    // nextExpectedRequestAt is pollingTime alone, as in hawkBit — so it is
    // deliberately not the same instant as the overdue threshold.
    let (_, target) = api.get("/rest/v1/targets/poll-7min").await;
    let last = target["pollStatus"]["lastRequestAt"].as_i64().unwrap();
    let next = target["pollStatus"]["nextExpectedRequestAt"]
        .as_i64()
        .unwrap();
    assert_eq!(
        next - last,
        60_000,
        "nextExpectedRequestAt = last + pollingTime"
    );
}

async fn total_of(api: &Api, path: &str) -> i64 {
    let (status, page) = api.get(path).await;
    assert_eq!(status, 200, "{page}");
    page["total"].as_i64().expect("a list answers with a total")
}

/// The list filter and the target card cannot disagree, and the two views
/// partition the fleet exactly (UI/UX rule 6).
#[tokio::test]
#[ignore]
async fn list_filter_and_target_card_agree_on_liveness() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    target_silent_for(&server, "live-now", Some(1_000)).await;
    target_silent_for(&server, "silent-hour", Some(3_600_000)).await;
    target_silent_for(&server, "never-spoke", None).await;

    let responding = total_of(&api, "/rest/v1/targets?q=pollState==responding").await;
    let not_responding = total_of(&api, "/rest/v1/targets?q=pollState!=responding").await;
    assert_eq!(
        responding + not_responding,
        total_of(&api, "/rest/v1/targets").await,
        "the expression is total, so the two are exact complements"
    );
    assert_eq!(responding, 1);
    assert_eq!(not_responding, 2);

    // Every row the list calls not-responding must say so on its own card.
    let (_, page) = api.get("/rest/v1/targets?q=pollState!=responding").await;
    for row in page["content"].as_array().unwrap() {
        let id = row["controllerId"].as_str().unwrap();
        let (_, card) = api.get(&format!("/rest/v1/targets/{id}")).await;
        match card["pollStatus"].as_object() {
            Some(status) => assert_eq!(
                status["overdue"], true,
                "{id}: the list filtered it out but the card calls it healthy"
            ),
            // Never polled: hawkBit omits pollStatus entirely, and so do we.
            None => assert_eq!(row["pollState"], "never"),
        }
    }

    let (_, page) = api.get("/rest/v1/targets?q=pollState==responding").await;
    for row in page["content"].as_array().unwrap() {
        let id = row["controllerId"].as_str().unwrap();
        let (_, card) = api.get(&format!("/rest/v1/targets/{id}")).await;
        assert_eq!(card["pollStatus"]["overdue"], false, "{id}");
    }
}

/// A typo in a settings field must not take the fleet view down.
///
/// `PUT /rest/v1/system/configs` checks that a value is a scalar, not that it
/// is a duration, so these are values an operator can really store.
#[tokio::test]
#[ignore]
async fn a_malformed_polling_time_does_not_take_the_target_list_down() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    target_silent_for(&server, "poll-12min", Some(12 * 60_000)).await;
    target_silent_for(&server, "poll-3min", Some(3 * 60_000)).await;

    for bad in [json!("5 minutes"), json!(300), json!("")] {
        let (status, _) = api
            .send(
                reqwest::Method::PUT,
                "/rest/v1/system/configs/pollingTime",
                &json!({ "value": bad }),
            )
            .await;
        assert_eq!(status, 200, "the config endpoint accepts it: {bad}");

        let (status, page) = api.get("/rest/v1/targets?q=pollState==overdue").await;
        assert_eq!(
            status, 200,
            "an unusable {bad} must not 500 the list: {page}"
        );
        let overdue: Vec<&str> = page["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["controllerId"].as_str().unwrap())
            .collect();
        assert_eq!(
            overdue,
            ["poll-12min"],
            "an unusable {bad} falls back to the documented default window"
        );
    }
}

/// The field is text, so an unknown state matches nothing rather than
/// answering 400. Pinned so it is a decision, not an accident.
#[tokio::test]
#[ignore]
async fn poll_state_rejects_nothing_but_matches_nothing() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    target_silent_for(&server, "poll-any", Some(1_000)).await;

    let (status, page) = api.get("/rest/v1/targets?q=pollState==bogus").await;
    assert_eq!(status, 200);
    assert_eq!(page["total"], 0);

    // But it is a real column: sorting by it is allowed (UI/UX rule 25).
    let (status, _) = api.get("/rest/v1/targets?sort=pollState:ASC").await;
    assert_eq!(status, 200);
}

#[tokio::test]
#[ignore]
async fn mgmt_system_configs_single_and_batch() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    // Defaults are served with global=true.
    let (status, all) = api.get("/rest/v1/system/configs").await;
    assert_eq!(status, 200);
    assert_eq!(all["pollingTime"]["value"], "00:05:00");
    assert_eq!(all["pollingTime"]["global"], true);

    // Single PUT.
    let (status, updated) = api
        .send(
            reqwest::Method::PUT,
            "/rest/v1/system/configs/pollingTime",
            &json!({ "value": "00:01:00" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(updated["value"], "00:01:00");
    let (_, one) = api.get("/rest/v1/system/configs/pollingTime").await;
    assert_eq!(one["value"], "00:01:00");
    assert_eq!(one["global"], false);

    // Batch PUT with typed values.
    let (status, _) = api
        .send(
            reqwest::Method::PUT,
            "/rest/v1/system/configs",
            &json!({
                "authentication.gatewaytoken.enabled": { "value": true },
                "authentication.gatewaytoken.key": { "value": "gw-key" }
            }),
        )
        .await;
    assert_eq!(status, 200);
    let (_, all) = api.get("/rest/v1/system/configs").await;
    assert_eq!(all["authentication.gatewaytoken.enabled"]["value"], true);

    // DELETE resets to default.
    let resp = api
        .req(
            reqwest::Method::DELETE,
            "/rest/v1/system/configs/pollingTime",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let (_, one) = api.get("/rest/v1/system/configs/pollingTime").await;
    assert_eq!(one["global"], true);
}

#[tokio::test]
#[ignore]
async fn mgmt_target_filters_and_metadata() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    // Bare-object create (no array here).
    let (status, filter) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targetfilters",
            &json!({ "name": "all-lab", "query": "controllerid==lab-*" }),
        )
        .await;
    assert_eq!(status, 201);
    let filter_id = filter["id"].as_i64().unwrap();
    assert_eq!(filter["query"], "controllerid==lab-*");

    // Broken RSQL rejected on create.
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targetfilters",
            &json!({ "name": "broken", "query": "nosuchfield==1" }),
        )
        .await;
    assert_eq!(status, 400);

    // Target metadata CRUD.
    api.send(
        reqwest::Method::POST,
        "/rest/v1/targets",
        &json!([{ "controllerId": "meta-target" }]),
    )
    .await;
    let (status, created) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/meta-target/metadata",
            &json!([{ "key": "site", "value": "berlin" }]),
        )
        .await;
    assert_eq!(status, 201);
    assert_eq!(created[0]["key"], "site");
    let (_, one) = api.get("/rest/v1/targets/meta-target/metadata/site").await;
    assert_eq!(one["value"], "berlin");

    // Filter targets by metadata.
    let (_, by_meta) = api.get("/rest/v1/targets?q=metadata.site==berlin").await;
    assert_eq!(by_meta["total"], 1);

    let _ = filter_id;

    // Row landed with the right shape in the shared DB.
    let count: i64 =
        sqlx::query("SELECT COUNT(*) AS c FROM sp_target_filter_query WHERE tenant = 'DEFAULT'")
            .fetch_one(&server.state.pool)
            .await
            .unwrap()
            .get("c");
    assert_eq!(count, 1);
}

/// Every column the UI shows must be sortable server-side (B7 parity).
#[tokio::test]
#[ignore]
async fn sort_parity_with_ui_columns() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    let matrix: &[(&str, &[&str])] = &[
        (
            "targets",
            &[
                "controllerId",
                "name",
                "updateStatus",
                "assignedDistributionSet",
                "lastTargetQuery",
                "createdAt",
            ],
        ),
        (
            "rollouts",
            &[
                "id",
                "name",
                "status",
                "totalTargets",
                "distributionSet",
                "createdAt",
            ],
        ),
        (
            "distributionsets",
            &["id", "name", "version", "type", "complete", "createdAt"],
        ),
        (
            "targetfilters",
            &[
                "id",
                "name",
                "query",
                "autoAssignDistributionSet",
                "createdAt",
            ],
        ),
        (
            "softwaremodules",
            &["id", "name", "version", "type", "vendor", "createdAt"],
        ),
    ];

    for (endpoint, columns) in matrix {
        for column in *columns {
            for direction in ["ASC", "DESC"] {
                let (status, body) = api
                    .get(&format!(
                        "/rest/v1/{endpoint}?sort={column}:{direction}&limit=1"
                    ))
                    .await;
                assert_eq!(
                    status, 200,
                    "sort {endpoint} by {column}:{direction} failed: {body}"
                );
            }
        }
    }
}

/// Everything the console's fifth wave leans on: paged metadata, the
/// auto-assign DS name, the group resource, and sortable columns.
#[tokio::test]
#[ignore]
async fn mgmt_console_contract() {
    let server = common::spawn().await;
    let api = Api::new(&server);

    // --- metadata pages -----------------------------------------------
    api.send(
        reqwest::Method::POST,
        "/rest/v1/targets",
        &json!([{ "controllerId": "meta-paged" }]),
    )
    .await;
    let entries: Vec<Value> = (0..5)
        .map(|n| json!({ "key": format!("k{n}"), "value": format!("v{n}") }))
        .collect();
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/meta-paged/metadata",
            &json!(entries),
        )
        .await;
    assert_eq!(status, 201);
    let (_, page) = api
        .get("/rest/v1/targets/meta-paged/metadata?limit=2&offset=2")
        .await;
    assert_eq!(page["total"], 5, "total counts every row, not the page");
    assert_eq!(page["content"].as_array().unwrap().len(), 2);
    assert_eq!(
        page["content"][0]["key"], "k2",
        "ordered by key, offset honoured"
    );

    // --- targetVisible survives a PUT that omits it --------------------
    let (_, sm) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/softwaremodules",
            &json!([{ "name": "meta-sm", "version": "1.0", "type": "os" }]),
        )
        .await;
    let sm_id = sm[0]["id"].as_i64().unwrap();
    api.send(
        reqwest::Method::POST,
        &format!("/rest/v1/softwaremodules/{sm_id}/metadata"),
        &json!([{ "key": "visible", "value": "yes", "targetVisible": true }]),
    )
    .await;
    let (_, updated) = api
        .send(
            reqwest::Method::PUT,
            &format!("/rest/v1/softwaremodules/{sm_id}/metadata/visible"),
            &json!({ "value": "still yes" }),
        )
        .await;
    assert_eq!(
        updated["targetVisible"], true,
        "PUT keeps the stored flag, so the answer must report the stored flag"
    );

    // --- auto-assign DS carries a name --------------------------------
    let (_, ds) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/distributionsets",
            &json!([{ "name": "auto-ds", "version": "2.0", "type": "os" }]),
        )
        .await;
    let ds_id = ds[0]["id"].as_i64().unwrap();
    let (_, filter) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targetfilters",
            &json!({ "name": "auto-filter", "query": "controllerid==meta-*" }),
        )
        .await;
    let filter_id = filter["id"].as_i64().unwrap();
    let (_, listed) = api.get("/rest/v1/targetfilters").await;
    assert!(
        listed["content"][0]
            .as_object()
            .unwrap()
            .contains_key("autoAssignDistributionSetName"),
        "the key must exist even when nothing is auto-assigned"
    );
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/targetfilters/{filter_id}/autoAssignDS"),
            &json!({ "id": ds_id, "type": "soft" }),
        )
        .await;
    assert_eq!(status, 200);
    let (_, one) = api
        .get(&format!("/rest/v1/targetfilters/{filter_id}"))
        .await;
    assert_eq!(one["autoAssignDistributionSetName"], "auto-ds 2.0");

    // --- target groups as an ordinary resource -------------------------
    for (controller_id, group) in [
        ("g-1", "eu/west"),
        ("g-2", "eu/west"),
        ("g-3", "eu/east"),
        ("g-4", "us"),
    ] {
        api.send(
            reqwest::Method::POST,
            "/rest/v1/targets",
            &json!([{ "controllerId": controller_id, "group": group }]),
        )
        .await;
    }
    let (status, groups) = api
        .get("/rest/v1/targetgroupdetails?sort=targets:DESC")
        .await;
    assert_eq!(status, 200);
    assert_eq!(groups["total"], 3, "three distinct groups");
    assert_eq!(groups["content"][0]["name"], "eu/west");
    assert_eq!(groups["content"][0]["targets"], 2, "biggest group first");

    let (_, filtered) = api.get("/rest/v1/targetgroupdetails?q=name==eu*").await;
    assert_eq!(filtered["total"], 2, "the filter runs on the server");

    // The count in a row and the query the console links to must agree.
    let (_, by_group) = api.get("/rest/v1/targets?q=group==%22eu/west%22").await;
    assert_eq!(by_group["total"], 2);

    // --- every column the console declares is sortable ------------------
    for (endpoint, column) in [
        ("targets", "assignedDistributionSetName"),
        // Computed in SQL from the tenant's polling config, so its
        // sortability is easier to break than a plain column's.
        ("targets", "pollState"),
        ("actions", "controllerId"),
        ("actions", "type"),
        ("actions", "rolloutName"),
        ("softwaremoduletypes", "maxAssignments"),
        ("targetgroupdetails", "name"),
        ("targetgroupdetails", "targets"),
    ] {
        let (status, _) = api
            .get(&format!("/rest/v1/{endpoint}?limit=1&sort={column}:ASC"))
            .await;
        assert_eq!(
            status, 200,
            "{endpoint}: sorting by '{column}' must work — the console renders \
             that column with a clickable header"
        );
    }

    // Aggregates stay out of the filter: an aggregate in WHERE is a 500.
    let (status, _) = api.get("/rest/v1/targetgroupdetails?q=targets==2").await;
    assert_eq!(status, 400, "unfilterable field must be a 400, not a crash");
}

// ---------------------------------------------------------------------------
// Target type compatibility (deployment-assignment spec)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn assignment_refuses_an_incompatible_target_type() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let rev2 = common::seed_revision(&server, "rev2").await;
    let board = common::seed_target(&server, "DEFAULT", "board-1").await;
    common::set_target_type(&server, board.id, rev1.target_type).await;
    let wrong = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-rev2", b"x").await;
    common::set_ds_type(&server, wrong.ds_id, rev2.ds_type).await;
    sqlx::query("UPDATE sp_distribution_set SET locked = FALSE WHERE id = $1")
        .bind(wrong.ds_id)
        .execute(&server.state.pool)
        .await
        .unwrap();

    let (status, body) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets/board-1/assignedDS",
            &json!([{ "id": wrong.ds_id }]),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["errorCode"],
        "hawkbit.server.error.target.type.incompatible"
    );
    assert_eq!(
        body["message"],
        "Target of type board-rev1 is not compatible with distribution set of types [os-rev2]"
    );
    assert_eq!(common::action_count(&server, board.id).await, 0);

    // The other direction names the target types.
    let (status, body) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/distributionsets/{}/assignedTargets", wrong.ds_id),
            &json!([{ "id": "board-1" }]),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["message"],
        "Targets of types [board-rev1] are not compatible with distribution set of type os-rev2"
    );
    assert_eq!(common::action_count(&server, board.id).await, 0);
    let locked: bool = sqlx::query_scalar("SELECT locked FROM sp_distribution_set WHERE id = $1")
        .bind(wrong.ds_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(!locked, "a refused assignment must not lock the set");
}

#[tokio::test]
#[ignore]
async fn one_incompatible_target_refuses_the_whole_request() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let rev2 = common::seed_revision(&server, "rev2").await;
    let good = common::seed_target(&server, "DEFAULT", "good").await;
    let bad = common::seed_target(&server, "DEFAULT", "bad").await;
    common::set_target_type(&server, good.id, rev1.target_type).await;
    common::set_target_type(&server, bad.id, rev2.target_type).await;
    let fw = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-rev1", b"x").await;
    common::set_ds_type(&server, fw.ds_id, rev1.ds_type).await;

    let (status, body) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/distributionsets/{}/assignedTargets", fw.ds_id),
            &json!([{ "id": "good" }, { "id": "bad" }]),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(common::action_count(&server, good.id).await, 0);
    assert_eq!(common::action_count(&server, bad.id).await, 0);
}

#[tokio::test]
#[ignore]
async fn compatible_and_untyped_targets_are_assigned() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let typed = common::seed_target(&server, "DEFAULT", "typed").await;
    let untyped = common::seed_target(&server, "DEFAULT", "untyped").await;
    common::set_target_type(&server, typed.id, rev1.target_type).await;
    let fw = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-rev1", b"x").await;
    common::set_ds_type(&server, fw.ds_id, rev1.ds_type).await;

    let (status, body) = api
        .send(
            reqwest::Method::POST,
            &format!("/rest/v1/distributionsets/{}/assignedTargets", fw.ds_id),
            &json!([{ "id": "typed" }, { "id": "untyped" }]),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["assigned"], 2);
    assert_eq!(common::action_count(&server, typed.id).await, 1);
    assert_eq!(common::action_count(&server, untyped.id).await, 1);
}

#[tokio::test]
#[ignore]
async fn target_type_required_is_a_known_key_off_by_default() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let (status, configs) = api.get("/rest/v1/system/configs").await;
    assert_eq!(status, 200);
    assert_eq!(
        configs["target.type.required"],
        json!({ "value": false, "global": true })
    );
}

#[tokio::test]
#[ignore]
async fn a_hardened_tenant_refuses_an_untyped_target() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let rev1 = common::seed_revision(&server, "rev1").await;
    let untyped = common::seed_target(&server, "DEFAULT", "no-type").await;
    let fw = common::seed_ds_with_artifact(&server, "DEFAULT", "fw-rev1", b"x").await;
    common::set_ds_type(&server, fw.ds_id, rev1.ds_type).await;
    let (status, _) = api
        .send(
            reqwest::Method::PUT,
            "/rest/v1/system/configs/target.type.required",
            &json!({ "value": true }),
        )
        .await;
    assert_eq!(status, 200);

    for (path, body) in [
        (
            "/rest/v1/targets/no-type/assignedDS".to_string(),
            json!([{ "id": fw.ds_id }]),
        ),
        (
            format!("/rest/v1/distributionsets/{}/assignedTargets", fw.ds_id),
            json!([{ "id": "no-type" }]),
        ),
    ] {
        let (status, refusal) = api.send(reqwest::Method::POST, &path, &body).await;
        assert_eq!(status, 400, "{path}: {refusal}");
        assert_eq!(
            refusal["errorCode"],
            "hawkbit.server.error.target.type.incompatible"
        );
        assert!(
            refusal["message"]
                .as_str()
                .unwrap()
                .contains("has no target type"),
            "{refusal}"
        );
    }
    assert_eq!(common::action_count(&server, untyped.id).await, 0);
}

/// hawkBit's `and` / `or` keywords mean `;` / `,`, and a word that is not a
/// keyword is a 400 — never one comparison against a spaced value that
/// silently matches nothing (a rollout over such a filter has no targets).
#[tokio::test]
#[ignore]
async fn rsql_keywords_match_the_punctuation_and_stray_words_are_refused() {
    let server = common::spawn().await;
    let api = Api::new(&server);
    let (status, _) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/targets",
            &json!([
                { "controllerId": "fleet-1", "name": "rock and roll" },
                { "controllerId": "fleet-2" },
                { "controllerId": "cross-1" },
                { "controllerId": "other-1" }
            ]),
        )
        .await;
    assert_eq!(status, 201);

    let total = |q: &str| {
        let path = format!("/rest/v1/targets?q={}", q.replace(' ', "%20"));
        let api = &api;
        async move { total_of(api, &path).await }
    };
    assert_eq!(total("controllerid==fleet-*,controllerid==cross*").await, 3);
    assert_eq!(
        total("controllerid==fleet-* or controllerid==cross*").await,
        3
    );
    assert_eq!(
        total("controllerid==fleet-* and name==\"rock and roll\"").await,
        1
    );
    assert_eq!(
        total("(controllerid==fleet-1 or controllerid==cross-1) and controllerid!=cross-1").await,
        1
    );

    for q in [
        "controllerid==fleet-*%20OR%20controllerid==cross*",
        "controllerid==fleet-*%20%26%26%20controllerid==cross*",
        "name==rock%20and%20roll",
    ] {
        let (status, error) = api.get(&format!("/rest/v1/targets?q={q}")).await;
        assert_eq!(status, 400, "{q} must be refused, got {error}");
        assert_eq!(
            error["errorCode"],
            "hawkbit.server.error.rest.param.rsqlInvalid"
        );
    }

    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "kw-fw", b"kw-payload").await;
    let (status, error) = api
        .send(
            reqwest::Method::POST,
            "/rest/v1/rollouts",
            &json!({
                "name": "keyword-typo",
                "distributionSetId": ds.ds_id,
                "targetFilterQuery": "controllerid==fleet-* AND controllerid==cross*",
                "amountGroups": 1,
                "type": "forced"
            }),
        )
        .await;
    assert_eq!(status, 400, "{error}");
    let (status, rollouts) = api.get("/rest/v1/rollouts").await;
    assert_eq!(status, 200);
    assert_eq!(rollouts["total"], 0, "no rollout was created: {rollouts}");
}
