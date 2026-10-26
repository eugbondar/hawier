//! `/rest/v1/targetfilters` — saved RSQL filters + auto-assignment.
//!
//! Note the asymmetry hawier-ui depends on: create takes a BARE object
//! (unlike the array-create of targets/DS/SM).

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{
    body_str, order_by, paged, parse_action_type, put_audit, request_base, require_str,
    rsql_filter, ListParams,
};
use crate::auth::MgmtIdentity;
use crate::deploy_logic;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/targetfilters", get(list).post(create))
        .route(
            "/rest/v1/targetfilters/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route(
            "/rest/v1/targetfilters/{id}/autoAssignDS",
            get(get_auto_assign)
                .post(set_auto_assign)
                .delete(remove_auto_assign),
        )
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "f.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "f.name",
                kind: Kind::Text,
            },
        ),
        (
            "query",
            Field::Column {
                sql: "f.query",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "f.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "f.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            "autoassigndistributionset",
            Field::Column {
                sql: "f.auto_assign_distribution_set",
                kind: Kind::Int,
            },
        ),
        (
            "autoassignds.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = f.auto_assign_distribution_set AND ds.name $CMP $V)",
            },
        ),
    ])
});

const SELECT: &str = "SELECT f.id, f.created_at, f.created_by, f.last_modified_at,
       f.last_modified_by, f.name, f.query, f.auto_assign_distribution_set,
       f.auto_assign_action_type, f.auto_assign_weight, f.confirmation_required,
       ads.name || ' ' || ads.version AS auto_assign_ds_label
FROM sp_target_filter_query f
LEFT JOIN sp_distribution_set ads ON ads.id = f.auto_assign_distribution_set";

fn filter_json(row: &PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "query": row.get::<String, _>("query"),
        "autoAssignDistributionSet": row.get::<Option<i64>, _>("auto_assign_distribution_set"),
        // hawier extension (API_NOTES §5): hawkBit answers with a bare id, and
        // a list column holding a number nobody can read is a dead end. The
        // key is always present — `null` when nothing is auto-assigned — so a
        // console column declared on it never renders as "empty".
        "autoAssignDistributionSetName": row.get::<Option<String>, _>("auto_assign_ds_label"),
        "autoAssignActionType": row
            .get::<Option<i32>, _>("auto_assign_action_type")
            .map(super::action_type_name),
        "autoAssignWeight": row.get::<i32, _>("auto_assign_weight"),
        "confirmationRequired": row.get::<Option<bool>, _>("confirmation_required"),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/targetfilters/{id}") },
            "autoAssignDS": { "href": format!("{base}/rest/v1/targetfilters/{id}/autoAssignDS") }
        }
    });
    put_audit(&mut value, row);
    value
}

async fn load_filter(state: &AppState, tenant: &str, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{SELECT} WHERE f.id = $1 AND UPPER(f.tenant) = UPPER($2)"
    ))
    .bind(id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("target filter {id} not found")))
}

/// Validate a filter query against the target field map before storing it —
/// a broken saved filter would wedge auto-assignment later.
fn validate_target_query(query: &str) -> AppResult<()> {
    super::targets::validate_rsql(query)
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "f.id ASC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_target_filter_query f
             WHERE UPPER(f.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(f.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(
        rows.iter().map(|r| filter_json(r, &base)).collect(),
        total,
    ))
}

/// Bare-object create (hawkBit takes no array here).
async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = require_str(&body, "name")?;
    let query = require_str(&body, "query")?;
    validate_target_query(&query)?;

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_target_filter_query
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, query, auto_assign_weight)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6)
         RETURNING id",
    )
    .bind(now_millis())
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(&name)
    .bind(&query)
    .bind(deploy_logic::DEFAULT_WEIGHT)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Conflict(format!("target filter '{name}' already exists"))
        }
        _ => AppError::Db(e),
    })?;

    let row = load_filter(&state, &identity.tenant, id).await?;
    Ok((
        StatusCode::CREATED,
        Json(filter_json(&row, &request_base(&headers))),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_filter(&state, &identity.tenant, id).await?;
    Ok(Json(filter_json(&row, &request_base(&headers))))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_filter(&state, &identity.tenant, id).await?;
    if let Some(query) = body_str(&body, "query") {
        validate_target_query(&query)?;
    }
    sqlx::query(
        "UPDATE sp_target_filter_query SET
             name = COALESCE($1, name),
             query = COALESCE($2, query),
             last_modified_at = $3, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $5",
    )
    .bind(body_str(&body, "name"))
    .bind(body_str(&body, "query"))
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&state.pool)
    .await?;
    let row = load_filter(&state, &identity.tenant, id).await?;
    Ok(Json(filter_json(&row, &request_base(&headers))))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    load_filter(&state, &identity.tenant, id).await?;
    sqlx::query("DELETE FROM sp_target_filter_query WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// autoAssignDS
// ---------------------------------------------------------------------------

async fn get_auto_assign(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let row = load_filter(&state, &identity.tenant, id).await?;
    let Some(ds_id) = row.get::<Option<i64>, _>("auto_assign_distribution_set") else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let base = request_base(&headers);
    let ds_row = sqlx::query(&format!(
        "{} WHERE d.id = $1 AND UPPER(d.tenant) = UPPER($2)",
        super::repr::DS_SELECT
    ))
    .bind(ds_id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("auto-assign DS vanished".into()))?;
    let modules = super::repr::ds_modules(&state.pool, ds_id, &base).await?;
    Ok(Json(super::repr::ds_json(&ds_row, &base, modules)).into_response())
}

async fn set_auto_assign(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_filter(&state, &identity.tenant, id).await?;
    let ds_id = body
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::BadRequest("'id' (distribution set) is required".into()))?;
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE",
    )
    .bind(ds_id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "distribution set {ds_id} not found"
        )));
    }

    let action_type = match body_str(&body, "type") {
        Some(t) => Some(parse_action_type(&t)?.code()),
        None => Some(hawier_api::codes::ActionType::Forced.code()),
    };
    let weight = body
        .get("weight")
        .and_then(|v| v.as_i64())
        .map(|w| w as i32)
        .unwrap_or(deploy_logic::DEFAULT_WEIGHT);

    sqlx::query(
        "UPDATE sp_target_filter_query SET
             auto_assign_distribution_set = $1,
             auto_assign_action_type = $2,
             auto_assign_weight = $3,
             auto_assign_initiated_by = $4,
             confirmation_required = $5,
             last_modified_at = $6, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $7",
    )
    .bind(ds_id)
    .bind(action_type)
    .bind(weight)
    .bind(&identity.username)
    .bind(body.get("confirmationRequired").and_then(|v| v.as_bool()))
    .bind(now_millis())
    .bind(id)
    .execute(&state.pool)
    .await?;

    let row = load_filter(&state, &identity.tenant, id).await?;
    Ok(Json(filter_json(&row, &request_base(&headers))))
}

async fn remove_auto_assign(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    load_filter(&state, &identity.tenant, id).await?;
    sqlx::query(
        "UPDATE sp_target_filter_query SET
             auto_assign_distribution_set = NULL,
             auto_assign_action_type = NULL,
             auto_assign_initiated_by = NULL,
             last_modified_at = $1,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $2",
    )
    .bind(now_millis())
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
