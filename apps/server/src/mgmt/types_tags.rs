//! `/rest/v1/{targettags,distributionsettags,targettypes,distributionsettypes,
//! softwaremoduletypes}` — tag and type resources (hawkBit Mgmt API).

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{
    body_str, one_or_many, order_by, paged, put_audit, request_base, require_str, rsql_filter,
    update_status_name, ListParams,
};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        // target tags
        .route(
            "/rest/v1/targettags",
            get(list_target_tags).post(create_target_tags),
        )
        .route(
            "/rest/v1/targettags/{id}",
            get(get_target_tag)
                .put(update_target_tag)
                .delete(delete_target_tag),
        )
        .route(
            "/rest/v1/targettags/{id}/assigned",
            get(list_tag_assigned_targets)
                .post(assign_tag_targets)
                .delete(unassign_tag_targets_bulk),
        )
        .route(
            "/rest/v1/targettags/{id}/assigned/{controller_id}",
            post(assign_tag_target).delete(unassign_tag_target),
        )
        // distribution set tags
        .route(
            "/rest/v1/distributionsettags",
            get(list_ds_tags).post(create_ds_tags),
        )
        .route(
            "/rest/v1/distributionsettags/{id}",
            get(get_ds_tag).put(update_ds_tag).delete(delete_ds_tag),
        )
        .route(
            "/rest/v1/distributionsettags/{id}/assigned",
            get(list_tag_assigned_ds)
                .post(assign_tag_ds_bulk)
                .delete(unassign_tag_ds_bulk),
        )
        .route(
            "/rest/v1/distributionsettags/{id}/assigned/{ds_id}",
            post(assign_tag_ds).delete(unassign_tag_ds),
        )
        // target types
        .route(
            "/rest/v1/targettypes",
            get(list_target_types).post(create_target_types),
        )
        .route(
            "/rest/v1/targettypes/{id}",
            get(get_target_type)
                .put(update_target_type)
                .delete(delete_target_type),
        )
        .route(
            "/rest/v1/targettypes/{id}/compatibledistributionsettypes",
            get(list_compatible_ds_types).post(add_compatible_ds_types),
        )
        .route(
            "/rest/v1/targettypes/{id}/compatibledistributionsettypes/{ds_type_id}",
            delete(remove_compatible_ds_type),
        )
        // distribution set types
        .route(
            "/rest/v1/distributionsettypes",
            get(list_ds_types).post(create_ds_types),
        )
        .route(
            "/rest/v1/distributionsettypes/{id}",
            get(get_ds_type).put(update_ds_type).delete(delete_ds_type),
        )
        .route(
            "/rest/v1/distributionsettypes/{id}/mandatorymoduletypes",
            get(list_mandatory_modules).post(add_mandatory_modules),
        )
        .route(
            "/rest/v1/distributionsettypes/{id}/mandatorymoduletypes/{sm_type_id}",
            delete(remove_mandatory_module),
        )
        .route(
            "/rest/v1/distributionsettypes/{id}/optionalmoduletypes",
            get(list_optional_modules).post(add_optional_modules),
        )
        .route(
            "/rest/v1/distributionsettypes/{id}/optionalmoduletypes/{sm_type_id}",
            delete(remove_optional_module),
        )
        // software module types
        .route(
            "/rest/v1/softwaremoduletypes",
            get(list_sm_types).post(create_sm_types),
        )
        .route(
            "/rest/v1/softwaremoduletypes/{id}",
            get(get_sm_type).put(update_sm_type).delete(delete_sm_type),
        )
}

// ---------------------------------------------------------------------------
// Field maps
// ---------------------------------------------------------------------------

static TAG_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "g.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "g.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "g.description",
                kind: Kind::Text,
            },
        ),
        (
            "colour",
            Field::Column {
                sql: "g.colour",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "g.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "g.last_modified_at",
                kind: Kind::Int,
            },
        ),
    ])
});

static TYPE_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "y.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "y.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "y.description",
                kind: Kind::Text,
            },
        ),
        (
            "colour",
            Field::Column {
                sql: "y.colour",
                kind: Kind::Text,
            },
        ),
        (
            "key",
            Field::Column {
                sql: "y.type_key",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "y.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "y.last_modified_at",
                kind: Kind::Int,
            },
        ),
    ])
});

/// Software module types carry one column the other two type tables do not,
/// and a declared console column has to be sortable — a header that answers
/// 400 replaces the list with an error callout.
static SM_TYPE_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    let mut fields = FieldMap::new(vec![(
        "maxassignments",
        Field::Column {
            sql: "y.max_ds_assignments",
            kind: Kind::Int,
        },
    )]);
    fields.extend(&TYPE_FIELDS);
    fields
});

