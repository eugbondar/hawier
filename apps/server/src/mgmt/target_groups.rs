//! `/rest/v1/targetgroups` — free-text hierarchical target grouping.
//!
//! NARROW: groups live in `sp_target.target_group VARCHAR(256)` as a
//! free-text path ("a/b/c") — hawkBit's third overlapping grouping
//! mechanism (SCHEMA_NOTES.md #13).

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use super::{paged, request_base, rsql_filter, update_status_name, ListParams};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::bind_values;
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/rest/v1/targetgroups",
            get(list_groups).put(assign_by_rsql_query),
        )
        .route(
            "/rest/v1/targetgroups/assigned",
            get(list_assigned).put(assign_ids).delete(unassign_ids),
        )
        .route(
            "/rest/v1/targetgroups/{group}/assigned",
            get(group_targets).put(assign_ids_to),
        )
        // hawier extension (API_NOTES §5). hawkBit's own `/targetgroups`
        // answers with a bare array of strings — no total, no filter, no
        // sort — so a console list built on it would show a pager that lies
        // and a search box that does nothing. This resource is the ordinary
        // shape: `{name, targets}` rows through ListParams/rsql/order_by/paged.
        // The hawkBit path above is left exactly as it is.
        .route("/rest/v1/targetgroupdetails", get(list_group_details))
        .route("/rest/v1/targetgroups/{group}", put(assign_by_rsql_to))
        .route("/rest/v1/targetgroups", delete(unassign_by_rsql))
}

/// Only the name is filterable: the row count is an aggregate, and an
/// aggregate in WHERE is a SQL error, not a 400. It stays a sort-only alias.
static GROUP_FIELDS: std::sync::LazyLock<crate::rsql::FieldMap> = std::sync::LazyLock::new(|| {
    crate::rsql::FieldMap::new(vec![(
        "name",
        crate::rsql::Field::Column {
            sql: "t.target_group",
            kind: crate::rsql::Kind::Text,
        },
    )])
});

/// GET /targetgroupdetails → one row per distinct group with its size.
///
/// NARROW: `SELECT DISTINCT target_group` scans `sp_target` — there is an
/// index on `(tenant, target_group)` from the hawkBit baseline, which covers
/// the grouping, but the count is still per-row work. Fine for a console
/// list, never for the poll path.
async fn list_group_details(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    // The filter applies to the group name, so it belongs in WHERE (before
    // grouping); the count is a sort-only alias (see GROUP_FIELDS).
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &GROUP_FIELDS, 1)?;
    let order = super::order_by_with(
        params.sort.as_deref(),
        &GROUP_FIELDS,
        "t.target_group ASC",
        &[("targets", "COUNT(*)")],
    )?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM (
                 SELECT t.target_group FROM sp_target t
                 WHERE UPPER(t.tenant) = UPPER($1) AND t.target_group IS NOT NULL{filter_sql}
                 GROUP BY t.target_group) g"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "SELECT t.target_group AS name, COUNT(*) AS targets FROM sp_target t
             WHERE UPPER(t.tenant) = UPPER($1) AND t.target_group IS NOT NULL{filter_sql}
             GROUP BY t.target_group {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let content: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "name": row.get::<String, _>("name"),
                "targets": row.get::<i64, _>("targets"),
            })
        })
        .collect();
    Ok(paged(content, total))
}

/// GET /targetgroups → plain list of distinct group names.
async fn list_groups(
    State(state): State<AppState>,
    identity: MgmtIdentity,
) -> AppResult<Json<Value>> {
    let groups: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT target_group FROM sp_target
         WHERE UPPER(tenant) = UPPER($1) AND target_group IS NOT NULL
         ORDER BY target_group",
    )
    .bind(&identity.tenant)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!(groups)))
}

fn target_brief(row: &sqlx::postgres::PgRow, base: &str) -> Value {
    let controller_id: String = row.get("controller_id");
    json!({
        "controllerId": controller_id,
        "name": row.get::<String, _>("name"),
        "group": row.get::<Option<String>, _>("target_group"),
        "updateStatus": update_status_name(row.get::<i32, _>("update_status")),
        "_links": { "self": { "href": format!("{base}/rest/v1/targets/{controller_id}") } }
    })
}

async fn targets_of_group_pattern(
    state: &AppState,
    identity: &MgmtIdentity,
    group: &str,
    subgroups: bool,
    params: &ListParams,
    base: &str,
) -> AppResult<Json<Value>> {
    // Exact group, or the group with its whole subtree (`group/...`).
    let clause = if subgroups {
        "(t.target_group = $2 OR t.target_group LIKE $2 || '/%')"
    } else {
        "t.target_group = $2"
    };
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM sp_target t WHERE UPPER(t.tenant) = UPPER($1) AND {clause}"
    ))
    .bind(&identity.tenant)
    .bind(group)
    .fetch_one(&state.pool)
    .await?;

    let rows = sqlx::query(&format!(
        "SELECT t.controller_id, t.name, t.target_group, t.update_status
         FROM sp_target t WHERE UPPER(t.tenant) = UPPER($1) AND {clause}
         ORDER BY t.id LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    ))
    .bind(&identity.tenant)
    .bind(group)
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(
        rows.iter().map(|r| target_brief(r, base)).collect(),
        total,
    ))
}

async fn group_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(group): Path<String>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    targets_of_group_pattern(
        &state,
        &identity,
        &group,
        false,
        &params,
        &request_base(&headers),
    )
    .await
}

