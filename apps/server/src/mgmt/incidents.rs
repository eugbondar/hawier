//! `/rest/v1/incidents` — the server's incident journal, read-only.
//!
//! hawier extension (hawkBit has no such resource; recorded in API_NOTES).
//! It lives on the mgmt surface, not `/ui/v1`, so it goes through the same
//! list pipeline as every other resource: RSQL filters, sorting, paging,
//! tenant scoping from the caller's identity — and therefore the console's
//! ordinary views and user-saved views work on it for free.

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
        .route("/rest/v1/incidents", get(list))
        .route("/rest/v1/incidents/{id}", get(get_one))
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "incident",
            Field::Column {
                sql: "i.id",
                kind: Kind::Int,
            },
        ),
        (
            "kind",
            Field::Column {
                sql: "i.kind",
                kind: Kind::Text,
            },
        ),
        (
            "status",
            Field::Column {
                sql: "i.status",
                kind: Kind::Int,
            },
        ),
        (
            "method",
            Field::Column {
                sql: "i.method",
                kind: Kind::Text,
            },
        ),
        (
            "path",
            Field::Column {
                sql: "i.path",
                kind: Kind::Text,
            },
        ),
        (
            "errorcode",
            Field::Column {
                sql: "i.error_code",
                kind: Kind::Text,
            },
        ),
        (
            "message",
            Field::Column {
                sql: "i.message",
                kind: Kind::Text,
            },
        ),
        (
            "username",
            Field::Column {
                sql: "i.username",
                kind: Kind::Text,
            },
        ),
        (
            // Epoch millis, so the same filters work as on hawkBit tables.
            "occurredat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM i.occurred_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

const SELECT: &str = "SELECT i.id, i.kind, i.tenant, i.username, i.method, i.path,
       i.status, i.error_code, i.message
FROM hawier_incident i";

/// DDI incidents carry the device's tenant; everything else is recorded
/// without one, and is visible to any operator of the installation.
const TENANT_SCOPE: &str = "(i.tenant IS NULL OR UPPER(i.tenant) = UPPER($1))";

fn incident_json(row: &PgRow) -> Value {
    let id: i64 = row.get("id");
    let parts = crate::incident::decode(id);
    json!({
        // A string: 19-digit ids do not survive JS f64 JSON parsing.
        "incident": id.to_string(),
        "kind": row.get::<String, _>("kind"),
        "occurredAt": parts.timestamp_ms,
        "node": parts.node,
        "sequence": parts.sequence,
        "tenant": row.get::<Option<String>, _>("tenant"),
        "username": row.get::<Option<String>, _>("username"),
        "method": row.get::<String, _>("method"),
        "path": row.get::<String, _>("path"),
        "status": row.get::<i32, _>("status"),
        "errorCode": row.get::<String, _>("error_code"),
        "message": row.get::<String, _>("message"),
    })
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "i.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_incident i WHERE {TENANT_SCOPE}{filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE {TENANT_SCOPE}{filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(rows.iter().map(incident_json).collect(), total))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = sqlx::query(&format!("{SELECT} WHERE i.id = $2 AND {TENANT_SCOPE}"))
        .bind(&identity.tenant)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "incident {id} not found — it may have been trimmed by retention"
            ))
        })?;
    Ok(Json(incident_json(&row)))
}
