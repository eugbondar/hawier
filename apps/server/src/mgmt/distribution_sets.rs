//! `/rest/v1/distributionsets`.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hawier_api::codes::{ActionStatus, ActionType};
use serde_json::{json, Value};
use sqlx::Row;

use super::repr::{ds_json, ds_modules, DS_SELECT};
use super::{body_str, one_or_many, paged, request_base, require_str, rsql_filter, ListParams};
use crate::actions as action_store;
use crate::auth::MgmtIdentity;
use crate::deploy_logic;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/distributionsets", get(list).post(create))
        .route(
            "/rest/v1/distributionsets/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route(
            "/rest/v1/distributionsets/{id}/assignedTargets",
            get(assigned_targets).post(assign_targets),
        )
        .route(
            "/rest/v1/distributionsets/{id}/installedTargets",
            get(installed_targets),
        )
        .route(
            "/rest/v1/distributionsets/{id}/autoAssignTargetFilters",
            get(auto_assign_filters),
        )
        .route(
            "/rest/v1/distributionsets/{id}/assignedSM",
            get(list_modules).post(assign_modules),
        )
        .route(
            "/rest/v1/distributionsets/{id}/assignedSM/{sm_id}",
            axum::routing::delete(unassign_module),
        )
        // hawier extension (absent in hawkBit, mirrors targets/{id}/tags);
        // recorded in API_NOTES.md.
        .route("/rest/v1/distributionsets/{id}/tags", get(list_tags))
        .route(
            "/rest/v1/distributionsets/{id}/metadata",
            get(list_metadata).post(create_metadata),
        )
        .route(
            "/rest/v1/distributionsets/{id}/metadata/{key}",
            get(get_metadata).put(put_metadata).delete(delete_metadata),
        )
        .route("/rest/v1/distributionsets/{id}/statistics", get(statistics))
        .route(
            "/rest/v1/distributionsets/{id}/statistics/{kind}",
            get(statistics_of_kind),
        )
        .route(
            "/rest/v1/distributionsets/{id}/invalidate",
            post(invalidate),
        )
}

static DS_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "d.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "d.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "d.description",
                kind: Kind::Text,
            },
        ),
        (
            "version",
            Field::Column {
                sql: "d.version",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "d.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "d.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            // Computed column — same NOT EXISTS expression as DS_SELECT's
            // `complete`; the RSQL value arrives as text, hence LOWER($V).
            "complete",
            Field::Exists {
                template: "(NOT EXISTS (SELECT 1 FROM sp_ds_type_element e \
                           JOIN sp_software_module_type mt ON mt.id = e.software_module_type \
                           WHERE e.distribution_set_type = d.ds_type AND e.mandatory = TRUE \
                             AND NOT EXISTS (SELECT 1 FROM sp_ds_sm sl \
                                 JOIN sp_software_module sm ON sm.id = sl.sm_id \
                                 WHERE sl.ds_id = d.id AND sm.sm_type = mt.id))) \
                           = (LOWER($V) = 'true')",
            },
        ),
        (
            // Computed: is this DS assigned to at least one target?
            // (hawkBit has no such filter; the console's Assigned/Unassigned
            // views need it.)
            "assigned",
            Field::Exists {
                template: "(EXISTS (SELECT 1 FROM sp_target tg \
                           WHERE tg.assigned_distribution_set = d.id)) \
                           = (LOWER($V) = 'true')",
            },
        ),
        (
            "valid",
            Field::Column {
                sql: "d.valid",
                kind: Kind::Bool,
            },
        ),
        (
            "locked",
            Field::Column {
                sql: "d.locked",
                kind: Kind::Bool,
            },
        ),
        (
            "requiredmigrationstep",
            Field::Column {
                sql: "d.required_migration_step",
                kind: Kind::Bool,
            },
        ),
        (
            "type",
            Field::Column {
                sql: "t.type_key",
                kind: Kind::Text,
            },
        ),
        (
            "metadata.*",
            Field::KeyValueSub {
                table: "sp_ds_metadata",
                fk: "ds",
                owner: "d.id",
                key_col: "meta_key",
                val_col: "meta_value",
            },
        ),
        (
            "tag",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_ds_tag l \
                           JOIN sp_distribution_set_tag tg ON tg.id = l.tag \
                           WHERE l.ds = d.id AND tg.name $CMP $V)",
            },
        ),
    ])
});

