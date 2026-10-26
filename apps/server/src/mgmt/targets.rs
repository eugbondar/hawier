//! `/rest/v1/targets` — the exemplar resource module; other modules follow
//! the same shape (field map → list query, json! builders, array-create).

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hawier_api::codes::{ActionStatus, ActionType, TargetUpdateStatus};
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{
    action_detail_status, action_type_name, body_str, one_or_many, order_by, paged,
    parse_action_type, put_audit, request_base, require_str, rsql_filter, update_status_name,
    ListParams,
};
use crate::actions as action_store;
use crate::auth::MgmtIdentity;
use crate::db::model::generate_sec_token;
use crate::deploy_logic;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/targets", get(list_targets).post(create_targets))
        .route(
            "/rest/v1/targets/{controller_id}",
            get(get_target).put(update_target).delete(delete_target),
        )
        .route(
            "/rest/v1/targets/{controller_id}/attributes",
            get(get_attributes),
        )
        .route(
            "/rest/v1/targets/{controller_id}/actions",
            get(list_actions).delete(cancel_all_actions),
        )
        .route(
            "/rest/v1/targets/{controller_id}/actions/{action_id}",
            get(get_action).put(update_action).delete(cancel_action),
        )
        .route(
            "/rest/v1/targets/{controller_id}/actions/{action_id}/status",
            get(list_action_status),
        )
        .route(
            "/rest/v1/targets/{controller_id}/assignedDS",
            get(get_assigned_ds).post(post_assigned_ds),
        )
        .route(
            "/rest/v1/targets/{controller_id}/installedDS",
            get(get_installed_ds),
        )
        .route(
            "/rest/v1/targets/{controller_id}/tags",
            get(list_target_tags),
        )
        .route(
            "/rest/v1/targets/{controller_id}/metadata",
            get(list_metadata).post(create_metadata),
        )
        .route(
            "/rest/v1/targets/{controller_id}/metadata/{key}",
            get(get_metadata).put(put_metadata).delete(delete_metadata),
        )
        .route(
            "/rest/v1/targets/{controller_id}/targettype",
            post(assign_target_type).delete(unassign_target_type),
        )
        .route(
            "/rest/v1/targets/{controller_id}/autoConfirm",
            get(get_auto_confirm),
        )
        .route(
            "/rest/v1/targets/{controller_id}/autoConfirm/activate",
            post(activate_auto_confirm),
        )
        .route(
            "/rest/v1/targets/{controller_id}/autoConfirm/deactivate",
            post(deactivate_auto_confirm),
        )
}

// ---------------------------------------------------------------------------
// Field map + queries
// ---------------------------------------------------------------------------

