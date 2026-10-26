//! `/ui/v1` state API tests (Phase 5). Run via `make it`.

use crate::common;

use serde_json::{json, Value};

fn ui(server: &common::TestServer, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    common::http()
        .request(method, format!("{}{}", server.base_url, path))
        .bearer_auth("it-internal-token")
}

#[tokio::test]
#[ignore]
async fn ui_state_requires_bearer_token() {
    let server = common::spawn().await;
    let resp = common::http()
        .get(format!(
            "{}/ui/v1/views?tenant=DEFAULT&username=admin",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let resp = common::http()
        .get(format!(
            "{}/ui/v1/views?tenant=DEFAULT&username=admin",
            server.base_url
        ))
        .bearer_auth("wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
#[ignore]
async fn ui_views_crud_and_reorder() {
    let server = common::spawn().await;

    // Create two views.
    let mut ids = Vec::new();
    for name in ["first", "second"] {
        let resp = ui(&server, reqwest::Method::POST, "/ui/v1/views")
            .json(&json!({
                "tenant": "DEFAULT", "username": "admin", "resource": "targets",
                "name": name, "q": "name==x*", "columns": "name,updateStatus",
                "sort": "name:ASC", "page_size": 50
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let view: Value = resp.json().await.unwrap();
        assert_eq!(view["name"], name);
        ids.push(view["id"].as_str().unwrap().to_string());
    }

    // Listing is scoped and position-ordered.
    let listing: Vec<Value> = ui(
        &server,
        reqwest::Method::GET,
        "/ui/v1/views?tenant=DEFAULT&username=admin&resource=targets",
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(listing.len(), 2);
    assert_eq!(listing[0]["name"], "first");
    assert_eq!(listing[0]["position"], 0);

    // Another user sees nothing.
    let other: Vec<Value> = ui(
        &server,
        reqwest::Method::GET,
        "/ui/v1/views?tenant=DEFAULT&username=bob&resource=targets",
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(other.is_empty());

    // Update.
    let resp = ui(
        &server,
        reqwest::Method::PUT,
        &format!("/ui/v1/views/{}", ids[0]),
    )
    .json(&json!({ "name": "renamed", "page_size": 100 }))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 200);
    let updated: Value = resp.json().await.unwrap();
    assert_eq!(updated["name"], "renamed");
    assert_eq!(updated["page_size"], 100);

    // Reorder (swap).
    let resp = ui(&server, reqwest::Method::PUT, "/ui/v1/views/reorder")
        .json(&json!({
            "tenant": "DEFAULT", "username": "admin", "resource": "targets",
            "ids": [ids[1], ids[0]]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let listing: Vec<Value> = ui(
        &server,
        reqwest::Method::GET,
        "/ui/v1/views?tenant=DEFAULT&username=admin&resource=targets",
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(listing[0]["name"], "second");

    // Delete.
    let resp = ui(
        &server,
        reqwest::Method::DELETE,
        &format!("/ui/v1/views/{}", ids[0]),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
#[ignore]
async fn ui_logs_two_phase_and_retention_scope() {
    let server = common::spawn().await;

    let resp = ui(&server, reqwest::Method::POST, "/ui/v1/logs")
        .json(&json!({
            "tenant": "DEFAULT", "username": "admin", "resource": "targets",
            "resource_id": "dev-1", "action": "delete",
            "request_summary": "DELETE /rest/v1/targets/dev-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let log: Value = resp.json().await.unwrap();
    assert_eq!(log["status"], "running");
    // Timestamps are epoch millis; an unfinished entry has none yet.
    assert!(log["created_at"].as_i64().unwrap() > 1_700_000_000_000);
    assert!(log["finished_at"].is_null());
    let log_id = log["id"].as_str().unwrap().to_string();

    let resp = ui(
        &server,
        reqwest::Method::PUT,
        &format!("/ui/v1/logs/{log_id}"),
    )
    .json(&json!({ "status": "success", "response_summary": "200 OK" }))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 200);
    let finished: Value = resp.json().await.unwrap();
    assert_eq!(finished["status"], "success");
    assert!(finished["finished_at"].as_i64().unwrap() > 1_700_000_000_000);

    // Reading the log is a mgmt resource, scoped by the caller's tenant.
    let listing: Value = common::http()
        .get(format!("{}/rest/v1/actionlog", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["total"], 1);
    let row = &listing["content"][0];
    assert_eq!(row["resource"], "targets");
    assert_eq!(row["resourceId"], "dev-1");
    assert!(row["createdAt"].as_i64().unwrap() > 1_700_000_000_000);

    // RSQL filtering works like on every other resource.
    let filtered: Value = common::http()
        .get(format!("{}/rest/v1/actionlog", server.base_url))
        .query(&[("q", "action==nosuchaction")])
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["total"], 0);
}

#[tokio::test]
#[ignore]
async fn ui_sessions_roundtrip_and_expiry() {
    let server = common::spawn().await;

    let resp = ui(&server, reqwest::Method::POST, "/ui/v1/sessions")
        .json(&json!({ "username": "admin", "tenant": "DEFAULT", "password": "admin" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let session: Value = resp.json().await.unwrap();
    // The id is an ordinary number; the cookie carries the random token,
    // because whoever holds it can read the stored password.
    let session_id = session["id"].as_str().unwrap().to_string();
    assert!(
        session_id.parse::<i64>().is_ok(),
        "session id must be a number, got {session_id}"
    );
    let token = session["token"].as_str().unwrap().to_string();
    assert_ne!(token, session_id, "the cookie must not carry the id");
    assert_eq!(session["password"], "admin");

    let resp = ui(
        &server,
        reqwest::Method::GET,
        &format!("/ui/v1/sessions/{token}"),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 200);

    // Force-expire in the DB → the API stops serving it.
    sqlx::query("UPDATE hawier_session SET expires_at = now() - interval '1 minute'")
        .execute(&server.state.pool)
        .await
        .unwrap();
    let resp = ui(
        &server,
        reqwest::Method::GET,
        &format!("/ui/v1/sessions/{token}"),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 404);

    // Logout is idempotent.
    let resp = ui(
        &server,
        reqwest::Method::DELETE,
        &format!("/ui/v1/sessions/{token}"),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 204);
}