async fn load_ds(state: &AppState, tenant: &str, id: i64) -> AppResult<sqlx::postgres::PgRow> {
    sqlx::query(&format!(
        "{DS_SELECT} WHERE d.id = $1 AND UPPER(d.tenant) = UPPER($2)"
    ))
    .bind(id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("distribution set {id} not found")))
}

async fn ds_value(state: &AppState, tenant: &str, id: i64, base: &str) -> AppResult<Value> {
    let row = load_ds(state, tenant, id).await?;
    let modules = ds_modules(&state.pool, id, base).await?;
    Ok(ds_json(&row, base, modules))
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &DS_FIELDS, 1)?;
    let order = super::order_by_with(
        params.sort.as_deref(),
        &DS_FIELDS,
        "d.id ASC",
        &[("complete", "complete")],
    )?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_distribution_set d
             JOIN sp_distribution_set_type t ON t.id = d.ds_type
             WHERE UPPER(d.tenant) = UPPER($1) AND d.deleted IS NOT TRUE{filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{DS_SELECT} WHERE UPPER(d.tenant) = UPPER($1) AND d.deleted IS NOT TRUE{filter_sql} \
             {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut content = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i64 = row.get("id");
        // compact representation omits the module list (hawier-ui uses it)
        let modules = if params.full() {
            ds_modules(&state.pool, id, &base).await?
        } else {
            Vec::new()
        };
        content.push(ds_json(row, &base, modules));
    }
    Ok(paged(content, total))
}

