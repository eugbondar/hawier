//! Incident-number mechanics: one decimal snowflake per request
//! (X-Request-Id), promoted to `"incident"` in every error body and to the
//! `hawier_incident` journal PK. `make it`.

use crate::common;

use serde_json::Value;
use sqlx::Row;

/// The header value must be a bare decimal snowflake (positive i64).
fn parse_incident(raw: &str) -> i64 {
    assert!(
        !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()),
        "expected a decimal incident number, got '{raw}'"
    );
    raw.parse().expect("fits into i64")
}

#[tokio::test]
#[ignore]
async fn every_request_gets_a_decimal_incident_number() {
    let server = common::spawn().await;

    // A successful request carries the number too — it is the trace id.
    let resp = common::http()
        .get(format!("{}/rest/v1/targets?limit=1", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let id = parse_incident(
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    );
    // Decodes back to this node and to roughly now.
    let parts = hawier_server::incident::decode(id);
    assert_eq!(parts.node, 9, "test config node id");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!((parts.timestamp_ms - now).abs() < 10_000);

    // A client-supplied X-Request-Id is never trusted (it is a DB PK).
    let resp = common::http()
        .get(format!("{}/rest/v1/targets?limit=1", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .header("X-Request-Id", "1")
        .send()
        .await
        .unwrap();
    let minted = parse_incident(
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    );
    assert_ne!(minted, 1, "client value must be overwritten");
}

#[tokio::test]
#[ignore]
async fn mgmt_error_carries_incident_and_lands_in_journal() {
    let server = common::spawn().await;

    // Broken RSQL → 400 with the incident number in header AND body.
    let resp = common::http()
        .get(format!(
            "{}/rest/v1/targets?q=nosuchfield==1",
            server.base_url
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let header_id = parse_incident(
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    );
    let body: Value = resp.json().await.unwrap();
    let incident = body["incident"].as_str().unwrap_or_default().to_string();
    assert_eq!(
        parse_incident(&incident),
        header_id,
        "one number ties header, body and journal: {body}"
    );
    assert!(body["errorCode"].as_str().unwrap().contains("rsql"));
    assert!(body.get("errorId").is_none(), "old field must be gone");
    assert!(body.get("requestId").is_none(), "old field must be gone");

    // The journal row exists under that id. Journaling is deliberately
    // fire-and-forget — an unlogged error must not fail the response — so
    // this polls, and the window is generous because a loaded machine
    // running the whole suite is exactly when it is slowest.
    let mut row = None;
    for _ in 0..100 {
        row = sqlx::query("SELECT status, path, username FROM hawier_incident WHERE id = $1")
            .bind(header_id)
            .fetch_optional(&server.state.pool)
            .await
            .unwrap();
        if row.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let row = row.expect("journal row for incident");
    assert_eq!(row.get::<i32, _>("status"), 400);
    assert_eq!(row.get::<String, _>("path"), "/rest/v1/targets");
    assert_eq!(
        row.get::<Option<String>, _>("username").as_deref(),
        Some("admin")
    );

    // The journal is an ordinary mgmt resource: RSQL finds it by number…
    let listing: Value = common::http()
        .get(format!("{}/rest/v1/incidents", server.base_url))
        .query(&[("q", format!("incident=={incident}"))])
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = listing["content"].as_array().expect("paged envelope");
    assert_eq!(content.len(), 1);
    assert_eq!(listing["total"], 1);
    assert_eq!(content[0]["incident"], Value::String(incident.clone()));
    assert_eq!(content[0]["status"], 400);
    // Requests made with Basic auth against /rest are plain mgmt clients.
    assert_eq!(content[0]["kind"], "mgmt");

    // …and the direct incident endpoint returns exactly it, with decode.
    let one: Value = common::http()
        .get(format!("{}/rest/v1/incidents/{incident}", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["incident"], Value::String(incident));
    assert_eq!(one["node"], 9);
    assert!(one["occurredAt"].as_i64().unwrap() > 0);
}

#[tokio::test]
#[ignore]
async fn plain_text_rejections_are_wrapped_into_exception_info() {
    let server = common::spawn().await;

    // Malformed JSON body → axum rejection (plain text) → wrapped JSON.
    let resp = common::http()
        .post(format!("{}/rest/v1/targets", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .header("Content-Type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .contains("application/json"));
    let body: Value = resp.json().await.unwrap();
    parse_incident(body["incident"].as_str().unwrap_or_default());
    assert!(body["message"].as_str().is_some());

    // 405 (GET-only route) is wrapped too.
    let resp = common::http()
        .delete(format!("{}/rest/v1/userinfo", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["errorCode"], "hawkbit.server.error.methodNotAllowed");
}

#[tokio::test]
#[ignore]
async fn ddi_auth_failures_are_not_journaled() {
    let server = common::spawn().await;

    // Unknown device with a bad token → 401 (carries the number, no row).
    let resp = common::http()
        .get(format!(
            "{}/DEFAULT/controller/v1/flood-dev",
            server.base_url
        ))
        .header(
            "Authorization",
            "TargetToken 00000000000000000000000000000000",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    parse_incident(body["incident"].as_str().unwrap_or_default());

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let journaled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hawier_incident WHERE path LIKE '%flood-dev%'")
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(journaled, 0, "DDI 4xx must not flood the journal");
}