static ASSIGNED_TARGET_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            "controllerid",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "t.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "t.description",
                kind: Kind::Text,
            },
        ),
    ])
});

static ASSIGNED_DS_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
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
            "version",
            Field::Column {
                sql: "d.version",
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
    ])
});

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn conflict_on_unique(err: sqlx::Error, what: &str) -> AppError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::Conflict(format!("{what}: name or key already in use"))
        }
        _ => AppError::Db(err),
    }
}

/// Accepts hawkBit's bare id (`5`) as well as an `{"id": 5}` object.
fn require_id(item: &Value) -> AppResult<i64> {
    item.as_i64()
        .or_else(|| item.get("id").and_then(Value::as_i64))
        .ok_or_else(|| AppError::BadRequest("'id' is required".into()))
}

/// Partial UPDATE of name/description/colour-style text columns; only keys
/// present in the body are touched (targets.rs pattern).
async fn update_named(
    state: &AppState,
    table: &str,
    id: i64,
    username: &str,
    body: &Value,
    columns: &[(&str, &str)],
    what: &str,
) -> AppResult<()> {
    let mut sets = vec![
        "last_modified_at = $1".to_string(),
        "last_modified_by = $2".to_string(),
        "optlock_revision = COALESCE(optlock_revision, 0) + 1".to_string(),
    ];
    let mut texts: Vec<Option<String>> = Vec::new();
    let mut arg_index = 2;
    for (json_key, column) in columns {
        if body.get(json_key).is_some() {
            arg_index += 1;
            sets.push(format!("{column} = ${arg_index}"));
            texts.push(body_str(body, json_key));
        }
    }
    let sql = format!(
        "UPDATE {table} SET {} WHERE id = ${}",
        sets.join(", "),
        arg_index + 1
    );
    let mut query = sqlx::query(&sql).bind(now_millis()).bind(username);
    for value in &texts {
        query = query.bind(value.clone());
    }
    query
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| conflict_on_unique(e, what))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tags (target tags and DS tags share shape and behaviour)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct TagKind {
    table: &'static str,
    resource: &'static str,
    what: &'static str,
}

const TARGET_TAG: TagKind = TagKind {
    table: "sp_target_tag",
    resource: "targettags",
    what: "target tag",
};
const DS_TAG: TagKind = TagKind {
    table: "sp_distribution_set_tag",
    resource: "distributionsettags",
    what: "distribution set tag",
};

fn tag_select(kind: TagKind) -> String {
    format!(
        "SELECT g.id, g.created_at, g.created_by, g.last_modified_at, g.last_modified_by, \
         g.name, g.description, g.colour FROM {} g",
        kind.table
    )
}

async fn load_tag_row(state: &AppState, tenant: &str, kind: TagKind, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{} WHERE UPPER(g.tenant) = UPPER($1) AND g.id = $2",
        tag_select(kind)
    ))
    .bind(tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("{} {id} not found", kind.what)))
}

async fn tag_list(
    state: &AppState,
    identity: &MgmtIdentity,
    params: &ListParams,
    base: &str,
    kind: TagKind,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &TAG_FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &TAG_FIELDS, "g.id ASC")?;

    let count_sql = format!(
        "SELECT COUNT(*) FROM {} g WHERE UPPER(g.tenant) = UPPER($1){filter_sql}",
        kind.table
    );
    let total: i64 = bind_values(sqlx::query(&count_sql).bind(&identity.tenant), &binds)
        .fetch_one(&state.pool)
        .await?
        .get(0);

    let list_sql = format!(
        "{} WHERE UPPER(g.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
        tag_select(kind),
        params.limit()?,
        params.offset()?
    );
    let rows = bind_values(sqlx::query(&list_sql).bind(&identity.tenant), &binds)
        .fetch_all(&state.pool)
        .await?;

    let content = rows
        .iter()
        .map(|r| super::repr::tag_json(r, base, kind.resource))
        .collect();
    Ok(paged(content, total))
}