async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::new();

    for item in &items {
        let name = require_str(item, "name")?;
        let version = require_str(item, "version")?;
        let type_key = body_str(item, "type").unwrap_or_else(|| "os_app".to_string());
        let ds_type: i64 = sqlx::query_scalar(
            "SELECT id FROM sp_distribution_set_type
             WHERE type_key = $1 AND UPPER(tenant) = UPPER($2)",
        )
        .bind(&type_key)
        .bind(&identity.tenant)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("distribution set type '{type_key}' not found"))
        })?;

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO sp_distribution_set
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, description, name, version, deleted, required_migration_step,
              ds_type, valid, locked)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, FALSE, $7, $8, TRUE, FALSE)
             RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(body_str(item, "description"))
        .bind(&name)
        .bind(&version)
        .bind(
            item.get("requiredMigrationStep")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )
        .bind(ds_type)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
                AppError::Conflict(format!("distribution set {name}:{version} already exists"))
            }
            _ => AppError::Db(e),
        })?;

        if let Some(modules) = item.get("modules").and_then(|v| v.as_array()) {
            for module in modules {
                if let Some(sm_id) = module.get("id").and_then(|v| v.as_i64()) {
                    sqlx::query(
                        "INSERT INTO sp_ds_sm (ds_id, sm_id) VALUES ($1, $2)
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(id)
                    .bind(sm_id)
                    .execute(&state.pool)
                    .await?;
                }
            }
        }

        created.push(ds_value(&state, &identity.tenant, id, &base).await?);
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    Ok(Json(
        ds_value(&state, &identity.tenant, id, &request_base(&headers)).await?,
    ))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    sqlx::query(
        "UPDATE sp_distribution_set SET
             name = COALESCE($1, name),
             description = COALESCE($2, description),
             version = COALESCE($3, version),
             required_migration_step = COALESCE($4, required_migration_step),
             locked = COALESCE($5, locked),
             last_modified_at = $6, last_modified_by = $7,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $8",
    )
    .bind(body_str(&body, "name"))
    .bind(body_str(&body, "description"))
    .bind(body_str(&body, "version"))
    .bind(body.get("requiredMigrationStep").and_then(|v| v.as_bool()))
    .bind(body.get("locked").and_then(|v| v.as_bool()))
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        ds_value(&state, &identity.tenant, id, &request_base(&headers)).await?,
    ))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    load_ds(&state, &identity.tenant, id).await?;
    // Soft delete: actions reference the DS (SCHEMA_NOTES.md #8).
    sqlx::query("UPDATE sp_distribution_set SET deleted = TRUE WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Target relations
// ---------------------------------------------------------------------------

async fn targets_by_column(
    state: &AppState,
    identity: &MgmtIdentity,
    ds_id: i64,
    column: &str,
    params: &ListParams,
    base: &str,
) -> AppResult<Json<Value>> {
    load_ds(state, &identity.tenant, ds_id).await?;
    // `load_ds` already proved the set belongs to this tenant, and the assign
    // path never crosses tenants — but every query that reads `sp_target`
    // states the tenant itself, so no future caller can widen this by accident.
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM sp_target t
         WHERE t.{column} = $1 AND UPPER(t.tenant) = UPPER($2)"
    ))
    .bind(ds_id)
    .bind(&identity.tenant)
    .fetch_one(&state.pool)
    .await?;
    let rows = sqlx::query(&format!(
        "SELECT t.controller_id, t.name, t.update_status FROM sp_target t
         WHERE t.{column} = $1 AND UPPER(t.tenant) = UPPER($2)
         ORDER BY t.id LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    ))
    .bind(ds_id)
    .bind(&identity.tenant)
    .fetch_all(&state.pool)
    .await?;
    let content = rows
        .iter()
        .map(|r| {
            let controller_id: String = r.get("controller_id");
            json!({
                "controllerId": controller_id,
                "name": r.get::<String, _>("name"),
                "updateStatus": super::update_status_name(r.get::<i32, _>("update_status")),
                "_links": { "self": { "href": format!("{base}/rest/v1/targets/{controller_id}") } }
            })
        })
        .collect();
    Ok(paged(content, total))
}

async fn assigned_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    targets_by_column(
        &state,
        &identity,
        id,
        "assigned_distribution_set",
        &params,
        &request_base(&headers),
    )
    .await
}

async fn installed_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    targets_by_column(
        &state,
        &identity,
        id,
        "installed_distribution_set",
        &params,
        &request_base(&headers),
    )
    .await
}