static TARGET_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
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
        (
            "createdat",
            Field::Column {
                sql: "t.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "t.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastcontrollerrequestat",
            Field::Column {
                sql: "t.last_target_query",
                kind: Kind::Int,
            },
        ),
        (
            "lasttargetquery",
            Field::Column {
                sql: "t.last_target_query",
                kind: Kind::Int,
            },
        ),
        (
            // Liveness, defined once in SQL (0001_hawier.sql). The same
            // expression filters this list, orders it, fills the `pollState`
            // column and decides `pollStatus.overdue`, so the count on a
            // view, the badge in a row and the flag on the card cannot
            // disagree (UI/UX rule 6).
            //
            // Only `t` is referenced, and no `$N`: this string is spliced
            // into the COUNT query (`FROM sp_target t` alone) as well as the
            // list query, and into four other callers of `target_fields()`
            // whose bind offsets differ — rollout.rs binds the tenant as $1,
            // target_groups.rs as $4.
            "pollstate",
            Field::Column {
                sql: "hawier_target_poll_state(t.last_target_query, t.tenant)",
                kind: Kind::Text,
            },
        ),
        (
            "assigneddistributionset",
            Field::Column {
                sql: "t.assigned_distribution_set",
                kind: Kind::Int,
            },
        ),
        (
            // The console shows the name, so the name has to be sortable —
            // a column header answering 400 replaces the list with an error.
            // Correlated subqueries, not the JOIN aliases: the COUNT query
            // runs over `sp_target` alone and knows nothing about `ads`/`ids`.
            "assigneddistributionsetname",
            Field::Column {
                sql: "(SELECT ds.name || ' ' || ds.version FROM sp_distribution_set ds \
                       WHERE ds.id = t.assigned_distribution_set)",
                kind: Kind::Text,
            },
        ),
        (
            "installeddistributionsetname",
            Field::Column {
                sql: "(SELECT ds.name || ' ' || ds.version FROM sp_distribution_set ds \
                       WHERE ds.id = t.installed_distribution_set)",
                kind: Kind::Text,
            },
        ),
        (
            "installeddistributionset",
            Field::Column {
                sql: "t.installed_distribution_set",
                kind: Kind::Int,
            },
        ),
        (
            // Alias for the JSON key the console shows.
            "installedat",
            Field::Column {
                sql: "t.install_date",
                kind: Kind::Int,
            },
        ),
        (
            "installdate",
            Field::Column {
                sql: "t.install_date",
                kind: Kind::Int,
            },
        ),
        (
            "ipaddress",
            Field::Column {
                sql: "t.address",
                kind: Kind::Text,
            },
        ),
        (
            "address",
            Field::Column {
                sql: "t.address",
                kind: Kind::Text,
            },
        ),
        (
            "group",
            Field::Column {
                sql: "t.target_group",
                kind: Kind::Text,
            },
        ),
        (
            "updatestatus",
            Field::EnumColumn {
                sql: "t.update_status",
                map: &[
                    ("unknown", 0),
                    ("in_sync", 1),
                    ("pending", 2),
                    ("error", 3),
                    ("registered", 4),
                ],
            },
        ),
        (
            "attribute.*",
            Field::KeyValueSub {
                table: "sp_target_attributes",
                fk: "target",
                owner: "t.id",
                key_col: "attribute_key",
                val_col: "attribute_value",
            },
        ),
        (
            "metadata.*",
            Field::KeyValueSub {
                table: "sp_target_metadata",
                fk: "target",
                owner: "t.id",
                key_col: "meta_key",
                val_col: "meta_value",
            },
        ),
        (
            "tag",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_target_target_tag l \
                           JOIN sp_target_tag tag ON tag.id = l.tag \
                           WHERE l.target = t.id AND tag.name $CMP $V)",
            },
        ),
        (
            "assignedds.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = t.assigned_distribution_set AND ds.name $CMP $V)",
            },
        ),
        (
            "assignedds.version",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = t.assigned_distribution_set AND ds.version $CMP $V)",
            },
        ),
        (
            "installedds.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = t.installed_distribution_set AND ds.name $CMP $V)",
            },
        ),
        (
            "installedds.version",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = t.installed_distribution_set AND ds.version $CMP $V)",
            },
        ),
        (
            "targettype.key",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_target_type y \
                           WHERE y.id = t.target_type AND y.type_key $CMP $V)",
            },
        ),
        (
            "targettype.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_target_type y \
                           WHERE y.id = t.target_type AND y.name $CMP $V)",
            },
        ),
    ])
});

/// Validate an RSQL expression against the target field map (used by
/// target filters and the auto-assign / rollout engines).
pub fn validate_rsql(query: &str) -> AppResult<()> {
    crate::rsql::to_sql(query, &TARGET_FIELDS, 0).map(|_| ())
}

/// Expose the field map so other modules can compile target filter queries.
pub fn target_fields() -> &'static FieldMap {
    &TARGET_FIELDS
}

const TARGET_SELECT: &str = "SELECT t.id, t.created_at, t.created_by, t.last_modified_at,
       t.last_modified_by, t.description, t.name, t.controller_id, t.sec_token,
       t.assigned_distribution_set, t.install_date, t.address, t.last_target_query,
       t.request_controller_attributes, t.installed_distribution_set, t.update_status,
       t.target_type, t.target_group,
       tt.name AS target_type_name,
       ads.name || ' ' || ads.version AS assigned_ds_label,
       ids.name || ' ' || ids.version AS installed_ds_label,
       EXISTS (SELECT 1 FROM sp_target_conf_status c WHERE c.target = t.id) AS auto_confirm_active,
       hawier_target_poll_state(t.last_target_query, t.tenant) AS poll_state,
       t.last_target_query + hawier_polling_millis(t.tenant) AS next_expected_at
FROM sp_target t
LEFT JOIN sp_target_type tt ON tt.id = t.target_type
LEFT JOIN sp_distribution_set ads ON ads.id = t.assigned_distribution_set
LEFT JOIN sp_distribution_set ids ON ids.id = t.installed_distribution_set";