async fn tag_create(
    state: &AppState,
    identity: &MgmtIdentity,
    base: &str,
    body: Value,
    kind: TagKind,
) -> AppResult<(StatusCode, Json<Value>)> {
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::with_capacity(items.len());
    for item in &items {
        let name = require_str(item, "name")?;
        let inserted: Option<i64> = sqlx::query_scalar(&format!(
            "INSERT INTO {} (created_at, created_by, last_modified_at, last_modified_by, \
             optlock_revision, tenant, name, description, colour) \
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6) \
             ON CONFLICT DO NOTHING RETURNING id",
            kind.table
        ))
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(&name)
        .bind(body_str(item, "description"))
        .bind(body_str(item, "colour"))
        .fetch_optional(&state.pool)
        .await?;
        let id = inserted
            .ok_or_else(|| AppError::Conflict(format!("{} '{name}' already exists", kind.what)))?;
        let row = load_tag_row(state, &identity.tenant, kind, id).await?;
        created.push(super::repr::tag_json(&row, base, kind.resource));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn tag_update(
    state: &AppState,
    identity: &MgmtIdentity,
    base: &str,
    kind: TagKind,
    id: i64,
    body: &Value,
) -> AppResult<Json<Value>> {
    load_tag_row(state, &identity.tenant, kind, id).await?;
    update_named(
        state,
        kind.table,
        id,
        &identity.username,
        body,
        &[
            ("name", "name"),
            ("description", "description"),
            ("colour", "colour"),
        ],
        kind.what,
    )
    .await?;
    let row = load_tag_row(state, &identity.tenant, kind, id).await?;
    Ok(Json(super::repr::tag_json(&row, base, kind.resource)))
}

async fn tag_delete(
    state: &AppState,
    identity: &MgmtIdentity,
    kind: TagKind,
    id: i64,
) -> AppResult<StatusCode> {
    let result = sqlx::query(&format!(
        "DELETE FROM {} WHERE UPPER(tenant) = UPPER($1) AND id = $2",
        kind.table
    ))
    .bind(&identity.tenant)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("{} {id} not found", kind.what)));
    }
    Ok(StatusCode::OK)
}

// --- target tag handlers ---------------------------------------------------

async fn list_target_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    tag_list(
        &state,
        &identity,
        &params,
        &request_base(&headers),
        TARGET_TAG,
    )
    .await
}

async fn create_target_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    tag_create(&state, &identity, &request_base(&headers), body, TARGET_TAG).await
}

async fn get_target_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    Ok(Json(super::repr::tag_json(
        &row,
        &request_base(&headers),
        TARGET_TAG.resource,
    )))
}

async fn update_target_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    tag_update(
        &state,
        &identity,
        &request_base(&headers),
        TARGET_TAG,
        id,
        &body,
    )
    .await
}

async fn delete_target_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    tag_delete(&state, &identity, TARGET_TAG, id).await
}

// --- target tag assignment --------------------------------------------------

const ASSIGNED_TARGET_SELECT: &str = "SELECT t.id, t.created_at, t.created_by,
       t.last_modified_at, t.last_modified_by, t.description, t.name, t.controller_id,
       t.address, t.last_target_query, t.install_date, t.request_controller_attributes,
       t.update_status, t.target_type, t.target_group,
       tt.name AS target_type_name,
       EXISTS (SELECT 1 FROM sp_target_conf_status c WHERE c.target = t.id) AS auto_confirm_active
FROM sp_target t
LEFT JOIN sp_target_type tt ON tt.id = t.target_type
JOIN sp_target_target_tag l ON l.target = t.id";

/// Compact target JSON (same shape as the targets list representation).
fn assigned_target_json(row: &PgRow, base: &str) -> Value {
    let controller_id: String = row.get("controller_id");
    let address: Option<String> = row.get("address");
    let ip = address
        .as_deref()
        .and_then(|a| a.split("://").nth(1))
        .map(str::to_string);
    let self_href = format!("{base}/rest/v1/targets/{controller_id}");
    let mut value = json!({
        "controllerId": controller_id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "updateStatus": update_status_name(row.get::<i32, _>("update_status")),
        "lastControllerRequestAt": row.get::<Option<i64>, _>("last_target_query"),
        "installedAt": row.get::<Option<i64>, _>("install_date"),
        "ipAddress": ip,
        "address": address,
        "group": row.get::<Option<String>, _>("target_group"),
        "requestAttributes": row.get::<bool, _>("request_controller_attributes"),
        "targetType": row.get::<Option<i64>, _>("target_type"),
        "targetTypeName": row.get::<Option<String>, _>("target_type_name"),
        "autoConfirmActive": row.get::<bool, _>("auto_confirm_active"),
        "_links": { "self": { "href": self_href } }
    });
    put_audit(&mut value, row);
    value
}

