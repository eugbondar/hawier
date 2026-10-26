//! `/rest/v1/actionlog` — what operators did in the console, read-only.
//!
//! hawier extension (recorded in API_NOTES). Writing stays on `/ui/v1/logs`
//! (the console records its own two-phase entries); reading lives here so
//! the log is an ordinary resource with views, sorting and paging.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{order_by, paged, rsql_filter, ListParams};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/actionlog", get(list))
        .route("/rest/v1/actionlog/{id}", get(get_one))
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "l.id::text",
                kind: Kind::Text,
            },
        ),
        (
            "username",
            Field::Column {
                sql: "l.username",
                kind: Kind::Text,
            },
        ),
        (
            "resource",
            Field::Column {
                sql: "l.resource",
                kind: Kind::Text,
            },
        ),
        (
            "resourceid",
            Field::Column {
                sql: "l.resource_id",
                kind: Kind::Text,
            },
        ),
        (
            "action",
            Field::Column {
                sql: "l.action",
                kind: Kind::Text,
            },
        ),
        (
            "status",
            Field::Column {
                sql: "l.status",
                kind: Kind::Text,
            },
        ),
        (
            "error",
            Field::Column {
                sql: "l.error",
                kind: Kind::Text,
            },
        ),
        (
            "requestsummary",
            Field::Column {
                sql: "l.request_summary",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM l.created_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

const SELECT: &str = "SELECT l.id, l.username, l.resource, l.resource_id, l.action,
       l.status, l.request_summary, l.response_summary, l.error,
       (EXTRACT(EPOCH FROM l.created_at) * 1000)::BIGINT AS created_ms,
       (EXTRACT(EPOCH FROM l.finished_at) * 1000)::BIGINT AS finished_ms
FROM hawier_action_log l";

fn log_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "username": row.get::<String, _>("username"),
        "resource": row.get::<String, _>("resource"),
        "resourceId": row.get::<String, _>("resource_id"),
        "action": row.get::<String, _>("action"),
        "status": row.get::<String, _>("status"),
        "requestSummary": row.get::<String, _>("request_summary"),
        "responseSummary": row.get::<String, _>("response_summary"),
        "error": row.get::<String, _>("error"),
        // Epoch millis, like every other timestamp on the wire.
        "createdAt": row.get::<Option<i64>, _>("created_ms"),
        "finishedAt": row.get::<Option<i64>, _>("finished_ms"),
    })
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "l.created_at DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_action_log l
             WHERE UPPER(l.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(l.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(rows.iter().map(log_json).collect(), total))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = sqlx::query(&format!(
        "{SELECT} WHERE l.id = $2 AND UPPER(l.tenant) = UPPER($1)"
    ))
    .bind(&identity.tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("action log entry {id} not found")))?;
    Ok(Json(log_json(&row)))
}