fn target_json(row: &PgRow, base: &str, full: bool) -> Value {
    let controller_id: String = row.get("controller_id");
    // Option rather than String: the CASE is total and t.tenant is NOT NULL,
    // so this cannot be null — but `get::<String, _>` on a null panics inside
    // the handler and resets the connection, while a null renders as an empty
    // cell. Defensive without inventing a fact.
    let poll_state: Option<String> = row.get("poll_state");
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
        // hawier extension (API_NOTES §5): hawkBit has no scalar liveness
        // field, and its `pollStatus` is an object served only in the `full`
        // representation — a list column can use neither. The console asks
        // for `compact`, so this is the only shape that reaches a row.
        "pollState": poll_state,
        // hawier extension (API_NOTES §5): hawkBit exposes the assigned and
        // installed sets only behind links, which a list column cannot use.
        "assignedDistributionSet": row.get::<Option<i64>, _>("assigned_distribution_set"),
        "assignedDistributionSetName": row.get::<Option<String>, _>("assigned_ds_label"),
        "installedDistributionSet": row.get::<Option<i64>, _>("installed_distribution_set"),
        "installedDistributionSetName": row.get::<Option<String>, _>("installed_ds_label"),
        "targetType": row.get::<Option<i64>, _>("target_type"),
        "targetTypeName": row.get::<Option<String>, _>("target_type_name"),
        "autoConfirmActive": row.get::<bool, _>("auto_confirm_active"),
        "_links": { "self": { "href": self_href } }
    });
    put_audit(&mut value, row);

    if full {
        let object = value.as_object_mut().unwrap();
        object.insert(
            "securityToken".into(),
            json!(row.get::<String, _>("sec_token")),
        );
        // hawkBit keeps two dates apart and so do we: nextExpectedRequestAt
        // is `last + pollingTime` — when the device said it would be back —
        // while "overdue" allows the further pollingOverdueTime of grace. So
        // the flag is deliberately NOT `now > nextExpectedRequestAt`; it is
        // the same column the list filtered on, and the card and the list
        // cannot disagree.
        if let (Some(last), Some(next)) = (
            row.get::<Option<i64>, _>("last_target_query"),
            row.get::<Option<i64>, _>("next_expected_at"),
        ) {
            object.insert(
                "pollStatus".into(),
                json!({
                    "lastRequestAt": last,
                    "nextExpectedRequestAt": next,
                    "overdue": poll_state.as_deref() == Some("overdue"),
                }),
            );
        }
        let links = object.get_mut("_links").unwrap().as_object_mut().unwrap();
        for rel in [
            "assignedDS",
            "installedDS",
            "attributes",
            "actions",
            "metadata",
            "autoConfirm",
        ] {
            let href = format!("{base}/rest/v1/targets/{controller_id}/{rel}");
            links.insert(rel.to_string(), json!({ "href": href }));
        }
    }
    value
}

async fn load_target_row(state: &AppState, tenant: &str, controller_id: &str) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{TARGET_SELECT} WHERE UPPER(t.tenant) = UPPER($1) AND t.controller_id = $2"
    ))
    .bind(tenant)
    .bind(controller_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("target {controller_id} not found")))
}

fn target_id_of(row: &PgRow) -> i64 {
    row.get("id")
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

async fn list_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &TARGET_FIELDS, 1)?;
    // `tag` filters via an EXISTS template (any tag matches), which cannot be
    // ordered by; sorting groups targets by their alphabetically first tag.
    let order = super::order_by_with(
        params.sort.as_deref(),
        &TARGET_FIELDS,
        "t.id ASC",
        &[(
            "tag",
            "(SELECT MIN(tg.name) FROM sp_target_target_tag l
              JOIN sp_target_tag tg ON tg.id = l.tag WHERE l.target = t.id)",
        )],
    )?;

    let count_sql =
        format!("SELECT COUNT(*) FROM sp_target t WHERE UPPER(t.tenant) = UPPER($1){filter_sql}");
    let total: i64 = bind_values(sqlx::query(&count_sql).bind(&identity.tenant), &binds)
        .fetch_one(&state.pool)
        .await?
        .get(0);

    let list_sql = format!(
        "{TARGET_SELECT} WHERE UPPER(t.tenant) = UPPER($1){filter_sql} {order} \
         LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    );
    let rows = bind_values(sqlx::query(&list_sql).bind(&identity.tenant), &binds)
        .fetch_all(&state.pool)
        .await?;

    let full = params.full();
    let content = rows.iter().map(|r| target_json(r, &base, full)).collect();
    Ok(paged(content, total))
}

