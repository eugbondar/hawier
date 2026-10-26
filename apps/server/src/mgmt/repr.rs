//! Shared mgmt JSON builders for entities referenced across resource
//! modules (DS appears under targets/assignedDS, SM under DS, tags under
//! several resources...). Field names follow hawkBit's Mgmt DTOs exactly.

use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::put_audit;

/// Columns every DS query must select for [`ds_json`]:
/// `d.* , type_key, type_name, complete` (see [`DS_SELECT`]).
pub const DS_SELECT: &str = "SELECT d.id, d.created_at, d.created_by, d.last_modified_at,
       d.last_modified_by, d.description, d.name, d.version, d.deleted,
       d.required_migration_step, d.valid, d.locked,
       t.type_key AS type_key, t.name AS type_name,
       NOT EXISTS (
           SELECT 1 FROM sp_ds_type_element e
           JOIN sp_software_module_type mt ON mt.id = e.software_module_type
           WHERE e.distribution_set_type = d.ds_type AND e.mandatory = TRUE
             AND NOT EXISTS (
                 SELECT 1 FROM sp_ds_sm l
                 JOIN sp_software_module m ON m.id = l.sm_id
                 WHERE l.ds_id = d.id AND m.sm_type = mt.id)
       ) AS complete
FROM sp_distribution_set d
JOIN sp_distribution_set_type t ON t.id = d.ds_type";

pub fn ds_json(row: &PgRow, base: &str, modules: Vec<Value>) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "version": row.get::<String, _>("version"),
        "modules": modules,
        "requiredMigrationStep": row.get::<Option<bool>, _>("required_migration_step").unwrap_or(false),
        "type": row.get::<String, _>("type_key"),
        "typeName": row.get::<String, _>("type_name"),
        "complete": row.get::<bool, _>("complete"),
        "deleted": row.get::<Option<bool>, _>("deleted").unwrap_or(false),
        "valid": row.get::<Option<bool>, _>("valid").unwrap_or(true),
        "locked": row.get::<bool, _>("locked"),
        "_links": { "self": { "href": format!("{base}/rest/v1/distributionsets/{id}") } }
    });
    put_audit(&mut value, row);
    value
}

/// Columns every SM query must select for [`sm_json`] (see [`SM_SELECT`]).
pub const SM_SELECT: &str = "SELECT m.id, m.created_at, m.created_by, m.last_modified_at,
       m.last_modified_by, m.description, m.name, m.version, m.deleted,
       m.vendor, m.encrypted, m.locked,
       t.type_key AS type_key, t.name AS type_name
FROM sp_software_module m
JOIN sp_software_module_type t ON t.id = m.sm_type";

pub fn sm_json(row: &PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "version": row.get::<String, _>("version"),
        "type": row.get::<String, _>("type_key"),
        "typeName": row.get::<String, _>("type_name"),
        "vendor": row.get::<Option<String>, _>("vendor"),
        "encrypted": row.get::<Option<bool>, _>("encrypted").unwrap_or(false),
        "locked": row.get::<bool, _>("locked"),
        "deleted": row.get::<Option<bool>, _>("deleted").unwrap_or(false),
        "_links": { "self": { "href": format!("{base}/rest/v1/softwaremodules/{id}") } }
    });
    put_audit(&mut value, row);
    value
}

/// Modules of one DS, as full SM JSON values.
pub async fn ds_modules(
    pool: &sqlx::PgPool,
    ds_id: i64,
    base: &str,
) -> crate::error::AppResult<Vec<Value>> {
    let rows = sqlx::query(&format!(
        "{SM_SELECT} JOIN sp_ds_sm l ON l.sm_id = m.id WHERE l.ds_id = $1 ORDER BY m.id"
    ))
    .bind(ds_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| sm_json(r, base)).collect())
}

/// Tag JSON (target tags and DS tags share the shape).
pub fn tag_json(row: &PgRow, base: &str, resource: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "colour": row.get::<Option<String>, _>("colour"),
        "_links": { "self": { "href": format!("{base}/rest/v1/{resource}/{id}") } }
    });
    put_audit(&mut value, row);
    value
}

/// Artifact JSON for mgmt (upload response / listing).
pub fn artifact_json(row: &PgRow, base: &str, sm_id: i64) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "hashes": {
            "sha1": row.get::<String, _>("sha1_hash"),
            "md5": row.get::<Option<String>, _>("md5_hash"),
            "sha256": row.get::<Option<String>, _>("sha256_hash").map(|s| s.trim().to_string()),
        },
        "providedFilename": row.get::<Option<String>, _>("provided_file_name"),
        "size": row.get::<Option<i64>, _>("file_size").unwrap_or(0),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/softwaremodules/{sm_id}/artifacts/{id}") },
            "download": { "href": format!("{base}/rest/v1/softwaremodules/{sm_id}/artifacts/{id}/download") }
        }
    });
    put_audit(&mut value, row);
    value
}
