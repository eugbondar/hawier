//! `/rest/v1/actions` — tenant-wide action list/get/delete.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use hawier_api::codes::ActionStatus;
use serde_json::{json, Value};
use sqlx::Row;

use super::{
    action_detail_status, action_type_name, order_by, paged, put_audit, request_base, rsql_filter,
    ListParams,
};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/actions", get(list))
        .route("/rest/v1/actions/{id}", get(get_one).delete(delete_one))
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "a.id",
                kind: Kind::Int,
            },
        ),
        (
            "weight",
            Field::Column {
                sql: "a.weight",
                kind: Kind::Int,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "a.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "a.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            "externalref",
            Field::Column {
                sql: "a.external_ref",
                kind: Kind::Text,
            },
        ),
        (
            "rollout",
            Field::Column {
                sql: "a.rollout",
                kind: Kind::Int,
            },
        ),
        (
            "rollout.id",
            Field::Column {
                sql: "a.rollout",
                kind: Kind::Int,
            },
        ),
        (
            // hawkBit's ActionFields.ROLLOUTGROUP — the console filters a
            // deploy group's actions with it.
            "rolloutgroup",
            Field::Column {
                sql: "a.rollout_group",
                kind: Kind::Int,
            },
        ),
        (
            "rolloutgroup.id",
            Field::Column {
                sql: "a.rollout_group",
                kind: Kind::Int,
            },
        ),
        (
            "target",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            // Every column the console declares has to be sortable: a header
            // that answers 400 replaces the list with an error callout.
            "controllerid",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            "rolloutname",
            Field::Column {
                sql: "r.name",
                kind: Kind::Text,
            },
        ),
        (
            // The JSON `type` is derived from the status, so sort by the same
            // derivation rather than by the raw code. The two codes come from
            // libs/api (never hand-written into SQL) and the expression is
            // built once, so it lives as long as the map itself.
            "type",
            Field::EnumColumn {
                sql: Box::leak(
                    format!(
                        "CASE WHEN a.status IN ({}, {}) THEN 1 ELSE 0 END",
                        ActionStatus::Canceled.code(),
                        ActionStatus::Canceling.code(),
                    )
                    .into_boxed_str(),
                ),
                map: &[("update", 0), ("cancel", 1)],
            },
        ),
        (
            "target.controllerid",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            "detailstatus",
            Field::EnumColumn {
                sql: "a.status",
                map: &[
                    ("finished", 0),
                    ("error", 1),
                    ("warning", 2),
                    ("running", 3),
                    ("canceled", 4),
                    ("canceling", 5),
                    ("retrieved", 6),
                    ("download", 7),
                    ("scheduled", 8),
                    ("cancel_rejected", 9),
                    ("downloaded", 10),
                    ("wait_for_confirmation", 11),
                ],
            },
        ),
        (
            "status",
            Field::EnumColumn {
                sql: "CASE WHEN a.active THEN 1 ELSE 0 END",
                map: &[("pending", 1), ("finished", 0)],
            },
        ),
        (
            "distributionset.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = a.distribution_set AND ds.name $CMP $V)",
            },
        ),
    ])
});

const SELECT: &str = "SELECT a.id, a.created_at, a.created_by, a.last_modified_at,
       a.last_modified_by, a.active, a.status, a.action_type, a.forced_time,
       a.distribution_set, a.target, a.rollout, a.rollout_group, a.weight,
       a.initiated_by, a.external_ref, a.last_action_status_code,
       t.controller_id AS controller_id, r.name AS rollout_name,
       ds.name AS ds_name, ds.version AS ds_version
FROM sp_action a
JOIN sp_target t ON t.id = a.target
LEFT JOIN sp_rollout r ON r.id = a.rollout
LEFT JOIN sp_distribution_set ds ON ds.id = a.distribution_set";

fn action_json(row: &sqlx::postgres::PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let controller_id: String = row.get("controller_id");
    let active = row.get::<Option<bool>, _>("active").unwrap_or(false);
    let status_code: i32 = row.get("status");
    let is_cancel = status_code == ActionStatus::Canceling.code()
        || status_code == hawier_api::codes::ActionStatus::Canceled.code();
    let mut value = json!({
        "id": id,
        "type": if is_cancel { "cancel" } else { "update" },
        "status": if active { "pending" } else { "finished" },
        "detailStatus": action_detail_status(status_code),
        "forceType": action_type_name(row.get::<i32, _>("action_type")),
        "weight": row.get::<i32, _>("weight"),
        // Which device and which rollout group — both already selected,
        // and a console list of actions is useless without them.
        "controllerId": controller_id,
        "rollout": row.get::<Option<i64>, _>("rollout"),
        "rolloutName": row.get::<Option<String>, _>("rollout_name"),
        "rolloutGroup": row.get::<Option<i64>, _>("rollout_group"),
        "externalRef": row.get::<Option<String>, _>("external_ref"),
        "lastStatusCode": row.get::<Option<i32>, _>("last_action_status_code"),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/actions/{id}") },
            "target": { "href": format!("{base}/rest/v1/targets/{controller_id}") },
            "distributionset": super::ds_link(row, base),
            "status": {
                "href": format!("{base}/rest/v1/targets/{controller_id}/actions/{id}/status")
            }
        }
    });
    put_audit(&mut value, row);
    value
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "a.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_action a JOIN sp_target t ON t.id = a.target
             WHERE UPPER(a.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(a.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(
        rows.iter().map(|r| action_json(r, &base)).collect(),
        total,
    ))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = sqlx::query(&format!(
        "{SELECT} WHERE a.id = $1 AND UPPER(a.tenant) = UPPER($2)"
    ))
    .bind(id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("action {id} not found")))?;
    Ok(Json(action_json(&row, &request_base(&headers))))
}

/// Delete a closed action (hawkBit refuses to delete active ones).
async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let active: Option<bool> = sqlx::query_scalar(
        "SELECT active FROM sp_action WHERE id = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    match active {
        None => Err(AppError::NotFound(format!("action {id} not found"))),
        Some(true) => Err(AppError::Conflict(format!(
            "action {id} is still active and cannot be deleted"
        ))),
        Some(false) => {
            sqlx::query("DELETE FROM sp_action WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await?;
            Ok(StatusCode::NO_CONTENT)
        }
    }
}