async fn create_targets(
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
        let controller_id = require_str(item, "controllerId")?;
        let name = body_str(item, "name").unwrap_or_else(|| controller_id.clone());
        let sec_token = body_str(item, "securityToken").unwrap_or_else(generate_sec_token);
        let target_type = item.get("targetType").and_then(|v| v.as_i64());

        // request_controller_attributes starts TRUE, as hawkBit's JpaTarget
        // does: a device provisioned here reports what it is on its first
        // poll, just like one that registered itself.
        let inserted = sqlx::query(
            "INSERT INTO sp_target
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, description, controller_id, sec_token, address,
              request_controller_attributes, update_status, target_type, target_group)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7, $8, TRUE, $9, $10, $11)
             ON CONFLICT DO NOTHING
             RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(&name)
        .bind(body_str(item, "description"))
        .bind(&controller_id)
        .bind(&sec_token)
        .bind(body_str(item, "address"))
        .bind(TargetUpdateStatus::Unknown.code())
        .bind(target_type)
        .bind(body_str(item, "group"))
        .fetch_optional(&state.pool)
        .await?;
        if inserted.is_none() {
            return Err(AppError::Conflict(format!(
                "target {controller_id} already exists"
            )));
        }

        let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
        created.push(target_json(&row, &base, true));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_target(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    Ok(Json(target_json(&row, &request_base(&headers), true)))
}

async fn update_target(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let id = target_id_of(&row);

    let mut sets = vec![
        "last_modified_at = $1".to_string(),
        "last_modified_by = $2".to_string(),
        "optlock_revision = COALESCE(optlock_revision, 0) + 1".to_string(),
    ];
    let mut arg_index = 2;
    let mut texts: Vec<(usize, Option<String>)> = Vec::new();

    for (json_key, column) in [
        ("name", "name"),
        ("description", "description"),
        ("address", "address"),
        ("securityToken", "sec_token"),
        ("group", "target_group"),
    ] {
        if body.get(json_key).is_some() {
            arg_index += 1;
            sets.push(format!("{column} = ${arg_index}"));
            texts.push((arg_index, body_str(&body, json_key)));
        }
    }
    let request_attributes = body.get("requestAttributes").and_then(|v| v.as_bool());
    if request_attributes.is_some() {
        arg_index += 1;
        sets.push(format!("request_controller_attributes = ${arg_index}"));
    }
    let target_type = body.get("targetType").and_then(|v| v.as_i64());
    if target_type.is_some() {
        arg_index += 1;
        sets.push(format!("target_type = ${arg_index}"));
    }

    let sql = format!(
        "UPDATE sp_target SET {} WHERE id = ${}",
        sets.join(", "),
        arg_index + 1
    );
    let mut query = sqlx::query(&sql)
        .bind(now_millis())
        .bind(&identity.username);
    for (_, value) in &texts {
        query = query.bind(value.clone());
    }
    if let Some(flag) = request_attributes {
        query = query.bind(flag);
    }
    if let Some(tt) = target_type {
        query = query.bind(tt);
    }
    query.bind(id).execute(&state.pool).await?;

    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    Ok(Json(target_json(&row, &request_base(&headers), true)))
}

async fn delete_target(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
) -> AppResult<StatusCode> {
    let result =
        sqlx::query("DELETE FROM sp_target WHERE UPPER(tenant) = UPPER($1) AND controller_id = $2")
            .bind(&identity.tenant)
            .bind(&controller_id)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "target {controller_id} not found"
        )));
    }
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Attributes / tags / metadata
// ---------------------------------------------------------------------------

async fn get_attributes(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let rows = sqlx::query(
        "SELECT attribute_key, attribute_value FROM sp_target_attributes
         WHERE target = $1 ORDER BY attribute_key",
    )
    .bind(target_id_of(&row))
    .fetch_all(&state.pool)
    .await?;
    let mut map = serde_json::Map::new();
    for r in rows {
        map.insert(
            r.get::<String, _>("attribute_key"),
            json!(r
                .get::<Option<String>, _>("attribute_value")
                .unwrap_or_default()),
        );
    }
    Ok(Json(Value::Object(map)))
}

async fn list_target_tags(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let rows = sqlx::query(
        "SELECT g.id, g.created_at, g.created_by, g.last_modified_at, g.last_modified_by,
                g.name, g.description, g.colour
         FROM sp_target_tag g JOIN sp_target_target_tag l ON l.tag = g.id
         WHERE l.target = $1 ORDER BY g.id",
    )
    .bind(target_id_of(&row))
    .fetch_all(&state.pool)
    .await?;
    let base = request_base(&headers);
    Ok(Json(Value::Array(
        rows.iter()
            .map(|r| super::repr::tag_json(r, &base, "targettags"))
            .collect(),
    )))
}

async fn list_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let target_id = target_id_of(&row);
    // Metadata is unbounded per object, so it pages like any other list —
    // a console panel must never quietly show the first N of many.
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sp_target_metadata WHERE target = $1")
            .bind(target_id)
            .fetch_one(&state.pool)
            .await?;
    let rows = sqlx::query(
        "SELECT meta_key, meta_value FROM sp_target_metadata WHERE target = $1
         ORDER BY meta_key LIMIT $2 OFFSET $3",
    )
    .bind(target_id)
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
    Ok(paged(content, total).0.into())
}