async fn list_tag_assigned_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &ASSIGNED_TARGET_FIELDS, 2)?;
    let order = order_by(params.sort.as_deref(), &ASSIGNED_TARGET_FIELDS, "t.id ASC")?;

    let count_sql = format!(
        "SELECT COUNT(*) FROM sp_target t JOIN sp_target_target_tag l ON l.target = t.id \
         WHERE UPPER(t.tenant) = UPPER($1) AND l.tag = $2{filter_sql}"
    );
    let total: i64 = bind_values(
        sqlx::query(&count_sql).bind(&identity.tenant).bind(id),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let list_sql = format!(
        "{ASSIGNED_TARGET_SELECT} WHERE UPPER(t.tenant) = UPPER($1) AND l.tag = $2{filter_sql} \
         {order} LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    );
    let rows = bind_values(
        sqlx::query(&list_sql).bind(&identity.tenant).bind(id),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let content = rows
        .iter()
        .map(|r| assigned_target_json(r, &base))
        .collect();
    Ok(paged(content, total))
}

async fn link_target_to_tag(
    state: &AppState,
    tenant: &str,
    tag_id: i64,
    controller_id: &str,
) -> AppResult<()> {
    let target_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_target WHERE UPPER(tenant) = UPPER($1) AND controller_id = $2",
    )
    .bind(tenant)
    .bind(controller_id)
    .fetch_optional(&state.pool)
    .await?;
    let target_id =
        target_id.ok_or_else(|| AppError::NotFound(format!("target {controller_id} not found")))?;
    sqlx::query(
        "INSERT INTO sp_target_target_tag (target, tag) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(target_id)
    .bind(tag_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn assign_tag_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    for controller_id in controller_id_list(body)? {
        link_target_to_tag(&state, &identity.tenant, id, &controller_id).await?;
    }
    Ok(StatusCode::OK)
}

/// hawkBit's wire format is a bare `["id1", "id2"]` list; accept
/// `{"controllerId": ...}` objects too for robustness.
fn controller_id_list(body: Value) -> crate::error::AppResult<Vec<String>> {
    let (items, _) = one_or_many(body)?;
    items
        .iter()
        .map(|item| match item {
            Value::String(s) => Ok(s.clone()),
            Value::Object(_) => require_str(item, "controllerId"),
            _ => Err(crate::error::AppError::BadRequest(
                "expected a controller id list".into(),
            )),
        })
        .collect()
}

async fn assign_tag_target(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, controller_id)): Path<(i64, String)>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    link_target_to_tag(&state, &identity.tenant, id, &controller_id).await?;
    Ok(StatusCode::OK)
}

async fn unassign_tag_targets_bulk(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    for controller_id in controller_id_list(body)? {
        sqlx::query(
            "DELETE FROM sp_target_target_tag
             WHERE tag = $1 AND target IN (
                 SELECT id FROM sp_target
                 WHERE UPPER(tenant) = UPPER($2) AND controller_id = $3)",
        )
        .bind(id)
        .bind(&identity.tenant)
        .bind(&controller_id)
        .execute(&state.pool)
        .await?;
    }
    Ok(StatusCode::OK)
}

async fn unassign_tag_ds_bulk(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    let (items, _) = one_or_many(body)?;
    for item in &items {
        sqlx::query("DELETE FROM sp_ds_tag WHERE tag = $1 AND ds = $2")
            .bind(id)
            .bind(require_id(item)?)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::OK)
}

async fn unassign_tag_target(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, controller_id)): Path<(i64, String)>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, TARGET_TAG, id).await?;
    let target_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_target WHERE UPPER(tenant) = UPPER($1) AND controller_id = $2",
    )
    .bind(&identity.tenant)
    .bind(&controller_id)
    .fetch_optional(&state.pool)
    .await?;
    let target_id =
        target_id.ok_or_else(|| AppError::NotFound(format!("target {controller_id} not found")))?;
    sqlx::query("DELETE FROM sp_target_target_tag WHERE target = $1 AND tag = $2")
        .bind(target_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// --- DS tag handlers ---------------------------------------------------------

async fn list_ds_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    tag_list(&state, &identity, &params, &request_base(&headers), DS_TAG).await
}

async fn create_ds_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    tag_create(&state, &identity, &request_base(&headers), body, DS_TAG).await
}

async fn get_ds_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    Ok(Json(super::repr::tag_json(
        &row,
        &request_base(&headers),
        DS_TAG.resource,
    )))
}

async fn update_ds_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    tag_update(
        &state,
        &identity,
        &request_base(&headers),
        DS_TAG,
        id,
        &body,
    )
    .await
}

async fn delete_ds_tag(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    tag_delete(&state, &identity, DS_TAG, id).await
}

// --- DS tag assignment --------------------------------------------------------

