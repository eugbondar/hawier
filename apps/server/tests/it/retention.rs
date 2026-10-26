//! Retention rules end to end. Run via `make it`.
//!
//! The rule fires directly rather than through the housekeeping loop, so the
//! tests are deterministic.

use crate::common;

use hawier_api::ext::kinds;
use serde_json::{json, Value};
use sqlx::Row;

/// A rule whose schedule has already fired, so a single tick runs it.
async fn rule(server: &common::TestServer, name: &str, dry_run: bool, params: Value) -> i64 {
    let id = server.state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_retention_rule
         (id, tenant, name, scope, filter_rsql, min_age_ms, cron, params, dry_run, active,
          last_run_at)
         VALUES ($1, 'DEFAULT', $2, 'artifact', '', 0, '* * * * * *', $3, $4, TRUE,
                 now() - interval '1 hour')",
    )
    .bind(id)
    .bind(name)
    .bind(params)
    .bind(dry_run)
    .execute(&server.state.pool)
    .await
    .unwrap();
    id
}

/// A software module with `count` artifacts, oldest first.
async fn module_with_artifacts(
    server: &common::TestServer,
    name: &str,
    version: &str,
    count: usize,
) -> (i64, Vec<i64>) {
    hawier_server::tenant::ensure_tenant(&server.state.pool, "DEFAULT")
        .await
        .unwrap();
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
         VALUES (1, 'test', 1, 'test', 1, 'DEFAULT', $1, $2, $3, FALSE, FALSE)
         RETURNING id",
    )
    .bind(name)
    .bind(version)
    .bind(sm_type)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();

    let mut ids = Vec::new();
    for n in 0..count {
        let content = format!("{name}-{version}-{n}");
        let stored = server
            .state
            .storage
            .store(
                "DEFAULT",
                futures::stream::iter(vec![Ok(bytes::Bytes::from(content.into_bytes()))]),
            )
            .await
            .unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO sp_artifact
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, md5_hash, file_size, provided_file_name, sha1_hash, software_module,
              sha256_hash)
             VALUES ($1, 'test', $1, 'test', 1, 'DEFAULT', $2, $3, $4, $5, $6, $7)
             RETURNING id",
        )
        .bind(1000 + n as i64)
        .bind(&stored.md5)
        .bind(stored.size)
        .bind(format!("{name}-{n}.bin"))
        .bind(&stored.sha1)
        .bind(sm_id)
        .bind(&stored.sha256)
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
        ids.push(id);
    }
    (sm_id, ids)
}

