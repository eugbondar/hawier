//! Management API (`/rest/v1`), hawkBit-compatible.
//!
//! Conventions shared by all resource modules:
//! - list endpoints take `offset` (default 0), `limit` (default 50, max 500),
//!   `sort` (`field:ASC,field2:DESC`), `q` (RSQL, see [`crate::rsql`]) and
//!   `representation` (`compact` | `full`);
//! - responses are built as `serde_json::json!` values with hawkBit's exact
//!   field names — the wire format is the contract, not Rust types;
//! - list envelope: `{"content": [...], "total": N, "size": M}`;
//! - create endpoints for targets/DS/SM/tags/types accept a JSON ARRAY and
//!   answer with an array (hawier-ui depends on this).

pub mod action_log;
pub mod actions;
pub mod distribution_sets;
pub mod extensions;
pub mod incidents;
pub mod repr;
pub mod retention;
pub mod roles;
pub mod rollouts;
pub mod software_modules;
pub mod system;
pub mod target_filters;
pub mod target_groups;
pub mod targets;
pub mod types_tags;
pub mod userinfo;
pub mod webhooks;

use axum::http::HeaderMap;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::rsql::{self, FieldMap};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/userinfo", get(userinfo::get_userinfo))
        .merge(targets::router())
        .merge(retention::router())
        .merge(extensions::router())
        .merge(roles::router())
        .merge(webhooks::router())
        .merge(types_tags::router())
        .merge(distribution_sets::router())
        .merge(software_modules::router())
        .merge(actions::router())
        .merge(rollouts::router())
        .merge(target_filters::router())
        .merge(target_groups::router())
        .merge(system::router())
        .merge(incidents::router())
        .merge(action_log::router())
}

pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 500;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub offset: i64,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    pub q: Option<String>,
    pub representation: Option<String>,
}

impl ListParams {
    pub fn limit(&self) -> AppResult<i64> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if limit < 1 {
            return Err(AppError::BadRequest("limit must be positive".into()));
        }
        if limit > MAX_LIMIT {
            return Err(AppError::BadRequest(format!(
                "limit must not exceed {MAX_LIMIT}"
            )));
        }
        Ok(limit)
    }

    pub fn offset(&self) -> AppResult<i64> {
        if self.offset < 0 {
            return Err(AppError::BadRequest("offset must not be negative".into()));
        }
        Ok(self.offset)
    }

    pub fn full(&self) -> bool {
        self.representation.as_deref() == Some("full")
    }
}

/// `{"content": [...], "total": N, "size": M}`.
pub fn paged(content: Vec<Value>, total: i64) -> Json<Value> {
    let size = content.len();
    Json(json!({ "content": content, "total": total, "size": size }))
}

/// Translate `sort=name:ASC,id:DESC` into an ORDER BY over mapped columns.
/// Only plain columns are sortable; unknown fields → 400.
pub fn order_by(sort: Option<&str>, map: &FieldMap, default: &str) -> AppResult<String> {
    order_by_with(sort, map, default, &[])
}

/// Like [`order_by`], with extra sort-only aliases (lowercased name → SQL,
/// e.g. an output-column alias for a computed value like DS `complete`).
pub fn order_by_with(
    sort: Option<&str>,
    map: &FieldMap,
    default: &str,
    extra: &[(&str, &str)],
) -> AppResult<String> {
    let Some(sort) = sort.filter(|s| !s.trim().is_empty()) else {
        return Ok(format!("ORDER BY {default}"));
    };
    let mut parts = Vec::new();
    for entry in sort.split(',') {
        let (field, direction) = entry.split_once(':').unwrap_or((entry, "ASC"));
        let direction = match direction.to_ascii_uppercase().as_str() {
            "ASC" => "ASC",
            "DESC" => "DESC",
            other => {
                return Err(AppError::BadRequest(format!(
                    "invalid sort direction '{other}'"
                )))
            }
        };
        let key = field.trim().to_lowercase();
        if let Some((_, sql)) = extra.iter().find(|(alias, _)| *alias == key) {
            parts.push(format!("{sql} {direction}"));
            continue;
        }
        match map.fields.get(key.as_str()) {
            Some(rsql::Field::Column { sql, .. }) | Some(rsql::Field::EnumColumn { sql, .. }) => {
                parts.push(format!("{sql} {direction}"))
            }
            _ => return Err(AppError::BadRequest(format!("cannot sort by '{field}'"))),
        }
    }
    Ok(format!("ORDER BY {}", parts.join(", ")))
}