/// POST assignedTargets: assign this DS to a list of controller ids.
async fn assign_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;

    let multi_assignments = crate::tenant::config_bool(
        &state.pool,
        &identity.tenant,
        crate::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        false,
    )
    .await?;

    // hawkBit refuses the whole request when any target is incompatible,
    // before anything is written (deployment-assignment spec).
    let requested: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            Value::String(s) => Some(s.clone()),
            _ => body_str(item, "id"),
        })
        .collect();
    deploy_logic::ensure_targets_compatible(
        &mut *state.pool.acquire().await?,
        &identity.tenant,
        id,
        &requested,
        deploy_logic::typed_only(&state.pool, &identity.tenant).await?,
    )
    .await?;

    let mut assigned_actions = Vec::new();
    let mut already_assigned = 0usize;
    for item in &items {
        let controller_id = match item {
            Value::String(s) => s.clone(),
            _ => require_str(item, "id")?,
        };
        let target = sqlx::query(
            "SELECT id, assigned_distribution_set FROM sp_target
             WHERE controller_id = $1 AND UPPER(tenant) = UPPER($2)",
        )
        .bind(&controller_id)
        .bind(&identity.tenant)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("target {controller_id} not found")))?;

        if target.get::<Option<i64>, _>("assigned_distribution_set") == Some(id)
            && !multi_assignments
        {
            already_assigned += 1;
            continue;
        }

        let action_type = match body_str(item, "type") {
            Some(t) => super::parse_action_type(&t)?,
            None => ActionType::Forced,
        };
        let outcome = deploy_logic::assign(
            &state.pool,
            &state.incidents,
            &identity.tenant,
            &deploy_logic::AssignRequest {
                target_id: target.get("id"),
                ds_id: id,
                action_type,
                forced_time: item.get("forcetime").and_then(|v| v.as_i64()),
                weight: item
                    .get("weight")
                    .and_then(|v| v.as_i64())
                    .map(|w| w as i32)
                    .unwrap_or(deploy_logic::DEFAULT_WEIGHT),
                initiated_by: identity.username.clone(),
                confirmation_required: item.get("confirmationRequired").and_then(|v| v.as_bool()),
                maintenance_window: item.get("maintenanceWindow").map(|mw| {
                    deploy_logic::MaintenanceWindow {
                        schedule: body_str(mw, "schedule").unwrap_or_default(),
                        duration: body_str(mw, "duration").unwrap_or_default(),
                        time_zone: body_str(mw, "timezone").unwrap_or_default(),
                    }
                }),
            },
        )
        .await?;
        assigned_actions.push(json!({
            "id": outcome.action_id,
            "_links": {
                "self": {
                    "href": format!("{base}/rest/v1/targets/{controller_id}/actions/{}", outcome.action_id)
                }
            }
        }));
    }

    let assigned = assigned_actions.len();
    Ok(Json(json!({
        "alreadyAssigned": already_assigned,
        "assignedActions": assigned_actions,
        "assigned": assigned,
        "total": assigned + already_assigned,
    })))
}

/// hawier extension: tags of one DS (hawkBit only exposes the inverse,
/// `distributionsettags/{id}/assigned`). Additive; see API_NOTES.md.
async fn list_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let rows = sqlx::query(
        "SELECT g.id, g.created_at, g.created_by, g.last_modified_at, g.last_modified_by,
                g.name, g.description, g.colour
         FROM sp_distribution_set_tag g JOIN sp_ds_tag l ON l.tag = g.id
         WHERE l.ds = $1 ORDER BY g.id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let base = request_base(&headers);
    Ok(Json(Value::Array(
        rows.iter()
            .map(|r| super::repr::tag_json(r, &base, "distributionsettags"))
            .collect(),
    )))
}

async fn auto_assign_filters(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let base = request_base(&headers);
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_target_filter_query
         WHERE auto_assign_distribution_set = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .fetch_one(&state.pool)
    .await?;
    let rows = sqlx::query(&format!(
        "SELECT id, name, query FROM sp_target_filter_query
         WHERE auto_assign_distribution_set = $1 AND UPPER(tenant) = UPPER($2)
         ORDER BY id LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    ))
    .bind(id)
    .bind(&identity.tenant)
    .fetch_all(&state.pool)
    .await?;
    let content = rows
        .iter()
        .map(|r| {
            let filter_id: i64 = r.get("id");
            json!({
                "id": filter_id,
                "name": r.get::<String, _>("name"),
                "query": r.get::<String, _>("query"),
                "_links": { "self": { "href": format!("{base}/rest/v1/targetfilters/{filter_id}") } }
            })
        })
        .collect();
    Ok(paged(content, total))
}

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

async fn list_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let base = request_base(&headers);
    let modules = ds_modules(&state.pool, id, &base).await?;
    let total = modules.len() as i64;
    let start = params.offset()?.min(total) as usize;
    let end = (start + params.limit()? as usize).min(modules.len());
    Ok(paged(modules[start..end].to_vec(), total))
}

