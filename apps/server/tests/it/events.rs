//! Domain events end to end: emission into the outbox in the same
//! transaction as the change, and delivery by the drainer. Run via `make it`.
//!
//! The drainer is called directly rather than through its background loop,
//! so the tests are deterministic.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::Router;
use hawier_api::ext::kinds;
use serde_json::{json, Value};
use sqlx::Row;

/// What a subscriber saw, so the test can assert on the wire format.
#[derive(Debug, Clone)]
struct Received {
    kind: String,
    signature: Option<String>,
    body: String,
}

#[derive(Clone)]
struct Receiver {
    seen: Arc<Mutex<Vec<Received>>>,
    /// Status to answer with — 500 turns the subscriber into a failing one.
    status: u16,
}

/// A stand-in subscriber on an ephemeral port.
async fn spawn_receiver(status: u16) -> (String, Arc<Mutex<Vec<Received>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let receiver = Receiver {
        seen: seen.clone(),
        status,
    };
    let app = Router::new()
        .route("/hook", post(handle))
        .with_state(receiver);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}/hook"), seen)
}

async fn handle(
    State(receiver): State<Receiver>,
    headers: HeaderMap,
    body: String,
) -> axum::http::StatusCode {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    receiver.seen.lock().unwrap().push(Received {
        kind: header("x-hawier-event").unwrap_or_default(),
        signature: header("x-hawier-signature"),
        body,
    });
    axum::http::StatusCode::from_u16(receiver.status).unwrap()
}

async fn subscribe(server: &common::TestServer, url: &str, secret: &str, mask: &str) -> i64 {
    let id = server.state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_webhook (id, tenant, name, url, secret, event_mask)
         VALUES ($1, 'DEFAULT', 'test-hook', $2, $3, $4)",
    )
    .bind(id)
    .bind(url)
    .bind(secret)
    .bind(mask)
    .execute(&server.state.pool)
    .await
    .unwrap();
    id
}

async fn outbox_rows(server: &common::TestServer, kind: &str) -> Vec<(i64, String, i32, Value)> {
    sqlx::query(
        "SELECT id, state, attempts, payload FROM hawier_event_outbox
         WHERE kind = $1 ORDER BY id",
    )
    .bind(kind)
    .fetch_all(&server.state.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| {
        (
            r.get("id"),
            r.get("state"),
            r.get("attempts"),
            r.get("payload"),
        )
    })
    .collect()
}

async fn drain(server: &common::TestServer) {
    let client = reqwest::Client::new();
    hawier_server::outbox::tick(&server.state, &client)
        .await
        .unwrap();
}