/// Build the optional RSQL WHERE fragment (`AND (...)`) for a list query.
pub fn rsql_filter(
    q: Option<&str>,
    map: &FieldMap,
    bind_offset: usize,
) -> AppResult<(String, Vec<rsql::Value>)> {
    match q.filter(|q| !q.trim().is_empty()) {
        Some(q) => {
            let clause = rsql::to_sql(q, map, bind_offset)?;
            Ok((format!(" AND {}", clause.sql), clause.binds))
        }
        None => Ok((String::new(), Vec::new())),
    }
}

/// External base URL for `_links` (mirrors DDI's helper).
pub fn request_base(headers: &HeaderMap) -> String {
    crate::ddi::request_base(headers)
}

/// Merge hawkBit's audit block into a JSON object.
pub fn put_audit(value: &mut Value, row: &sqlx::postgres::PgRow) {
    use sqlx::Row;
    let object = value
        .as_object_mut()
        .expect("audit target must be an object");
    if let Ok(v) = row.try_get::<Option<String>, _>("created_by") {
        object.insert("createdBy".into(), json!(v));
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>("created_at") {
        object.insert("createdAt".into(), json!(v));
    }
    if let Ok(v) = row.try_get::<Option<String>, _>("last_modified_by") {
        object.insert("lastModifiedBy".into(), json!(v));
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>("last_modified_at") {
        object.insert("lastModifiedAt".into(), json!(v));
    }
}

/// `update_status` int → hawkBit's lowercase wire name.
pub fn update_status_name(code: i32) -> &'static str {
    match code {
        0 => "unknown",
        1 => "in_sync",
        2 => "pending",
        3 => "error",
        4 => "registered",
        _ => "unknown",
    }
}

/// The `_links.distributionset` object of an action.
///
/// hawkBit puts `name` next to `href`, and the console has no other source
/// for it: while the link carried an href alone, every action page showed an
/// empty "Distribution set" row. Two endpoints serve actions (the flat list
/// and the target-scoped one) and both must say the same thing, hence one
/// helper — `ds_name`/`ds_version` have to be selected by the caller.
pub fn ds_link(row: &sqlx::postgres::PgRow, base: &str) -> serde_json::Value {
    use sqlx::Row as _;
    let id: i64 = row.get("distribution_set");
    let name: Option<String> = row.try_get("ds_name").ok().flatten();
    let version: Option<String> = row.try_get("ds_version").ok().flatten();
    // A set is "name:version" — the name alone is not unique in hawkBit.
    let label = match (name, version) {
        (Some(name), Some(version)) => Some(format!("{name}:{version}")),
        (name, _) => name,
    };
    serde_json::json!({
        "href": format!("{base}/rest/v1/distributionsets/{id}"),
        "id": id,
        "name": label,
    })
}

/// `Action.Status` int → mgmt `detailStatus` / action-status `type` wire
/// value (lowercase Java enum name).
pub fn action_detail_status(code: i32) -> String {
    hawier_api::codes::ActionStatus::try_from(code)
        .map(|s| s.name().to_lowercase())
        .unwrap_or_else(|_| code.to_string())
}

/// ActionType int → mgmt wire value.
pub fn action_type_name(code: i32) -> &'static str {
    match code {
        0 => "forced",
        1 => "soft",
        2 => "timeforced",
        3 => "downloadonly",
        _ => "forced",
    }
}

pub fn parse_action_type(value: &str) -> AppResult<hawier_api::codes::ActionType> {
    use hawier_api::codes::ActionType;
    match value.to_ascii_lowercase().as_str() {
        "forced" => Ok(ActionType::Forced),
        "soft" => Ok(ActionType::Soft),
        "timeforced" => Ok(ActionType::TimeForced),
        "downloadonly" => Ok(ActionType::DownloadOnly),
        other => Err(AppError::BadRequest(format!(
            "unknown action type '{other}'"
        ))),
    }
}

/// Accept either a JSON array of objects or a single object; return a Vec
/// plus whether the input was an array (response mirrors request shape).
pub fn one_or_many(body: Value) -> AppResult<(Vec<Value>, bool)> {
    match body {
        Value::Array(items) => Ok((items, true)),
        Value::Object(_) => Ok((vec![body], false)),
        _ => Err(AppError::BadRequest(
            "expected a JSON object or array".into(),
        )),
    }
}

pub fn body_str(item: &Value, key: &str) -> Option<String> {
    item.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

pub fn require_str(item: &Value, key: &str) -> AppResult<String> {
    body_str(item, key)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("'{key}' is required")))
}