async fn create_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let (items, _) = one_or_many(body)?;
    let mut created = Vec::new();
    for item in &items {
        let key = require_str(item, "key")?;
        let value = body_str(item, "value").unwrap_or_default();
        sqlx::query(
            "INSERT INTO sp_target_metadata (target, meta_key, meta_value) VALUES ($1, $2, $3)
             ON CONFLICT (target, meta_key) DO UPDATE SET meta_value = EXCLUDED.meta_value",
        )
        .bind(target_id_of(&row))
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
    Path((controller_id, key)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let entry = sqlx::query(
        "SELECT meta_key, meta_value FROM sp_target_metadata
         WHERE target = $1 AND meta_key = $2",
    )
    .bind(target_id_of(&row))
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("metadata '{key}' not found")))?;
    Ok(Json(json!({
        "key": entry.get::<String, _>("meta_key"),
        "value": entry.get::<Option<String>, _>("meta_value"),
    })))
}

async fn put_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((controller_id, key)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let value = body_str(&body, "value").unwrap_or_default();
    let result = sqlx::query(
        "UPDATE sp_target_metadata SET meta_value = $1 WHERE target = $2 AND meta_key = $3",
    )
    .bind(&value)
    .bind(target_id_of(&row))
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
    Path((controller_id, key)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let row = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let result = sqlx::query("DELETE FROM sp_target_metadata WHERE target = $1 AND meta_key = $2")
        .bind(target_id_of(&row))
        .bind(&key)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("metadata '{key}' not found")));
    }
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

static ACTION_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
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
            "status",
            Field::EnumColumn {
                sql: "a.active",
                // pending=active(true=1), finished=inactive(0) — special-cased below.
                map: &[("pending", 1), ("finished", 0)],
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
            "distributionset.name",
            Field::Exists {
                template: "EXISTS (SELECT 1 FROM sp_distribution_set ds \
                           WHERE ds.id = a.distribution_set AND ds.name $CMP $V)",
            },
        ),
    ])
});

const ACTION_SELECT: &str = "SELECT a.id, a.created_at, a.created_by, a.last_modified_at,
       a.last_modified_by, a.active, a.status, a.action_type, a.forced_time,
       a.distribution_set, a.target, a.rollout, a.rollout_group, a.weight,
       a.initiated_by, a.external_ref, a.last_action_status_code,
       a.maintenance_cron_schedule, a.maintenance_duration, a.maintenance_time_zone,
       r.name AS rollout_name, ds.name AS ds_name, ds.version AS ds_version
FROM sp_action a
LEFT JOIN sp_rollout r ON r.id = a.rollout
LEFT JOIN sp_distribution_set ds ON ds.id = a.distribution_set";

pub fn action_json(row: &PgRow, base: &str, controller_id: &str) -> Value {
    let id: i64 = row.get("id");
    let active = row.get::<Option<bool>, _>("active").unwrap_or(false);
    let status_code: i32 = row.get("status");
    let is_cancel = status_code == ActionStatus::Canceling.code()
        || status_code == ActionStatus::Canceled.code();
    let mut value = json!({
        "id": id,
        "type": if is_cancel { "cancel" } else { "update" },
        "status": if active { "pending" } else { "finished" },
        "detailStatus": action_detail_status(status_code),
        "forceType": action_type_name(row.get::<i32, _>("action_type")),
        "weight": row.get::<i32, _>("weight"),
        "rollout": row.get::<Option<i64>, _>("rollout"),
        "rolloutName": row.get::<Option<String>, _>("rollout_name"),
        "externalRef": row.get::<Option<String>, _>("external_ref"),
        "lastStatusCode": row.get::<Option<i32>, _>("last_action_status_code"),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/targets/{controller_id}/actions/{id}") },
            "distributionset": super::ds_link(row, base),
            "status": {
                "href": format!("{base}/rest/v1/targets/{controller_id}/actions/{id}/status")
            },
            "target": { "href": format!("{base}/rest/v1/targets/{controller_id}") }
        }
    });
    put_audit(&mut value, row);
    value
}

async fn list_actions(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let target_id = target_id_of(&target);
    let base = request_base(&headers);

    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &ACTION_FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &ACTION_FIELDS, "a.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_action a WHERE a.target = $1{filter_sql}"
        ))
        .bind(target_id),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{ACTION_SELECT} WHERE a.target = $1{filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(target_id),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let content = rows
        .iter()
        .map(|r| action_json(r, &base, &controller_id))
        .collect();
    Ok(paged(content, total))
}