async fn artifacts_left(server: &common::TestServer, sm_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sp_artifact WHERE software_module = $1")
        .bind(sm_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}

async fn last_result(server: &common::TestServer, rule_id: i64) -> String {
    sqlx::query_scalar("SELECT last_result FROM hawier_retention_rule WHERE id = $1")
        .bind(rule_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn a_dry_run_reports_and_deletes_nothing() {
    let server = common::spawn().await;
    let (sm_id, _) = module_with_artifacts(&server, "dev-fw", "1.0.0-dev", 5).await;
    let rule_id = rule(&server, "dev-cleanup", true, json!({ "keepPerModule": 2 })).await;

    hawier_server::retention::tick(&server.state).await.unwrap();

    assert_eq!(
        artifacts_left(&server, sm_id).await,
        5,
        "a dry run must not delete anything"
    );
    let result = last_result(&server, rule_id).await;
    assert!(
        result.contains("dry run") && result.contains("would delete 3"),
        "the operator has to be told what it would have done: {result}"
    );
}

#[tokio::test]
#[ignore]
async fn an_armed_rule_keeps_the_newest_and_deletes_the_rest() {
    let server = common::spawn().await;
    let (sm_id, ids) = module_with_artifacts(&server, "dev-fw", "1.0.0-dev", 5).await;
    rule(&server, "dev-cleanup", false, json!({ "keepPerModule": 2 })).await;

    hawier_server::retention::tick(&server.state).await.unwrap();

    assert_eq!(artifacts_left(&server, sm_id).await, 2);
    let surviving: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM sp_artifact WHERE software_module = $1 ORDER BY id")
            .bind(sm_id)
            .fetch_all(&server.state.pool)
            .await
            .unwrap();
    assert_eq!(
        surviving,
        vec![ids[3], ids[4]],
        "the newest two by creation time survive"
    );
}

#[tokio::test]
#[ignore]
async fn releases_are_left_alone() {
    let server = common::spawn().await;
    let (sm_id, _) = module_with_artifacts(&server, "rel-fw", "1.0.0", 5).await;
    rule(&server, "dev-cleanup", false, json!({ "keepPerModule": 0 })).await;

    hawier_server::retention::tick(&server.state).await.unwrap();

    assert_eq!(
        artifacts_left(&server, sm_id).await,
        5,
        "a version with no pre-release marker is a release"
    );
}

#[tokio::test]
#[ignore]
async fn deleting_announces_itself() {
    let server = common::spawn().await;
    module_with_artifacts(&server, "dev-fw", "1.0.0-dev", 3).await;
    rule(&server, "dev-cleanup", false, json!({ "keepPerModule": 1 })).await;

    hawier_server::retention::tick(&server.state).await.unwrap();

    let events: Vec<Value> = sqlx::query("SELECT payload FROM hawier_event_outbox WHERE kind = $1")
        .bind(kinds::ARTIFACT_DELETED)
        .fetch_all(&server.state.pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get("payload"))
        .collect();
    assert_eq!(events.len(), 2, "an automatic deletion is still a deletion");
    assert_eq!(
        events[0]["deletedBy"], "retention:dev-cleanup",
        "and it says which rule did it"
    );
}

#[tokio::test]
#[ignore]
async fn shared_content_survives_until_the_last_reference_goes() {
    let server = common::spawn().await;
    // Two modules, same bytes: content-addressed storage stores one blob.
    let (_, first) = module_with_artifacts(&server, "twin-a", "1.0.0-dev", 1).await;
    let sha1: String = sqlx::query_scalar("SELECT sha1_hash FROM sp_artifact WHERE id = $1")
        .bind(first[0])
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    let (sm_b, _) = module_with_artifacts(&server, "twin-b", "1.0.0-dev", 1).await;
    sqlx::query("UPDATE sp_artifact SET sha1_hash = $1 WHERE software_module = $2")
        .bind(&sha1)
        .bind(sm_b)
        .execute(&server.state.pool)
        .await
        .unwrap();

    rule(&server, "dev-cleanup", false, json!({ "keepPerModule": 0 })).await;
    hawier_server::retention::tick(&server.state).await.unwrap();

    // Both rows are gone, and only then may the blob be.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sp_artifact WHERE sha1_hash = $1")
            .bind(&sha1)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        !server.state.storage.exists("DEFAULT", &sha1).await.unwrap(),
        "the last reference went, so the blob may go too"
    );
}

#[tokio::test]
#[ignore]
async fn a_rule_with_a_bad_filter_is_refused_on_write() {
    let server = common::spawn().await;
    let response = common::http()
        .post(format!("{}/rest/v1/retentionrules", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({
            "name": "typo",
            "cron": "0 0 3 * * *",
            "filter": "module.colour==red",
        }))
        .send()
        .await
        .unwrap();
    // A rule that cannot run is worse than no rule: it looks armed.
    assert_eq!(response.status().as_u16(), 400);

    let response = common::http()
        .post(format!("{}/rest/v1/retentionrules", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({ "name": "nope", "cron": "not a cron" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
}

#[tokio::test]
#[ignore]
async fn a_rule_created_through_the_api_defaults_to_dry_run() {
    let server = common::spawn().await;
    let response = common::http()
        .post(format!("{}/rest/v1/retentionrules", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({ "name": "careful", "cron": "0 0 3 * * *" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let created: Value = response.json().await.unwrap();
    assert_eq!(
        created["dryRun"], true,
        "automation that deletes starts out only saying what it would do"
    );
}

#[tokio::test]
#[ignore]
async fn a_rules_filter_narrows_what_the_strategy_is_offered() {
    let server = common::spawn().await;
    let (kept, _) = module_with_artifacts(&server, "other-fw", "1.0.0-dev", 3).await;
    let (swept, _) = module_with_artifacts(&server, "dev-fw", "1.0.0-dev", 3).await;

    // The filter is the host's half of the decision, and it runs as SQL with
    // its own bind placeholders — a rule with a filter exercises a path an
    // empty one never touches.
    let id = server.state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_retention_rule
         (id, tenant, name, scope, filter_rsql, min_age_ms, cron, params, dry_run, active,
          last_run_at)
         VALUES ($1, 'DEFAULT', 'only-dev-fw', 'artifact', 'module.name==dev-fw', 0,
                 '* * * * * *', $2, FALSE, TRUE, now() - interval '1 hour')",
    )
    .bind(id)
    .bind(json!({ "keepPerModule": 1 }))
    .execute(&server.state.pool)
    .await
    .unwrap();

    hawier_server::retention::tick(&server.state).await.unwrap();

    assert_eq!(
        artifacts_left(&server, kept).await,
        3,
        "a module the filter excludes is never even offered"
    );
    assert_eq!(artifacts_left(&server, swept).await, 1);
}
