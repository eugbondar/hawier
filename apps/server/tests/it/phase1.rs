//! Phase 1 integration tests: migrations, Flyway history, mgmt auth,
//! tenant bootstrap. Run via `make it`.

use crate::common;

use sqlx::Row;

#[tokio::test]
#[ignore]
async fn migrate_fresh_db_writes_flyway_history() {
    let server = common::spawn().await;

    let rows = sqlx::query(
        r#"SELECT "installed_rank", "version", "description", "type", "script",
                  "checksum", "success"
           FROM "schema_version" ORDER BY "installed_rank""#,
    )
    .fetch_all(&server.state.pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 3, "exactly the three hawkbit migrations");
    let expected = &hawier_server::db::flyway::HAWKBIT_MIGRATIONS;
    for (row, migration) in rows.iter().zip(expected.iter()) {
        assert_eq!(row.get::<String, _>("version"), migration.version);
        assert_eq!(row.get::<String, _>("type"), migration.kind);
        assert_eq!(row.get::<String, _>("script"), migration.script);
        assert!(row.get::<bool, _>("success"));
        assert_eq!(
            row.get::<Option<i32>, _>("checksum"),
            Some(hawier_server::db::flyway::checksum(migration.sql)),
            "checksum must match the Flyway line-CRC32 of {}",
            migration.script
        );
    }

    // Key hawkbit tables exist.
    for table in [
        "sp_target",
        "sp_action",
        "sp_rollout",
        "sp_artifact",
        "sp_lock",
    ] {
        let exists: bool = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
             WHERE table_name = $1) AS e",
        )
        .bind(table)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
        .get("e");
        assert!(exists, "table {table} must exist");
    }

    // SP_LOCK got the V1_20_1 column.
    let has_expired_after: bool = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
         WHERE table_name = 'sp_lock' AND column_name = 'expired_after') AS e",
    )
    .fetch_one(&server.state.pool)
    .await
    .unwrap()
    .get("e");
    assert!(has_expired_after);

    // hawier's own track.
    let hawier_rows =
        sqlx::query("SELECT version, name FROM hawier_schema_version ORDER BY version")
            .fetch_all(&server.state.pool)
            .await
            .unwrap();
    // The hawier track is one file, edited in place while we are still in
    // development — so assert the resulting tables, not a list of names.
    assert_eq!(hawier_rows.len(), 1);
    assert_eq!(hawier_rows[0].get::<String, _>("name"), "0001_hawier");
    for table in [
        "hawier_saved_view",
        "hawier_action_log",
        "hawier_session",
        "hawier_incident",
        "hawier_search_query",
    ] {
        let exists: bool = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
             WHERE table_name = $1) AS e",
        )
        .bind(table)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
        .get("e");
        assert!(exists, "table {table} must exist");
    }
}

#[tokio::test]
#[ignore]
async fn migrate_is_idempotent() {
    let server = common::spawn().await;
    // Second run must be a no-op, not an error.
    // The database is already up here, so the wait never elapses.
    hawier_server::db::migrate::run(&server.db_url, std::time::Duration::from_secs(5))
        .await
        .unwrap();
    let count: i64 = sqlx::query(r#"SELECT COUNT(*) AS c FROM "schema_version""#)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
        .get("c");
    assert_eq!(count, 3);
}

#[tokio::test]
#[ignore]
async fn userinfo_authenticates_and_bootstraps_tenant() {
    let server = common::spawn().await;
    let client = common::http();

    // No credentials → 401 with hawkbit-style error body.
    let resp = client
        .get(format!("{}/rest/v1/userinfo", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Wrong password → 401.
    let resp = client
        .get(format!("{}/rest/v1/userinfo", server.base_url))
        .header("Authorization", common::basic("admin", "nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Correct credentials → 200 + tenant bootstrap.
    let resp = client
        .get(format!("{}/rest/v1/userinfo", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["username"], "admin");
    assert_eq!(body["tenant"], "DEFAULT");

    // Standard hawkbit type set created for the tenant.
    let smt: Vec<String> = sqlx::query(
        "SELECT type_key FROM sp_software_module_type WHERE tenant = 'DEFAULT' ORDER BY type_key",
    )
    .fetch_all(&server.state.pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get("type_key"))
    .collect();
    assert_eq!(smt, ["application", "os"]);

    let dst: Vec<String> = sqlx::query(
        "SELECT type_key FROM sp_distribution_set_type WHERE tenant = 'DEFAULT' ORDER BY type_key",
    )
    .fetch_all(&server.state.pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get("type_key"))
    .collect();
    assert_eq!(dst, ["app", "os", "os_app"]);

    let elements: i64 = sqlx::query("SELECT COUNT(*) AS c FROM sp_ds_type_element")
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
        .get("c");
    assert_eq!(elements, 4);

    // Tenant default DS type is os_app.
    let default_key: String = sqlx::query(
        "SELECT t.type_key FROM sp_tenant ten
         JOIN sp_distribution_set_type t ON t.id = ten.default_ds_type
         WHERE ten.tenant = 'DEFAULT'",
    )
    .fetch_one(&server.state.pool)
    .await
    .unwrap()
    .get("type_key");
    assert_eq!(default_key, "os_app");
}