async fn list_tag_assigned_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &ASSIGNED_DS_FIELDS, 2)?;
    let order = order_by(params.sort.as_deref(), &ASSIGNED_DS_FIELDS, "d.id ASC")?;

    let count_sql = format!(
        "SELECT COUNT(*) FROM sp_distribution_set d JOIN sp_ds_tag l ON l.ds = d.id \
         WHERE UPPER(d.tenant) = UPPER($1) AND l.tag = $2{filter_sql}"
    );
    let total: i64 = bind_values(
        sqlx::query(&count_sql).bind(&identity.tenant).bind(id),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let list_sql = format!(
        "{} JOIN sp_ds_tag l ON l.ds = d.id \
         WHERE UPPER(d.tenant) = UPPER($1) AND l.tag = $2{filter_sql} {order} \
         LIMIT {} OFFSET {}",
        super::repr::DS_SELECT,
        params.limit()?,
        params.offset()?
    );
    let rows = bind_values(
        sqlx::query(&list_sql).bind(&identity.tenant).bind(id),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut content = Vec::with_capacity(rows.len());
    for row in &rows {
        let ds_id: i64 = row.get("id");
        let modules = super::repr::ds_modules(&state.pool, ds_id, &base).await?;
        content.push(super::repr::ds_json(row, &base, modules));
    }
    Ok(paged(content, total))
}

async fn link_ds_to_tag(state: &AppState, tenant: &str, tag_id: i64, ds_id: i64) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE",
    )
    .bind(ds_id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "distribution set {ds_id} not found"
        )));
    }
    sqlx::query("INSERT INTO sp_ds_tag (ds, tag) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(ds_id)
        .bind(tag_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn assign_tag_ds_bulk(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    let (items, _) = one_or_many(body)?;
    for item in &items {
        link_ds_to_tag(&state, &identity.tenant, id, require_id(item)?).await?;
    }
    Ok(StatusCode::OK)
}

async fn assign_tag_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, ds_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    link_ds_to_tag(&state, &identity.tenant, id, ds_id).await?;
    Ok(StatusCode::OK)
}

async fn unassign_tag_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, ds_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    load_tag_row(&state, &identity.tenant, DS_TAG, id).await?;
    sqlx::query("DELETE FROM sp_ds_tag WHERE ds = $1 AND tag = $2")
        .bind(ds_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Types (shared list/load/update/delete machinery)
// ---------------------------------------------------------------------------

const TARGET_TYPE_SELECT: &str = "SELECT y.id, y.created_at, y.created_by,
       y.last_modified_at, y.last_modified_by, y.name, y.description, y.colour, y.type_key
FROM sp_target_type y";

const DS_TYPE_SELECT: &str = "SELECT y.id, y.created_at, y.created_by,
       y.last_modified_at, y.last_modified_by, y.name, y.description, y.colour,
       y.type_key, y.deleted
FROM sp_distribution_set_type y";

const SM_TYPE_SELECT: &str = "SELECT y.id, y.created_at, y.created_by,
       y.last_modified_at, y.last_modified_by, y.name, y.description, y.colour,
       y.type_key, y.deleted, y.max_ds_assignments, y.min_artifacts
FROM sp_software_module_type y";

fn target_type_json(row: &PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "colour": row.get::<Option<String>, _>("colour"),
        "key": row.get::<String, _>("type_key"),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/targettypes/{id}") },
            "compatibledistributionsettypes": {
                "href": format!("{base}/rest/v1/targettypes/{id}/compatibledistributionsettypes")
            }
        }
    });
    put_audit(&mut value, row);
    value
}

fn ds_type_json(row: &PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "colour": row.get::<Option<String>, _>("colour"),
        "key": row.get::<String, _>("type_key"),
        "deleted": row.get::<Option<bool>, _>("deleted").unwrap_or(false),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/distributionsettypes/{id}") },
            "mandatorymodules": {
                "href": format!("{base}/rest/v1/distributionsettypes/{id}/mandatorymoduletypes")
            },
            "optionalmodules": {
                "href": format!("{base}/rest/v1/distributionsettypes/{id}/optionalmoduletypes")
            }
        }
    });
    put_audit(&mut value, row);
    value
}

fn sm_type_json(row: &PgRow, base: &str) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "colour": row.get::<Option<String>, _>("colour"),
        "key": row.get::<String, _>("type_key"),
        "maxAssignments": row.get::<i32, _>("max_ds_assignments"),
        "minArtifacts": row.get::<i32, _>("min_artifacts"),
        "deleted": row.get::<Option<bool>, _>("deleted").unwrap_or(false),
        "_links": { "self": { "href": format!("{base}/rest/v1/softwaremoduletypes/{id}") } }
    });
    put_audit(&mut value, row);
    value
}

#[derive(Clone, Copy)]
struct TypeSpec {
    table: &'static str,
    select: &'static str,
    /// Table has a `deleted` column: lists hide soft-deleted rows and DELETE
    /// falls back to a soft delete when the type is still referenced.
    has_deleted: bool,
    what: &'static str,
    to_json: fn(&PgRow, &str) -> Value,
    /// Field map for filtering and sorting this type's list.
    fields: fn() -> &'static FieldMap,
}