async fn assign_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    let ds = load_ds(&state, &identity.tenant, id).await?;
    if ds.get::<bool, _>("locked") {
        return Err(AppError::Conflict(
            "distribution set is locked and cannot be modified".into(),
        ));
    }
    let (items, _) = one_or_many(body)?;
    for item in &items {
        let sm_id = item
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| AppError::BadRequest("'id' (software module) is required".into()))?;
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM sp_software_module
             WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE",
        )
        .bind(sm_id)
        .bind(&identity.tenant)
        .fetch_optional(&state.pool)
        .await?;
        if exists.is_none() {
            return Err(AppError::NotFound(format!(
                "software module {sm_id} not found"
            )));
        }
        sqlx::query("INSERT INTO sp_ds_sm (ds_id, sm_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(sm_id)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::OK)
}

async fn unassign_module(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, sm_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    let ds = load_ds(&state, &identity.tenant, id).await?;
    if ds.get::<bool, _>("locked") {
        return Err(AppError::Conflict(
            "distribution set is locked and cannot be modified".into(),
        ));
    }
    sqlx::query("DELETE FROM sp_ds_sm WHERE ds_id = $1 AND sm_id = $2")
        .bind(id)
        .bind(sm_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

async fn list_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_ds_metadata WHERE ds = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query(
        "SELECT meta_key, meta_value FROM sp_ds_metadata WHERE ds = $1
         ORDER BY meta_key LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(params.limit()?)
    .bind(params.offset()?)
    .fetch_all(&state.pool)
    .await?;
    let content: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "key": r.get::<String, _>("meta_key"),
                "value": r.get::<Option<String>, _>("meta_value"),
            })
        })
        .collect();
    Ok(paged(content, total))
}

async fn create_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    load_ds(&state, &identity.tenant, id).await?;
    let (items, _) = one_or_many(body)?;
    let mut created = Vec::new();
    for item in &items {
        let key = require_str(item, "key")?;
        let value = body_str(item, "value").unwrap_or_default();
        sqlx::query(
            "INSERT INTO sp_ds_metadata (ds, meta_key, meta_value) VALUES ($1, $2, $3)
             ON CONFLICT (ds, meta_key) DO UPDATE SET meta_value = EXCLUDED.meta_value",
        )
        .bind(id)
        .bind(&key)
        .bind(&value)
        .execute(&state.pool)
        .await?;
        created.push(json!({ "key": key, "value": value }));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let row = sqlx::query(
        "SELECT meta_key, meta_value FROM sp_ds_metadata WHERE ds = $1 AND meta_key = $2",
    )
    .bind(id)
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("metadata '{key}' not found")))?;
    Ok(Json(json!({
        "key": row.get::<String, _>("meta_key"),
        "value": row.get::<Option<String>, _>("meta_value"),
    })))
}

async fn put_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let value = body_str(&body, "value").unwrap_or_default();
    let result =
        sqlx::query("UPDATE sp_ds_metadata SET meta_value = $1 WHERE ds = $2 AND meta_key = $3")
            .bind(&value)
            .bind(id)
            .bind(&key)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("metadata '{key}' not found")));
    }
    Ok(Json(json!({ "key": key, "value": value })))
}

async fn delete_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
) -> AppResult<StatusCode> {
    load_ds(&state, &identity.tenant, id).await?;
    let result = sqlx::query("DELETE FROM sp_ds_metadata WHERE ds = $1 AND meta_key = $2")
        .bind(id)
        .bind(&key)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("metadata '{key}' not found")));
    }
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Statistics / invalidate
// ---------------------------------------------------------------------------