async fn create_rollout(server: &common::TestServer, name: &str) -> i64 {
    let ds = common::seed_ds_with_artifact(server, "DEFAULT", "evt-fw", b"payload").await;
    let http = common::http();
    let response = http
        .post(format!("{}/rest/v1/rollouts", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({
            "name": name,
            "distributionSetId": ds.ds_id,
            "targetFilterQuery": "controllerid==evt-*",
            "amountGroups": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    response.json::<Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn creating_a_rollout_writes_an_event_in_the_same_transaction() {
    let server = common::spawn().await;
    let rollout_id = create_rollout(&server, "evt-rollout").await;

    let rows = outbox_rows(&server, kinds::ROLLOUT_CREATED).await;
    assert_eq!(rows.len(), 1, "expected exactly one created event");
    let (_, state, attempts, payload) = &rows[0];
    assert_eq!(state, "pending");
    assert_eq!(*attempts, 0);
    assert_eq!(payload["rolloutId"].as_i64(), Some(rollout_id));
    assert_eq!(payload["name"], "evt-rollout");
}

/// An operator pressing pause is the same fact as the engine pausing on a
/// threshold, and a subscriber asked to hear about pauses meant both.
///
/// Only the engine emitted it, so `page-on-pause` — the obvious reason to
/// subscribe at all — stayed silent for every pause a person performed.
#[tokio::test]
#[ignore]
async fn an_operator_pause_is_announced_like_the_engine_s() {
    let server = common::spawn().await;
    // Targets for the filter, so the rollout has a group to run.
    for n in 0..3 {
        common::seed_target(&server, "DEFAULT", &format!("evt-{n}")).await;
    }
    let rollout = create_rollout(&server, "pause-me").await;

    let http = common::http();
    let act = |verb: &'static str| {
        let url = format!("{}/rest/v1/rollouts/{rollout}/{verb}", server.base_url);
        let http = http.clone();
        async move {
            http.post(url)
                .header("Authorization", common::basic("admin", "admin"))
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };

    // creating -> ready -> starting -> running, the way the engine does it.
    hawier_server::rollout::tick(&server.state).await.unwrap();
    assert_eq!(act("start").await, 200);
    hawier_server::rollout::tick(&server.state).await.unwrap();

    assert!(
        outbox_rows(&server, hawier_api::ext::kinds::ROLLOUT_PAUSED)
            .await
            .is_empty(),
        "nothing has paused yet"
    );
    assert_eq!(act("pause").await, 200);

    let paused = outbox_rows(&server, hawier_api::ext::kinds::ROLLOUT_PAUSED).await;
    assert_eq!(paused.len(), 1, "the operator's pause must be announced");
    let payload = &paused[0].3;
    assert_eq!(payload["rolloutId"].as_i64(), Some(rollout));
    assert!(
        payload["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("admin"),
        "the reason says who did it: {payload}"
    );
}

#[tokio::test]
#[ignore]
async fn a_rejected_rollout_leaves_no_event_behind() {
    let server = common::spawn().await;
    create_rollout(&server, "evt-dup").await;

    // Same name twice: the second create fails on the unique constraint, and
    // the whole transaction — event included — must roll back with it.
    let ds_id: i64 = sqlx::query_scalar("SELECT id FROM sp_distribution_set LIMIT 1")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    let response = common::http()
        .post(format!("{}/rest/v1/rollouts", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({
            "name": "evt-dup",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==evt-*",
            "amountGroups": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 409);

    let rows = outbox_rows(&server, kinds::ROLLOUT_CREATED).await;
    assert_eq!(
        rows.len(),
        1,
        "the failed create must not have left an event"
    );
}

#[tokio::test]
#[ignore]
async fn the_drainer_delivers_and_signs() {
    let server = common::spawn().await;
    let (url, seen) = spawn_receiver(200).await;
    let webhook_id = subscribe(&server, &url, "s3cret", "*").await;
    create_rollout(&server, "evt-delivered").await;

    drain(&server).await;

    let received = seen.lock().unwrap().clone();
    assert_eq!(received.len(), 1, "subscriber should have been called once");
    assert_eq!(received[0].kind, kinds::ROLLOUT_CREATED);

    // The signature is an HMAC of the exact body, so a subscriber can tell
    // the call came from us.
    let signature = received[0]
        .signature
        .clone()
        .expect("a subscription with a secret must be signed");
    assert!(signature.starts_with("sha256="));
    let body: Value = serde_json::from_str(&received[0].body).unwrap();
    assert_eq!(body["kind"], kinds::ROLLOUT_CREATED);
    assert_eq!(body["tenant"], "DEFAULT");

    let rows = outbox_rows(&server, kinds::ROLLOUT_CREATED).await;
    assert_eq!(rows[0].1, "delivered");

    let deliveries: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hawier_webhook_delivery WHERE webhook = $1 AND status = 200",
    )
    .bind(webhook_id)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(deliveries, 1, "the attempt must be recorded");
}

#[tokio::test]
#[ignore]
async fn an_unsigned_subscription_gets_no_signature_header() {
    let server = common::spawn().await;
    let (url, seen) = spawn_receiver(200).await;
    subscribe(&server, &url, "", "*").await;
    create_rollout(&server, "evt-unsigned").await;

    drain(&server).await;

    let received = seen.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    assert!(received[0].signature.is_none());
}

#[tokio::test]
#[ignore]
async fn a_mask_keeps_unrelated_events_away() {
    let server = common::spawn().await;
    let (url, seen) = spawn_receiver(200).await;
    subscribe(&server, &url, "", kinds::ARTIFACT_DELETED).await;
    create_rollout(&server, "evt-masked").await;

    drain(&server).await;

    assert!(
        seen.lock().unwrap().is_empty(),
        "a subscription for another kind must not be called"
    );
    // With nothing to deliver the event is still done, not stuck pending.
    let rows = outbox_rows(&server, kinds::ROLLOUT_CREATED).await;
    assert_eq!(rows[0].1, "delivered");
}

#[tokio::test]
#[ignore]
async fn a_failing_subscriber_is_retried_later_not_dropped() {
    let server = common::spawn().await;
    let (url, seen) = spawn_receiver(500).await;
    let webhook_id = subscribe(&server, &url, "", "*").await;
    create_rollout(&server, "evt-failing").await;

    drain(&server).await;

    assert_eq!(seen.lock().unwrap().len(), 1, "it should have been tried");
    let rows = outbox_rows(&server, kinds::ROLLOUT_CREATED).await;
    assert_eq!(
        rows[0].1, "pending",
        "a failed event goes back in the queue"
    );
    assert_eq!(rows[0].2, 1, "the attempt must be counted");

    // Backoff, so an immediate second drain does not hammer the subscriber.
    drain(&server).await;
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the retry must wait for its backoff"
    );

    let failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hawier_webhook_delivery WHERE webhook = $1 AND status = 0",
    )
    .bind(webhook_id)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    assert_eq!(failed, 1, "the failure must be visible afterwards");
}

#[tokio::test]
#[ignore]
async fn artifact_upload_and_delete_are_announced() {
    let server = common::spawn().await;
    // The seeded module is locked (an assigned DS locks its modules), and a
    // locked module refuses uploads — so this test needs its own.
    common::seed_ds_with_artifact(&server, "DEFAULT", "evt-art", b"bytes").await;
    let sm_type: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_software_module_type
         WHERE UPPER(tenant) = 'DEFAULT' AND type_key = 'os'",
    )
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    let sm_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_software_module
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, version, sm_type, deleted, locked)
         VALUES (1, 'test', 1, 'test', 1, 'DEFAULT', 'evt-open', '1.0.0', $1, FALSE, FALSE)
         RETURNING id",
    )
    .bind(sm_type)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    let http = common::http();

    // Upload through the mgmt API so the handler's own transaction runs.
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(b"fresh-artifact".to_vec()).file_name("fresh.bin"),
    );
    let response = http
        .post(format!(
            "{}/rest/v1/softwaremodules/{}/artifacts",
            server.base_url, sm_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let artifact_id = response.json::<Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap();

    let uploaded = outbox_rows(&server, kinds::ARTIFACT_UPLOADED).await;
    assert_eq!(uploaded.len(), 1);
    assert_eq!(uploaded[0].3["filename"], "fresh.bin");

    let response = http
        .delete(format!(
            "{}/rest/v1/softwaremodules/{}/artifacts/{}",
            server.base_url, sm_id, artifact_id
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    let deleted = outbox_rows(&server, kinds::ARTIFACT_DELETED).await;
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].3["artifactId"].as_i64(), Some(artifact_id));
    assert_eq!(deleted[0].3["blobRemoved"], true);
}

// ---------------------------------------------------------------------------
// The subscription resource itself
// ---------------------------------------------------------------------------

async fn mgmt(
    server: &common::TestServer,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = common::http()
        .request(method, format!("{}{path}", server.base_url))
        .header("Authorization", common::basic("admin", "admin"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore]
async fn a_subscription_secret_is_write_only() {
    let server = common::spawn().await;
    let (status, created) = mgmt(
        &server,
        reqwest::Method::POST,
        "/rest/v1/webhooks",
        Some(json!({
            "name": "release-bot",
            "url": "https://example.invalid/hook",
            "secret": "s3cret",
        })),
    )
    .await;
    assert_eq!(status, 201, "create failed: {created}");

    // The secret is what a subscriber verifies the HMAC with. It goes in and
    // never comes back — only whether deliveries are signed at all.
    assert!(
        created.get("secret").is_none(),
        "the secret must never be rendered back: {created}"
    );
    assert_eq!(created["signed"], true);

    let id = created["id"].as_str().unwrap();
    let (_, fetched) = mgmt(
        &server,
        reqwest::Method::GET,
        &format!("/rest/v1/webhooks/{id}"),
        None,
    )
    .await;
    assert!(
        fetched.get("secret").is_none(),
        "not on the object page either"
    );

    let (_, listed) = mgmt(&server, reqwest::Method::GET, "/rest/v1/webhooks", None).await;
    assert!(
        !serde_json::to_string(&listed).unwrap().contains("s3cret"),
        "and not in the listing"
    );
}

#[tokio::test]
#[ignore]
async fn an_update_without_a_secret_keeps_the_stored_one() {
    let server = common::spawn().await;
    let (_, created) = mgmt(
        &server,
        reqwest::Method::POST,
        "/rest/v1/webhooks",
        Some(json!({
            "name": "keep-secret",
            "url": "https://example.invalid/hook",
            "secret": "s3cret",
        })),
    )
    .await;
    let id = created["id"].as_str().unwrap();

    // The console cannot round-trip a secret it was never given, so an edit
    // that leaves the field blank must not silently unsign the subscription.
    let (status, updated) = mgmt(
        &server,
        reqwest::Method::PUT,
        &format!("/rest/v1/webhooks/{id}"),
        Some(json!({ "name": "renamed" })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(updated["name"], "renamed");
    assert_eq!(
        updated["signed"], true,
        "an omitted secret keeps the stored one"
    );
}

#[tokio::test]
#[ignore]
async fn a_mask_naming_an_unknown_event_is_refused() {
    let server = common::spawn().await;
    let (status, body) = mgmt(
        &server,
        reqwest::Method::POST,
        "/rest/v1/webhooks",
        Some(json!({
            "name": "typo",
            "url": "https://example.invalid/hook",
            "eventMask": "rollout.exploded",
        })),
    )
    .await;
    // Accepting it would create a subscription that looks healthy and never
    // fires — the worst possible failure for a notification.
    assert_eq!(status, 400, "unknown kind must be refused: {body}");

    let (status, _) = mgmt(
        &server,
        reqwest::Method::POST,
        "/rest/v1/webhooks",
        Some(json!({
            "name": "fine",
            "url": "https://example.invalid/hook",
            "eventMask": format!("{},{}", kinds::ROLLOUT_STARTED, kinds::ARTIFACT_UPLOADED),
        })),
    )
    .await;
    assert_eq!(status, 201, "known kinds are accepted");
}

#[tokio::test]
#[ignore]
async fn a_subscription_created_through_the_api_receives_events() {
    let server = common::spawn().await;
    let (url, seen) = spawn_receiver(200).await;
    let (status, _) = mgmt(
        &server,
        reqwest::Method::POST,
        "/rest/v1/webhooks",
        Some(json!({
            "name": "end-to-end",
            "url": url,
            "eventMask": kinds::ROLLOUT_CREATED,
        })),
    )
    .await;
    assert_eq!(status, 201);

    create_rollout(&server, "evt-api-hook").await;
    drain(&server).await;

    let received = seen.lock().unwrap().clone();
    assert_eq!(
        received.len(),
        1,
        "the subscription an operator made must fire"
    );
    assert_eq!(received[0].kind, kinds::ROLLOUT_CREATED);
}