const TARGET_TYPE: TypeSpec = TypeSpec {
    table: "sp_target_type",
    select: TARGET_TYPE_SELECT,
    has_deleted: false,
    what: "target type",
    to_json: target_type_json,
    fields: || &TYPE_FIELDS,
};
const DS_TYPE: TypeSpec = TypeSpec {
    table: "sp_distribution_set_type",
    select: DS_TYPE_SELECT,
    has_deleted: true,
    what: "distribution set type",
    to_json: ds_type_json,
    fields: || &TYPE_FIELDS,
};
const SM_TYPE: TypeSpec = TypeSpec {
    table: "sp_software_module_type",
    select: SM_TYPE_SELECT,
    has_deleted: true,
    what: "software module type",
    to_json: sm_type_json,
    fields: || &SM_TYPE_FIELDS,
};

async fn load_type_row(
    state: &AppState,
    tenant: &str,
    spec: TypeSpec,
    id: i64,
) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{} WHERE UPPER(y.tenant) = UPPER($1) AND y.id = $2",
        spec.select
    ))
    .bind(tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("{} {id} not found", spec.what)))
}

async fn type_list(
    state: &AppState,
    identity: &MgmtIdentity,
    params: &ListParams,
    base: &str,
    spec: TypeSpec,
) -> AppResult<Json<Value>> {
    let fields = (spec.fields)();
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), fields, 1)?;
    let order = order_by(params.sort.as_deref(), fields, "y.id ASC")?;
    let alive = if spec.has_deleted {
        " AND y.deleted IS NOT TRUE"
    } else {
        ""
    };

    let count_sql = format!(
        "SELECT COUNT(*) FROM {} y WHERE UPPER(y.tenant) = UPPER($1){alive}{filter_sql}",
        spec.table
    );
    let total: i64 = bind_values(sqlx::query(&count_sql).bind(&identity.tenant), &binds)
        .fetch_one(&state.pool)
        .await?
        .get(0);

    let list_sql = format!(
        "{} WHERE UPPER(y.tenant) = UPPER($1){alive}{filter_sql} {order} LIMIT {} OFFSET {}",
        spec.select,
        params.limit()?,
        params.offset()?
    );
    let rows = bind_values(sqlx::query(&list_sql).bind(&identity.tenant), &binds)
        .fetch_all(&state.pool)
        .await?;

    let content = rows.iter().map(|r| (spec.to_json)(r, base)).collect();
    Ok(paged(content, total))
}

async fn type_update(
    state: &AppState,
    identity: &MgmtIdentity,
    base: &str,
    spec: TypeSpec,
    id: i64,
    body: &Value,
) -> AppResult<Json<Value>> {
    load_type_row(state, &identity.tenant, spec, id).await?;
    update_named(
        state,
        spec.table,
        id,
        &identity.username,
        body,
        &[
            ("name", "name"),
            ("description", "description"),
            ("colour", "colour"),
        ],
        spec.what,
    )
    .await?;
    let row = load_type_row(state, &identity.tenant, spec, id).await?;
    Ok(Json((spec.to_json)(&row, base)))
}

/// DELETE a type. Tables with a `deleted` column fall back to a soft delete
/// when the row is still referenced (hawkBit keeps in-use types around).
async fn type_delete(
    state: &AppState,
    identity: &MgmtIdentity,
    spec: TypeSpec,
    id: i64,
) -> AppResult<StatusCode> {
    load_type_row(state, &identity.tenant, spec, id).await?;
    let result = sqlx::query(&format!("DELETE FROM {} WHERE id = $1", spec.table))
        .bind(id)
        .execute(&state.pool)
        .await;
    match result {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => match &err {
            sqlx::Error::Database(db) if spec.has_deleted && db.is_foreign_key_violation() => {
                sqlx::query(&format!(
                    "UPDATE {} SET deleted = TRUE, last_modified_at = $1, \
                     last_modified_by = $2 WHERE id = $3",
                    spec.table
                ))
                .bind(now_millis())
                .bind(&identity.username)
                .bind(id)
                .execute(&state.pool)
                .await?;
                Ok(StatusCode::OK)
            }
            _ => Err(err.into()),
        },
    }
}

// --- target types -------------------------------------------------------------

async fn list_target_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    type_list(
        &state,
        &identity,
        &params,
        &request_base(&headers),
        TARGET_TYPE,
    )
    .await
}