async fn collect_statistics(
    state: &AppState,
    identity: &MgmtIdentity,
    id: i64,
) -> AppResult<Value> {
    load_ds(state, &identity.tenant, id).await?;

    let rollouts = sqlx::query(
        "SELECT status, COUNT(*) AS c FROM sp_rollout
         WHERE distribution_set = $1 AND deleted IS NOT TRUE GROUP BY status",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let mut rollouts_map = serde_json::Map::new();
    let mut rollouts_total = 0i64;
    for row in &rollouts {
        let count: i64 = row.get("c");
        rollouts_total += count;
        let name = hawier_api::codes::RolloutStatus::try_from(row.get::<i32, _>("status"))
            .map(|s| format!("{s:?}").to_lowercase())
            .unwrap_or_else(|_| "unknown".into());
        rollouts_map.insert(name, json!(count));
    }

    let actions = sqlx::query(
        "SELECT status, COUNT(*) AS c FROM sp_action WHERE distribution_set = $1 GROUP BY status",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let mut actions_map = serde_json::Map::new();
    let mut actions_total = 0i64;
    for row in &actions {
        let count: i64 = row.get("c");
        actions_total += count;
        actions_map.insert(
            super::action_detail_status(row.get::<i32, _>("status")),
            json!(count),
        );
    }

    let auto_assignments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_target_filter_query
         WHERE auto_assign_distribution_set = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .fetch_one(&state.pool)
    .await?;

    Ok(json!({
        "rollouts": { "total": rollouts_total, "details": rollouts_map },
        "actions": { "total": actions_total, "details": actions_map },
        "autoAssignments": auto_assignments,
        "totalAutoAssignments": auto_assignments,
    }))
}

async fn statistics(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    Ok(Json(collect_statistics(&state, &identity, id).await?))
}

async fn statistics_of_kind(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, kind)): Path<(i64, String)>,
) -> AppResult<Json<Value>> {
    let stats = collect_statistics(&state, &identity, id).await?;
    let key = match kind.as_str() {
        "rollouts" => "rollouts",
        "actions" => "actions",
        "autoassignments" => "autoAssignments",
        other => return Err(AppError::NotFound(format!("unknown statistics '{other}'"))),
    };
    Ok(Json(stats.get(key).cloned().unwrap_or(Value::Null)))
}

/// POST invalidate: mark the DS invalid, optionally cancelling actions and
/// stopping rollouts that use it.
async fn invalidate(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_ds(&state, &identity.tenant, id).await?;
    let action_cancelation = body_str(&body, "actionCancelationType")
        .unwrap_or_else(|| "none".into())
        .to_lowercase();
    let cancel_rollouts = body
        .get("cancelRollouts")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE sp_distribution_set SET valid = FALSE, last_modified_at = $1,
             last_modified_by = $2, optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $3",
    )
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    let mut canceled = 0i64;
    if action_cancelation != "none" {
        let rows = sqlx::query(
            "SELECT id FROM sp_action WHERE distribution_set = $1 AND active = TRUE FOR UPDATE",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let force = action_cancelation == "force";
        for row in &rows {
            let action_id: i64 = row.get("id");
            let (status, active) = if force {
                (ActionStatus::Canceled, false)
            } else {
                (ActionStatus::Canceling, true)
            };
            action_store::insert_status(
                &mut tx,
                &identity.tenant,
                action_id,
                status,
                None,
                None,
                &identity.username,
                &["Update Server: distribution set invalidated".to_string()],
            )
            .await?;
            action_store::set_action_status(&mut tx, action_id, status, active, &identity.username)
                .await?;
            canceled += 1;
        }
    }

    let mut stopped_rollouts = 0i64;
    if cancel_rollouts {
        let result = sqlx::query(
            "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3
             WHERE distribution_set = $4 AND status NOT IN ($5, $6, $1)",
        )
        .bind(hawier_api::codes::RolloutStatus::Stopping.code())
        .bind(now_millis())
        .bind(&identity.username)
        .bind(id)
        .bind(hawier_api::codes::RolloutStatus::Finished.code())
        .bind(hawier_api::codes::RolloutStatus::Stopped.code())
        .execute(&mut *tx)
        .await?;
        stopped_rollouts = result.rows_affected() as i64;
    }

    tx.commit().await?;
    Ok(Json(json!({
        "affectedActions": canceled,
        "affectedRollouts": stopped_rollouts,
    })))
}
