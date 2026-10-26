//! `/rest/v1/roles` and `/rest/v1/grants` — who may do what.
//!
//! hawier extension (recorded in API_NOTES): hawkBit keeps users in
//! configuration and has no role model at all. Ordinary resources, so the
//! console gets them for free (UI/UX rule 11).
//!
//! Both are themselves `SYSTEM` resources, so handing someone the whole
//! repository does not let them grant themselves the rest.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use hawier_api::ext::permissions;
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
        .route("/rest/v1/roles", get(list).post(create))
        .route(
            "/rest/v1/roles/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route("/rest/v1/grants", get(list_grants).post(create_grant))
        .route("/rest/v1/grants/{id}", axum::routing::delete(delete_grant))
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
            "description",
            Field::Column {
                sql: "r.description",
                kind: Kind::Text,
            },
        ),
        (
            "permissions",
            Field::Column {
                sql: "r.permissions",
                kind: Kind::Text,
            },
        ),
        (
            // A declared column, so it has to be sortable and filterable like
            // any other (UI/UX rule 25).
            "holders",
            Field::Column {
                sql: "(SELECT COUNT(*) FROM hawier_grant g WHERE g.role = r.id)",
                kind: Kind::Int,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM r.created_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

static GRANT_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "g.id::text",
                kind: Kind::Text,
            },
        ),
        (
            "username",
            Field::Column {
                sql: "g.username",
                kind: Kind::Text,
            },
        ),
        (
            "role",
            Field::Column {
                sql: "g.role::text",
                kind: Kind::Text,
            },
        ),
        (
            "rolename",
            Field::Column {
                sql: "r.name",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM g.created_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

const SELECT: &str = "SELECT r.id, r.name, r.description, r.permissions,
       (EXTRACT(EPOCH FROM r.created_at) * 1000)::BIGINT AS created_ms,
       (SELECT COUNT(*) FROM hawier_grant g WHERE g.role = r.id) AS holders
FROM hawier_role r";

const GRANT_SELECT: &str = "SELECT g.id, g.username, g.role, r.name AS role_name,
       (EXTRACT(EPOCH FROM g.created_at) * 1000)::BIGINT AS created_ms
FROM hawier_grant g JOIN hawier_role r ON r.id = g.role";

fn role_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "name": row.get::<String, _>("name"),
        "description": row.get::<String, _>("description"),
        "permissions": row.get::<String, _>("permissions"),
        "holders": row.get::<i64, _>("holders"),
        "createdAt": row.get::<Option<i64>, _>("created_ms"),
    })
}

fn grant_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "username": row.get::<String, _>("username"),
        "role": row.get::<i64, _>("role").to_string(),
        "roleName": row.get::<String, _>("role_name"),
        "createdAt": row.get::<Option<i64>, _>("created_ms"),
    })
}

/// Refuse a permission nothing checks. A role granting `DELETE_EVERYTHING`
/// would look powerful and do nothing, which is the worst way for an access
/// control list to be wrong.
fn validate_permissions(raw: &str) -> AppResult<String> {
    let mut cleaned = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if !permissions::KNOWN.contains(&entry) {
            return Err(AppError::BadRequest(format!(
                "unknown permission '{entry}'; known permissions are {}",
                permissions::KNOWN.join(", ")
            )));
        }
        if !cleaned.contains(&entry.to_string()) {
            cleaned.push(entry.to_string());
        }
    }
    if cleaned.is_empty() {
        return Err(AppError::BadRequest(
            "a role with no permissions grants nothing; give it at least one".to_string(),
        ));
    }
    Ok(cleaned.join(","))
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "r.name ASC")?;
    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_role r WHERE UPPER(r.tenant) = UPPER($1){filter_sql}"
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
    Ok(paged(rows.iter().map(role_json).collect(), total))
}

async fn list_grants(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &GRANT_FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &GRANT_FIELDS, "g.username ASC")?;
    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_grant g JOIN hawier_role r ON r.id = g.role
             WHERE UPPER(g.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);
    let rows = bind_values(
        sqlx::query(&format!(
            "{GRANT_SELECT} WHERE UPPER(g.tenant) = UPPER($1){filter_sql} {order} \
             LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(paged(rows.iter().map(grant_json).collect(), total))
}

async fn load(state: &AppState, tenant: &str, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{SELECT} WHERE r.id = $2 AND UPPER(r.tenant) = UPPER($1)"
    ))
    .bind(tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("role {id} not found")))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    Ok(Json(role_json(&load(&state, &identity.tenant, id).await?)))
}