async fn create_target_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::with_capacity(items.len());
    for item in &items {
        let name = require_str(item, "name")?;
        let key = require_str(item, "key")?;
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO sp_target_type
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, description, colour, type_key)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7)
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(&name)
        .bind(body_str(item, "description"))
        .bind(body_str(item, "colour"))
        .bind(&key)
        .fetch_optional(&state.pool)
        .await?;
        let id = inserted.ok_or_else(|| {
            AppError::Conflict(format!("target type '{name}' / '{key}' already exists"))
        })?;
        if let Some(compatible) = item
            .get("compatibledistributionsettypes")
            .and_then(Value::as_array)
        {
            for entry in compatible {
                link_compatible_ds_type(&state, &identity.tenant, id, require_id(entry)?).await?;
            }
        }
        let row = load_type_row(&state, &identity.tenant, TARGET_TYPE, id).await?;
        created.push(target_type_json(&row, &base));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_target_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_type_row(&state, &identity.tenant, TARGET_TYPE, id).await?;
    Ok(Json(target_type_json(&row, &request_base(&headers))))
}

async fn update_target_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    type_update(
        &state,
        &identity,
        &request_base(&headers),
        TARGET_TYPE,
        id,
        &body,
    )
    .await
}

async fn delete_target_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    type_delete(&state, &identity, TARGET_TYPE, id).await
}

// --- target type ↔ compatible DS types -----------------------------------------

async fn link_compatible_ds_type(
    state: &AppState,
    tenant: &str,
    target_type_id: i64,
    ds_type_id: i64,
) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set_type
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE",
    )
    .bind(ds_type_id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "distribution set type {ds_type_id} not found"
        )));
    }
    sqlx::query(
        "INSERT INTO sp_target_type_ds_type (target_type, distribution_set_type)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(target_type_id)
    .bind(ds_type_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn list_compatible_ds_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_type_row(&state, &identity.tenant, TARGET_TYPE, id).await?;
    let base = request_base(&headers);
    let rows = sqlx::query(&format!(
        "{DS_TYPE_SELECT} JOIN sp_target_type_ds_type l ON l.distribution_set_type = y.id \
         WHERE l.target_type = $1 ORDER BY y.id"
    ))
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Value::Array(
        rows.iter().map(|r| ds_type_json(r, &base)).collect(),
    )))
}

async fn add_compatible_ds_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    load_type_row(&state, &identity.tenant, TARGET_TYPE, id).await?;
    let (items, _) = one_or_many(body)?;
    for item in &items {
        link_compatible_ds_type(&state, &identity.tenant, id, require_id(item)?).await?;
    }
    Ok(StatusCode::OK)
}

async fn remove_compatible_ds_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, ds_type_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    load_type_row(&state, &identity.tenant, TARGET_TYPE, id).await?;
    sqlx::query(
        "DELETE FROM sp_target_type_ds_type WHERE target_type = $1 AND distribution_set_type = $2",
    )
    .bind(id)
    .bind(ds_type_id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::OK)
}

// --- distribution set types -----------------------------------------------------

async fn list_ds_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    type_list(&state, &identity, &params, &request_base(&headers), DS_TYPE).await
}

async fn create_ds_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::with_capacity(items.len());
    for item in &items {
        let name = require_str(item, "name")?;
        let key = require_str(item, "key")?;
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO sp_distribution_set_type
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, description, colour, deleted, type_key)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, FALSE, $7)
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(&name)
        .bind(body_str(item, "description"))
        .bind(body_str(item, "colour"))
        .bind(&key)
        .fetch_optional(&state.pool)
        .await?;
        let id = inserted.ok_or_else(|| {
            AppError::Conflict(format!(
                "distribution set type '{name}' / '{key}' already exists"
            ))
        })?;
        for (json_key, mandatory) in [("mandatorymodules", true), ("optionalmodules", false)] {
            if let Some(modules) = item.get(json_key).and_then(Value::as_array) {
                for entry in modules {
                    link_module_type(&state, &identity.tenant, id, require_id(entry)?, mandatory)
                        .await?;
                }
            }
        }
        let row = load_type_row(&state, &identity.tenant, DS_TYPE, id).await?;
        created.push(ds_type_json(&row, &base));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_ds_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_type_row(&state, &identity.tenant, DS_TYPE, id).await?;
    Ok(Json(ds_type_json(&row, &request_base(&headers))))
}

async fn update_ds_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    type_update(
        &state,
        &identity,
        &request_base(&headers),
        DS_TYPE,
        id,
        &body,
    )
    .await
}

async fn delete_ds_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    type_delete(&state, &identity, DS_TYPE, id).await
}

// --- DS type ↔ module types -------------------------------------------------------