async fn get_action(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((controller_id, action_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let row = sqlx::query(&format!(
        "{ACTION_SELECT} WHERE a.id = $1 AND a.target = $2"
    ))
    .bind(action_id)
    .bind(target_id_of(&target))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("action {action_id} not found")))?;
    Ok(Json(action_json(
        &row,
        &request_base(&headers),
        &controller_id,
    )))
}

/// PUT switches a soft action to forced (`{"forceType": "forced"}`).
async fn update_action(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((controller_id, action_id)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let force_type = require_str(&body, "forceType")?;
    if parse_action_type(&force_type)? != ActionType::Forced {
        return Err(AppError::BadRequest(
            "only forceType 'forced' is supported".into(),
        ));
    }
    let result = sqlx::query(
        "UPDATE sp_action SET action_type = $1, last_modified_at = $2, last_modified_by = $3
         WHERE id = $4 AND target = $5 AND active = TRUE",
    )
    .bind(ActionType::Forced.code())
    .bind(now_millis())
    .bind(&identity.username)
    .bind(action_id)
    .bind(target_id_of(&target))
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "active action {action_id} not found"
        )));
    }
    let row = sqlx::query(&format!("{ACTION_SELECT} WHERE a.id = $1"))
        .bind(action_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(action_json(
        &row,
        &request_base(&headers),
        &controller_id,
    )))
}

#[derive(serde::Deserialize)]
struct CancelParams {
    #[serde(default)]
    force: bool,
}

