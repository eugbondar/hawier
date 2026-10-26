//! `/rest/v1/retentionrules` — automatic deletion rules.
//!
//! hawier extension (recorded in API_NOTES). An ordinary resource, so the
//! console gets sorting, paging and saved views for free (UI/UX rule 11).
//!
//! Everything that can be checked on write is checked on write: the cron has
//! to parse, the filter has to name fields that exist, and the scope has to
//! be one something implements. A rule that deletes things is the last place
//! to discover a typo at three in the morning.

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
        .route("/rest/v1/retentionrules", get(list).post(create))
        .route(
            "/rest/v1/retentionrules/{id}",
            get(get_one).put(update).delete(delete_one),
        )
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "r.id::text",
                kind: Kind::Text,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "r.name",
                kind: Kind::Text,
            },
        ),
        (
            "scope",
            Field::Column {
                sql: "r.scope",
                kind: Kind::Text,
            },
        ),
        (
            "cron",
            Field::Column {
                sql: "r.cron",
                kind: Kind::Text,
            },
        ),
        (
            "filter",
            Field::Column {
                sql: "r.filter_rsql",
                kind: Kind::Text,
            },
        ),
        (
            "minagems",
            Field::Column {
                sql: "r.min_age_ms",
                kind: Kind::Int,
            },
        ),
        (
            "dryrun",
            Field::Column {
                sql: "r.dry_run",
                kind: Kind::Bool,
            },
        ),
        (
            "active",
            Field::Column {
                sql: "r.active",
                kind: Kind::Bool,
            },
        ),
        (
            "lastresult",
            Field::Column {
                sql: "r.last_result",
                kind: Kind::Text,
            },
        ),
        (
            "lastrunat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM r.last_run_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

const SELECT: &str = "SELECT r.id, r.name, r.scope, r.filter_rsql, r.min_age_ms, r.cron,
       r.params, r.dry_run, r.active, r.last_result,
       (EXTRACT(EPOCH FROM r.last_run_at) * 1000)::BIGINT AS last_run_ms,
       (EXTRACT(EPOCH FROM r.created_at) * 1000)::BIGINT AS created_ms
FROM hawier_retention_rule r";

/// Scopes something actually implements. A rule with any other scope would
/// sit there looking active and never fire.
const SCOPES: [&str; 1] = ["artifact"];

fn rule_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "name": row.get::<String, _>("name"),
        "scope": row.get::<String, _>("scope"),
        "filter": row.get::<String, _>("filter_rsql"),
        "minAgeMs": row.get::<i64, _>("min_age_ms"),
        "cron": row.get::<String, _>("cron"),
        "params": row.get::<Value, _>("params"),
        "dryRun": row.get::<bool, _>("dry_run"),
        "active": row.get::<bool, _>("active"),
        "lastResult": row.get::<String, _>("last_result"),
        "lastRunAt": row.get::<Option<i64>, _>("last_run_ms"),
        "createdAt": row.get::<Option<i64>, _>("created_ms"),
    })
}

/// Reject on write what would otherwise fail silently at three in the
/// morning: an unparseable schedule, a filter naming a field that does not
/// exist, or a scope nothing implements.
fn validate(scope: &str, cron: &str, filter: &str) -> AppResult<()> {
    if !SCOPES.contains(&scope) {
        return Err(AppError::BadRequest(format!(
            "unknown scope '{scope}'; known scopes are {}",
            SCOPES.join(", ")
        )));
    }
    crate::maintenance::parse_schedule(cron)?;
    if !filter.trim().is_empty() {
        // Same parse the rule will do when it fires, so a bad filter is a 400
        // now rather than a warning in a log later.
        crate::rsql::to_sql(filter, crate::retention::candidate_fields(), 1)?;
    }
    Ok(())
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "r.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_retention_rule r
             WHERE UPPER(r.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(r.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(rows.iter().map(rule_json).collect(), total))
}

async fn load(state: &AppState, tenant: &str, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{SELECT} WHERE r.id = $2 AND UPPER(r.tenant) = UPPER($1)"
    ))
    .bind(tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("retention rule {id} not found")))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    Ok(Json(rule_json(&load(&state, &identity.tenant, id).await?)))
}

async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Value>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("'name' is required".to_string()))?
        .to_string();
    let cron = body
        .get("cron")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("'cron' is required".to_string()))?
        .to_string();
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("artifact")
        .to_string();
    let filter = body
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    validate(&scope, &cron, &filter)?;

    let id = state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_retention_rule
         (id, tenant, name, scope, filter_rsql, min_age_ms, cron, params, dry_run, active)
         VALUES ($1, UPPER($2), $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .bind(&name)
    .bind(&scope)
    .bind(&filter)
    .bind(body.get("minAgeMs").and_then(|v| v.as_i64()).unwrap_or(0))
    .bind(&cron)
    .bind(body.get("params").cloned().unwrap_or_else(|| json!({})))
    // Deleting automation starts out only saying what it would do.
    .bind(body.get("dryRun").and_then(|v| v.as_bool()).unwrap_or(true))
    .bind(body.get("active").and_then(|v| v.as_bool()).unwrap_or(true))
    .execute(&state.pool)
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(rule_json(&load(&state, &identity.tenant, id).await?)),
    ))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let current = load(&state, &identity.tenant, id).await?;
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or(current.get("scope"));
    let cron = body
        .get("cron")
        .and_then(|v| v.as_str())
        .unwrap_or(current.get("cron"));
    let filter = body
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or(current.get("filter_rsql"));
    validate(scope, cron, filter)?;

    sqlx::query(
        "UPDATE hawier_retention_rule SET
            name = COALESCE($3, name),
            scope = $4, cron = $5, filter_rsql = $6,
            min_age_ms = COALESCE($7, min_age_ms),
            params = COALESCE($8, params),
            dry_run = COALESCE($9, dry_run),
            active = COALESCE($10, active),
            updated_at = now()
         WHERE id = $2 AND UPPER(tenant) = UPPER($1)",
    )
    .bind(&identity.tenant)
    .bind(id)
    .bind(body.get("name").and_then(|v| v.as_str()))
    .bind(scope)
    .bind(cron)
    .bind(filter)
    .bind(body.get("minAgeMs").and_then(|v| v.as_i64()))
    .bind(body.get("params").cloned())
    .bind(body.get("dryRun").and_then(|v| v.as_bool()))
    .bind(body.get("active").and_then(|v| v.as_bool()))
    .execute(&state.pool)
    .await?;

    Ok(Json(rule_json(&load(&state, &identity.tenant, id).await?)))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<axum::http::StatusCode> {
    let deleted = sqlx::query(
        "DELETE FROM hawier_retention_rule WHERE id = $2 AND UPPER(tenant) = UPPER($1)",
    )
    .bind(&identity.tenant)
    .bind(id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound(format!("retention rule {id} not found")));
    }
    Ok(axum::http::StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_rule_passes() {
        validate("artifact", "0 0 3 * * *", "module.version==*-dev").unwrap();
    }

    #[test]
    fn an_unparseable_cron_is_refused() {
        assert!(validate("artifact", "not a cron", "").is_err());
    }

    #[test]
    fn a_filter_naming_an_unknown_field_is_refused() {
        // Otherwise the rule looks active and every run logs a warning.
        assert!(validate("artifact", "0 0 3 * * *", "module.colour==red").is_err());
    }

    #[test]
    fn an_unimplemented_scope_is_refused() {
        assert!(validate("targets", "0 0 3 * * *", "").is_err());
    }
}