#[derive(Deserialize)]
struct AssignedQuery {
    #[serde(rename = "groupFilter")]
    group_filter: Option<String>,
    group: Option<String>,
    #[serde(flatten)]
    list: ListParams,
}

/// GET /targetgroups/assigned?groupFilter=X — group incl. subgroups.
async fn list_assigned(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(query): Query<AssignedQuery>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let group = query
        .group_filter
        .or(query.group)
        .ok_or_else(|| AppError::BadRequest("groupFilter is required".into()))?;
    targets_of_group_pattern(
        &state,
        &identity,
        &group,
        true,
        &query.list,
        &request_base(&headers),
    )
    .await
}

async fn set_group(
    state: &AppState,
    identity: &MgmtIdentity,
    group: Option<&str>,
    controller_ids: &[String],
) -> AppResult<()> {
    for controller_id in controller_ids {
        let updated = sqlx::query(
            "UPDATE sp_target SET target_group = $1, last_modified_at = $2,
                 last_modified_by = $3, optlock_revision = COALESCE(optlock_revision, 0) + 1
             WHERE controller_id = $4 AND UPPER(tenant) = UPPER($5)",
        )
        .bind(group)
        .bind(now_millis())
        .bind(&identity.username)
        .bind(controller_id)
        .bind(&identity.tenant)
        .execute(&state.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(AppError::NotFound(format!(
                "target {controller_id} not found"
            )));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct GroupParam {
    group: String,
}

/// PUT /targetgroups/assigned?group=X  body: ["id1", "id2"].
async fn assign_ids(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(param): Query<GroupParam>,
    Json(body): Json<Vec<String>>,
) -> AppResult<StatusCode> {
    set_group(&state, &identity, Some(&param.group), &body).await?;
    Ok(StatusCode::OK)
}

/// PUT /targetgroups/{group}/assigned  body: ["id1", "id2"].
async fn assign_ids_to(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(group): Path<String>,
    Json(body): Json<Vec<String>>,
) -> AppResult<StatusCode> {
    set_group(&state, &identity, Some(&group), &body).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct RsqlParam {
    rsql: Option<String>,
    #[serde(rename = "rsqlParam")]
    rsql_param: Option<String>,
    group: Option<String>,
}

impl RsqlParam {
    fn rsql(&self) -> AppResult<&str> {
        self.rsql
            .as_deref()
            .or(self.rsql_param.as_deref())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("rsql parameter is required".into()))
    }
}

async fn assign_group_by_rsql(
    state: &AppState,
    identity: &MgmtIdentity,
    group: Option<&str>,
    rsql: &str,
) -> AppResult<u64> {
    let (filter_sql, binds) = rsql_filter(Some(rsql), super::targets::target_fields(), 3)?;
    let sql = format!(
        "UPDATE sp_target SET target_group = $1,
             last_modified_at = $2, last_modified_by = $3,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id IN (SELECT t.id FROM sp_target t
                      WHERE UPPER(t.tenant) = UPPER($4){filter_sql})"
    );
    // Binds: $1 group, $2 now, $3 user, $4 tenant, then RSQL binds from $4+1.
    // rsql_filter was given offset 3 → placeholders start at $4; shift by
    // renumbering: our WHERE uses $4 for tenant so rsql binds start at $5.
    let sql = renumber_rsql_placeholders(&sql, 4, binds.len());
    let result = bind_values(
        sqlx::query(&sql)
            .bind(group)
            .bind(now_millis())
            .bind(&identity.username)
            .bind(&identity.tenant),
        &binds,
    )
    .execute(&state.pool)
    .await?;
    Ok(result.rows_affected())
}

/// rsql_filter produced $4..$N (offset 3); the tenant already claimed $4 in
/// the outer statement, so shift every rsql placeholder up by one.
fn renumber_rsql_placeholders(sql: &str, from: usize, count: usize) -> String {
    let mut out = sql.to_string();
    // Replace from the highest number down so $4 → $5 doesn't collide.
    for n in (from..from + count).rev() {
        out = out.replace(&format!("${n}"), &format!("${}", n + 1));
    }
    out
}

/// PUT /targetgroups?group=X&rsqlParam=... — assign group by filter.
async fn assign_by_rsql_query(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(param): Query<RsqlParam>,
) -> AppResult<StatusCode> {
    let group = param
        .group
        .clone()
        .ok_or_else(|| AppError::BadRequest("group parameter is required".into()))?;
    assign_group_by_rsql(&state, &identity, Some(&group), param.rsql()?).await?;
    Ok(StatusCode::OK)
}

/// PUT /targetgroups/{group}?rsql=... — same with the group in the path.
async fn assign_by_rsql_to(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(group): Path<String>,
    Query(param): Query<RsqlParam>,
) -> AppResult<StatusCode> {
    assign_group_by_rsql(&state, &identity, Some(&group), param.rsql()?).await?;
    Ok(StatusCode::OK)
}

/// DELETE /targetgroups/assigned  body: ["id1", ...] — clear their group.
async fn unassign_ids(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Vec<String>>,
) -> AppResult<StatusCode> {
    set_group(&state, &identity, None, &body).await?;
    Ok(StatusCode::OK)
}

/// DELETE /targetgroups?rsql=... — clear groups by filter.
async fn unassign_by_rsql(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(param): Query<RsqlParam>,
) -> AppResult<StatusCode> {
    assign_group_by_rsql(&state, &identity, None, param.rsql()?).await?;
    Ok(StatusCode::OK)
}