async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Value>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let name = super::require_str(&body, "name")?;
    let permissions = validate_permissions(
        body.get("permissions")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )?;
    let id = state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_role (id, tenant, name, description, permissions)
         VALUES ($1, UPPER($2), $3, $4, $5)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .bind(&name)
    .bind(
        body.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )
    .bind(&permissions)
    .execute(&state.pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Conflict(format!("role '{name}' already exists"))
        }
        _ => AppError::Db(e),
    })?;
    // The first role in a tenant switches enforcement on, so nobody should
    // wait out a cache to find that out.
    state.authz_cache.invalidate();
    Ok((
        axum::http::StatusCode::CREATED,
        Json(role_json(&load(&state, &identity.tenant, id).await?)),
    ))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load(&state, &identity.tenant, id).await?;
    let permissions = match body.get("permissions").and_then(|v| v.as_str()) {
        Some(raw) => Some(validate_permissions(raw)?),
        None => None,
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE hawier_role SET
            name = COALESCE($3, name),
            description = COALESCE($4, description),
            permissions = COALESCE($5, permissions),
            updated_at = now()
         WHERE id = $2 AND UPPER(tenant) = UPPER($1)",
    )
    .bind(&identity.tenant)
    .bind(id)
    .bind(body.get("name").and_then(|v| v.as_str()))
    .bind(body.get("description").and_then(|v| v.as_str()))
    .bind(permissions)
    .execute(&mut *tx)
    .await?;
    // Editing the administrator role down to a reader is the quiet way to
    // revoke administration from everyone holding it.
    keep_an_administrator(&mut tx, &identity.tenant).await?;
    tx.commit().await?;
    state.authz_cache.invalidate();
    Ok(Json(role_json(&load(&state, &identity.tenant, id).await?)))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<axum::http::StatusCode> {
    let mut tx = state.pool.begin().await?;
    let deleted =
        sqlx::query("DELETE FROM hawier_role WHERE id = $2 AND UPPER(tenant) = UPPER($1)")
            .bind(&identity.tenant)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound(format!("role {id} not found")));
    }
    // Grants cascade with the role, so deleting one can revoke the last
    // administrator without ever touching a grant.
    keep_an_administrator(&mut tx, &identity.tenant).await?;
    tx.commit().await?;
    state.authz_cache.invalidate();
    Ok(axum::http::StatusCode::OK)
}

/// A tenant that has any grants must have someone who can administer it.
///
/// Checked *after* the write, inside its transaction, so a change that would
/// leave nobody holding `UPDATE_SYSTEM` rolls back instead of bricking the
/// tenant. This closes the four ways to do it: assigning the first role to
/// somebody else, revoking the last administrator's grant, editing the
/// administrator role down, and deleting it (grants cascade with it).
///
/// `permissions` is stored as the comma-joined output of
/// `validate_permissions`, so splitting on ',' is exact rather than a
/// substring match that would also accept `NOT_UPDATE_SYSTEM`.
async fn keep_an_administrator(tx: &mut sqlx::PgConnection, tenant: &str) -> AppResult<()> {
    let grants: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hawier_grant WHERE UPPER(tenant) = UPPER($1)")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await?;
    if grants == 0 {
        // Nobody is subject to anything, so authorisation is off again and
        // there is nothing to lock anyone out of.
        return Ok(());
    }
    let admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hawier_grant g JOIN hawier_role r ON r.id = g.role
         WHERE UPPER(g.tenant) = UPPER($1)
           AND string_to_array(r.permissions, ',') && ARRAY[$2, $3]",
    )
    .bind(tenant)
    .bind(permissions::UPDATE_SYSTEM)
    .bind(permissions::ALL)
    .fetch_one(&mut *tx)
    .await?;
    if admins == 0 {
        return Err(AppError::BadRequest(format!(
            "this would leave {tenant} with no administrator: somebody must hold a role with \
             {} or {}, or nobody can manage roles again",
            permissions::UPDATE_SYSTEM,
            permissions::ALL
        )));
    }
    Ok(())
}

async fn create_grant(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Value>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let username = super::require_str(&body, "username")?;
    let role: i64 = body
        .get("role")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_i64())
        })
        .ok_or_else(|| AppError::BadRequest("'role' is required".to_string()))?;
    load(&state, &identity.tenant, role).await?;

    let id = state.incidents.next();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO hawier_grant (id, tenant, username, role) VALUES ($1, UPPER($2), $3, $4)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .bind(&username)
    .bind(role)
    .execute(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Conflict(format!("{username} already holds that role"))
        }
        _ => AppError::Db(e),
    })?;
    // The first grant is what switches authorisation on, so it is also the
    // one that can switch it on around somebody else.
    keep_an_administrator(&mut tx, &identity.tenant).await?;

    let row = sqlx::query(&format!("{GRANT_SELECT} WHERE g.id = $1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    state.authz_cache.invalidate();
    Ok((axum::http::StatusCode::CREATED, Json(grant_json(&row))))
}

async fn delete_grant(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<axum::http::StatusCode> {
    let mut tx = state.pool.begin().await?;
    let deleted =
        sqlx::query("DELETE FROM hawier_grant WHERE id = $2 AND UPPER(tenant) = UPPER($1)")
            .bind(&identity.tenant)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound(format!("grant {id} not found")));
    }
    keep_an_administrator(&mut tx, &identity.tenant).await?;
    tx.commit().await?;
    state.authz_cache.invalidate();
    Ok(axum::http::StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_permissions_are_kept_and_deduplicated() {
        let cleaned = validate_permissions(" READ_TARGET , READ_TARGET ,DELETE_TARGET").unwrap();
        assert_eq!(cleaned, "READ_TARGET,DELETE_TARGET");
    }

    #[test]
    fn an_unknown_permission_is_refused() {
        // A role granting something nothing checks would look powerful and do
        // nothing — the worst way for an access list to be wrong.
        assert!(validate_permissions("DELETE_EVERYTHING").is_err());
    }

    #[test]
    fn an_empty_role_is_refused() {
        assert!(validate_permissions("  ,  ").is_err());
    }
}