/// DELETE = cancel: soft-cancel (CANCELING) or force-quit with `?force=true`.
/// `DELETE /targets/{id}/actions` — cancel every active action of one
/// target (hawkBit has this; hawier answered 405 until now). `?force=true`
/// closes them outright instead of asking the device.
async fn cancel_all_actions(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    Query(params): Query<CancelParams>,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let target_id = target_id_of(&target);
    let active: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_action WHERE target = $1 AND active = TRUE ORDER BY id",
    )
    .bind(target_id)
    .fetch_all(&state.pool)
    .await?;

    let mut canceled = 0usize;
    for action_id in &active {
        match cancel_one_action(&state, &identity, target_id, *action_id, params.force).await {
            Ok(()) => canceled += 1,
            // Already closed or already canceling: not a failure of the
            // whole call, the caller asked for "make them stop".
            Err(AppError::Gone(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(Json(json!({ "canceled": canceled, "total": active.len() })))
}

async fn cancel_action(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((controller_id, action_id)): Path<(String, i64)>,
    Query(params): Query<CancelParams>,
) -> AppResult<StatusCode> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    cancel_one_action(
        &state,
        &identity,
        target_id_of(&target),
        action_id,
        params.force,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Cancel one action: soft (CANCELING — the device is asked and confirms)
/// or forced (CANCELED outright). Shared by the single- and all-actions
/// endpoints.
async fn cancel_one_action(
    state: &AppState,
    identity: &MgmtIdentity,
    target_id: i64,
    action_id: i64,
    force: bool,
) -> AppResult<()> {
    let mut tx = state.pool.begin().await?;
    let action = action_store::lock_action(&mut tx, &identity.tenant, action_id)
        .await?
        .filter(|a| a.target == target_id)
        .ok_or_else(|| AppError::NotFound(format!("action {action_id} not found")))?;

    if !action.is_active() {
        return Err(AppError::Gone(format!(
            "action {action_id} is already closed"
        )));
    }

    let (status, active, message) = if force {
        (
            ActionStatus::Canceled,
            false,
            "Update Server: cancellation forced by operator",
        )
    } else {
        if action.status == ActionStatus::Canceling.code() {
            return Err(AppError::Gone(format!(
                "action {action_id} is already canceling"
            )));
        }
        (
            ActionStatus::Canceling,
            true,
            "Update Server: cancellation requested",
        )
    };

    action_store::insert_status(
        &mut tx,
        &identity.tenant,
        action.id,
        status,
        None,
        None,
        &identity.username,
        &[message.to_string()],
    )
    .await?;
    action_store::set_action_status(&mut tx, action.id, status, active, &identity.username).await?;
    tx.commit().await?;
    Ok(())
}

async fn list_action_status(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((controller_id, action_id)): Path<(String, i64)>,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let owns: Option<i64> =
        sqlx::query_scalar("SELECT id FROM sp_action WHERE id = $1 AND target = $2")
            .bind(action_id)
            .bind(target_id_of(&target))
            .fetch_optional(&state.pool)
            .await?;
    if owns.is_none() {
        return Err(AppError::NotFound(format!("action {action_id} not found")));
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_action_status WHERE action = $1")
        .bind(action_id)
        .fetch_one(&state.pool)
        .await?;

    let rows = sqlx::query(&format!(
        "SELECT s.id, s.created_at, s.target_occurred_at, s.status, s.code,
                s.last_modified_at, s.optlock_revision
         FROM sp_action_status s WHERE s.action = $1
         ORDER BY s.id DESC LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    ))
    .bind(action_id)
    .fetch_all(&state.pool)
    .await?;

    let mut content = Vec::with_capacity(rows.len());
    for row in &rows {
        let status_id: i64 = row.get("id");
        let messages: Vec<String> = sqlx::query_scalar(
            "SELECT detail_message FROM sp_action_status_messages WHERE action_status = $1",
        )
        .bind(status_id)
        .fetch_all(&state.pool)
        .await?;
        // Additive to hawkBit (API_NOTES §5): a controller repeating a status
        // verbatim does not get a new row, it bumps the existing one's
        // revision. `repeats` is how many times that happened, `lastReportedAt`
        // is when it last did — without them a device stuck in a retry loop
        // would look like a device that reported once and went quiet.
        let repeats = row.get::<Option<i64>, _>("optlock_revision").unwrap_or(1) - 1;
        content.push(json!({
            "id": status_id,
            "type": action_detail_status(row.get::<i32, _>("status")),
            "messages": messages,
            "reportedAt": row.get::<i64, _>("target_occurred_at"),
            "timestamp": row.get::<Option<i64>, _>("created_at"),
            "code": row.get::<Option<i32>, _>("code"),
            "repeats": repeats.max(0),
            "lastReportedAt": row.get::<Option<i64>, _>("last_modified_at"),
        }));
    }
    Ok(paged(content, total))
}

// ---------------------------------------------------------------------------
// assignedDS / installedDS
// ---------------------------------------------------------------------------

async fn ds_json_by_id(
    state: &AppState,
    tenant: &str,
    ds_id: i64,
    base: &str,
) -> AppResult<Option<Value>> {
    let row = sqlx::query(&format!(
        "{} WHERE d.id = $1 AND UPPER(d.tenant) = UPPER($2)",
        super::repr::DS_SELECT
    ))
    .bind(ds_id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        Some(row) => {
            let modules = super::repr::ds_modules(&state.pool, ds_id, base).await?;
            Ok(Some(super::repr::ds_json(&row, base, modules)))
        }
        None => Ok(None),
    }
}

async fn get_assigned_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let assigned: Option<i64> = target.get("assigned_distribution_set");
    match assigned {
        Some(ds_id) => {
            let ds = ds_json_by_id(&state, &identity.tenant, ds_id, &request_base(&headers))
                .await?
                .ok_or_else(|| AppError::NotFound("assigned DS vanished".into()))?;
            Ok(Json(ds).into_response())
        }
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn get_installed_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let installed: Option<i64> = target.get("installed_distribution_set");
    match installed {
        Some(ds_id) => {
            let ds = ds_json_by_id(&state, &identity.tenant, ds_id, &request_base(&headers))
                .await?
                .ok_or_else(|| AppError::NotFound("installed DS vanished".into()))?;
            Ok(Json(ds).into_response())
        }
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// POST assignedDS: accepts one assignment object or a list.
async fn post_assigned_ds(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let target_id = target_id_of(&target);
    let currently_assigned: Option<i64> = target.get("assigned_distribution_set");
    let (items, _) = one_or_many(body)?;
    let base = request_base(&headers);
    let multi_assignments = crate::tenant::config_bool(
        &state.pool,
        &identity.tenant,
        crate::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        false,
    )
    .await?;

    // hawkBit refuses the whole request when any set is incompatible, before
    // anything is written (deployment-assignment spec).
    let requested: Vec<i64> = items
        .iter()
        .filter_map(|item| item.get("id").and_then(|v| v.as_i64()))
        .collect();
    deploy_logic::ensure_sets_compatible(
        &mut *state.pool.acquire().await?,
        target_id,
        &requested,
        deploy_logic::typed_only(&state.pool, &identity.tenant).await?,
    )
    .await?;

    let mut assigned_actions = Vec::new();
    let mut already_assigned = 0usize;

    for item in &items {
        let ds_id = item
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
        // In multi-assignment mode a repeated DS still creates a new action
        // (hawkBit skips the already-assigned shortcut there).
        if currently_assigned == Some(ds_id) && !multi_assignments {
            already_assigned += 1;
            continue;
        }

        let action_type = match body_str(item, "type") {
            Some(t) => parse_action_type(&t)?,
            None => ActionType::Forced,
        };
        let forced_time = item.get("forcetime").and_then(|v| v.as_i64());
        let weight = item
            .get("weight")
            .and_then(|v| v.as_i64())
            .map(|w| w as i32)
            .unwrap_or(deploy_logic::DEFAULT_WEIGHT);
        let confirmation_required = item.get("confirmationRequired").and_then(|v| v.as_bool());
        let maintenance_window =
            item.get("maintenanceWindow")
                .map(|mw| deploy_logic::MaintenanceWindow {
                    schedule: body_str(mw, "schedule").unwrap_or_default(),
                    duration: body_str(mw, "duration").unwrap_or_default(),
                    time_zone: body_str(mw, "timezone").unwrap_or_default(),
                });

        let outcome = deploy_logic::assign(
            &state.pool,
            &state.incidents,
            &identity.tenant,
            &deploy_logic::AssignRequest {
                target_id,
                ds_id,
                action_type,
                forced_time,
                weight,
                initiated_by: identity.username.clone(),
                confirmation_required,
                maintenance_window,
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

// ---------------------------------------------------------------------------
// Target type assignment
// ---------------------------------------------------------------------------

async fn assign_target_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<StatusCode> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let type_id = body
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::BadRequest("'id' (target type) is required".into()))?;
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_target_type WHERE id = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(type_id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "target type {type_id} not found"
        )));
    }
    sqlx::query("UPDATE sp_target SET target_type = $1 WHERE id = $2")
        .bind(type_id)
        .bind(target_id_of(&target))
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

async fn unassign_target_type(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
) -> AppResult<StatusCode> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    sqlx::query("UPDATE sp_target SET target_type = NULL WHERE id = $1")
        .bind(target_id_of(&target))
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Auto-confirm (mgmt side)
// ---------------------------------------------------------------------------

async fn get_auto_confirm(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
) -> AppResult<Json<Value>> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let row = sqlx::query(
        "SELECT initiator, remark, created_at FROM sp_target_conf_status
         WHERE target = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(target_id_of(&target))
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(match row {
        Some(row) => json!({
            "active": true,
            "initiator": row.get::<Option<String>, _>("initiator"),
            "remark": row.get::<Option<String>, _>("remark"),
            "activatedAt": row.get::<Option<i64>, _>("created_at"),
        }),
        None => json!({ "active": false }),
    }))
}

async fn activate_auto_confirm(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
    body: Option<Json<Value>>,
) -> AppResult<StatusCode> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    let target_id = target_id_of(&target);
    let body = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));

    let mut tx = state.pool.begin().await?;
    let already: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sp_target_conf_status WHERE target = $1)")
            .bind(target_id)
            .fetch_one(&mut *tx)
            .await?;
    if already {
        return Err(AppError::Conflict(
            "auto-confirmation already active".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO sp_target_conf_status
         (target, initiator, remark, created_at, created_by,
          last_modified_at, last_modified_by, optlock_revision, tenant)
         VALUES ($1, $2, $3, $4, $5, $4, $5, 1, UPPER($6))",
    )
    .bind(target_id)
    .bind(body_str(&body, "initiator").unwrap_or_else(|| identity.username.clone()))
    .bind(body_str(&body, "remark"))
    .bind(now_millis())
    .bind(&identity.username)
    .bind(&identity.tenant)
    .execute(&mut *tx)
    .await?;

    // Release any gated actions right away (same as the DDI-side activate).
    let pending = sqlx::query(
        "SELECT id FROM sp_action WHERE target = $1 AND active = TRUE AND status = $2 FOR UPDATE",
    )
    .bind(target_id)
    .bind(ActionStatus::WaitForConfirmation.code())
    .fetch_all(&mut *tx)
    .await?;
    for row in &pending {
        let action_id: i64 = row.get("id");
        action_store::insert_status(
            &mut tx,
            &identity.tenant,
            action_id,
            ActionStatus::Running,
            None,
            None,
            &identity.username,
            &["Action auto-confirmed by activated auto-confirmation.".to_string()],
        )
        .await?;
        action_store::set_action_status(
            &mut tx,
            action_id,
            ActionStatus::Running,
            true,
            &identity.username,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::OK)
}

async fn deactivate_auto_confirm(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(controller_id): Path<String>,
) -> AppResult<StatusCode> {
    let target = load_target_row(&state, &identity.tenant, &controller_id).await?;
    sqlx::query("DELETE FROM sp_target_conf_status WHERE target = $1")
        .bind(target_id_of(&target))
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}