async fn link_module_type(
    state: &AppState,
    tenant: &str,
    ds_type_id: i64,
    sm_type_id: i64,
    mandatory: bool,
) -> AppResult<()> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_software_module_type
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE",
    )
    .bind(sm_type_id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "software module type {sm_type_id} not found"
        )));
    }
    sqlx::query(
        "INSERT INTO sp_ds_type_element (mandatory, distribution_set_type, software_module_type)
         VALUES ($1, $2, $3)
         ON CONFLICT (distribution_set_type, software_module_type)
         DO UPDATE SET mandatory = EXCLUDED.mandatory",
    )
    .bind(mandatory)
    .bind(ds_type_id)
    .bind(sm_type_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn list_module_types(
    state: &AppState,
    identity: &MgmtIdentity,
    ds_type_id: i64,
    base: &str,
    mandatory: bool,
) -> AppResult<Json<Value>> {
    load_type_row(state, &identity.tenant, DS_TYPE, ds_type_id).await?;
    let cond = if mandatory {
        "e.mandatory = TRUE"
    } else {
        "e.mandatory IS NOT TRUE"
    };
    let rows = sqlx::query(&format!(
        "{SM_TYPE_SELECT} JOIN sp_ds_type_element e ON e.software_module_type = y.id \
         WHERE e.distribution_set_type = $1 AND {cond} ORDER BY y.id"
    ))
    .bind(ds_type_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(Value::Array(
        rows.iter().map(|r| sm_type_json(r, base)).collect(),
    )))
}

async fn add_module_types(
    state: &AppState,
    identity: &MgmtIdentity,
    ds_type_id: i64,
    body: Value,
    mandatory: bool,
) -> AppResult<StatusCode> {
    load_type_row(state, &identity.tenant, DS_TYPE, ds_type_id).await?;
    let (items, _) = one_or_many(body)?;
    for item in &items {
        link_module_type(
            state,
            &identity.tenant,
            ds_type_id,
            require_id(item)?,
            mandatory,
        )
        .await?;
    }
    Ok(StatusCode::OK)
}

async fn remove_module_type(
    state: &AppState,
    identity: &MgmtIdentity,
    ds_type_id: i64,
    sm_type_id: i64,
    mandatory: bool,
) -> AppResult<StatusCode> {
    load_type_row(state, &identity.tenant, DS_TYPE, ds_type_id).await?;
    let cond = if mandatory {
        "mandatory = TRUE"
    } else {
        "mandatory IS NOT TRUE"
    };
    sqlx::query(&format!(
        "DELETE FROM sp_ds_type_element \
         WHERE distribution_set_type = $1 AND software_module_type = $2 AND {cond}"
    ))
    .bind(ds_type_id)
    .bind(sm_type_id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::OK)
}

async fn list_mandatory_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    list_module_types(&state, &identity, id, &request_base(&headers), true).await
}

async fn add_mandatory_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    add_module_types(&state, &identity, id, body, true).await
}

async fn remove_mandatory_module(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, sm_type_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    remove_module_type(&state, &identity, id, sm_type_id, true).await
}

async fn list_optional_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    list_module_types(&state, &identity, id, &request_base(&headers), false).await
}

async fn add_optional_modules(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    add_module_types(&state, &identity, id, body, false).await
}

async fn remove_optional_module(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, sm_type_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    remove_module_type(&state, &identity, id, sm_type_id, false).await
}

// --- software module types ----------------------------------------------------

async fn list_sm_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    type_list(&state, &identity, &params, &request_base(&headers), SM_TYPE).await
}

async fn create_sm_types(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::with_capacity(items.len());
    for item in &items {
        let name = require_str(item, "name")?;
        let key = require_str(item, "key")?;
        let max_assignments = item
            .get("maxAssignments")
            .and_then(Value::as_i64)
            .unwrap_or(1) as i32;
        let min_artifacts = item
            .get("minArtifacts")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32;
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO sp_software_module_type
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, description, colour, deleted, type_key,
              max_ds_assignments, min_artifacts)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, FALSE, $7, $8, $9)
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(&name)
        .bind(body_str(item, "description"))
        .bind(body_str(item, "colour"))
        .bind(&key)
        .bind(max_assignments)
        .bind(min_artifacts)
        .fetch_optional(&state.pool)
        .await?;
        let id = inserted.ok_or_else(|| {
            AppError::Conflict(format!(
                "software module type '{name}' / '{key}' already exists"
            ))
        })?;
        let row = load_type_row(&state, &identity.tenant, SM_TYPE, id).await?;
        created.push(sm_type_json(&row, &base));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_sm_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_type_row(&state, &identity.tenant, SM_TYPE, id).await?;
    Ok(Json(sm_type_json(&row, &request_base(&headers))))
}

async fn update_sm_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    type_update(
        &state,
        &identity,
        &request_base(&headers),
        SM_TYPE,
        id,
        &body,
    )
    .await
}

async fn delete_sm_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    type_delete(&state, &identity, SM_TYPE, id).await
}
