//! Internal `/ui/v1` state API for hawier-ui (saved views, action logs,
//! sessions). Auth: `Authorization: Bearer ${HAWIER_INTERNAL_TOKEN}`.
//!
//! hawier-ui has zero direct DB access (project decision); these endpoints
//! are the only way UI state reaches the `hawier_*` tables. Served on both
//! node roles — pure request-scoped DB access, no singletons.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hawier_api::ui_state::{
    ActionLog, ActionLogFinish, ActionLogStart, SavedView, SavedViewCreate, SavedViewReorder,
    SavedViewUpdate, UiSession, UiSessionCreate,
};
use serde::Deserialize;
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::auth::InternalIdentity;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Session lifetime (matches the UI cookie: 8 hours).
const SESSION_TTL_SECONDS: i64 = 8 * 3600;
/// Keep at most this many log rows per tenant.
const LOG_RETENTION_PER_TENANT: i64 = 1000;
/// Keep at most this many recent searches per user.
const SEARCH_HISTORY_PER_USER: i64 = 20;

/// This build of the server: `vYY.MM.DD-<short sha>` (see build.rs).
pub const BUILD: &str = env!("HAWIER_BUILD");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ui/v1/version", get(version))
        .route("/ui/v1/surfaces", get(surfaces))
        .route("/ui/v1/views", get(list_views).post(create_view))
        .route("/ui/v1/views/reorder", put(reorder_views))
        .route("/ui/v1/views/{id}", put(update_view).delete(delete_view))
        // Reading the log is a mgmt resource (/rest/v1/actionlog); the
        // console only writes here, with the internal token.
        .route("/ui/v1/logs", post(start_log))
        .route("/ui/v1/logs/{id}", put(finish_log))
        .route(
            "/ui/v1/searches",
            get(list_searches)
                .post(record_search)
                .delete(clear_searches),
        )
        .route("/ui/v1/searches/{id}", axum::routing::delete(delete_search))
        .route("/ui/v1/sessions", post(create_session))
        .route(
            "/ui/v1/sessions/{id}",
            get(get_session).delete(delete_session),
        )
}

/// What this node is running, so the console can show its own build string
/// next to the server's — a mismatch after a deploy is then visible instead
/// of guessed from behaviour.
async fn version(
    State(state): State<AppState>,
    _auth: InternalIdentity,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "build": BUILD,
        "profile": crate::build_profile(),
        "node": state.config.node_id,
        "role": match state.config.role {
            crate::config::Role::Control => "control",
            crate::config::Role::Sub => "sub",
        },
    }))
}

// ---------------------------------------------------------------------------
// Saved views
// ---------------------------------------------------------------------------

fn view_from_row(row: &PgRow) -> SavedView {
    SavedView {
        id: row.get::<i64, _>("id").to_string(),
        tenant: row.get("tenant"),
        username: row.get("username"),
        resource: row.get("resource"),
        name: row.get("name"),
        q: row.get("q"),
        columns: row.get("columns"),
        sort: row.get("sort"),
        page_size: row.get::<i32, _>("page_size").max(1) as usize,
        position: row.get::<i32, _>("position").max(0) as usize,
        created_at: row.get::<i64, _>("created_ms"),
        updated_at: row.get::<i64, _>("updated_ms"),
    }
}

const VIEW_SELECT: &str = "SELECT id, tenant, username, resource, name, q, columns, sort,
       page_size, position,
       (EXTRACT(EPOCH FROM created_at) * 1000)::BIGINT AS created_ms,
       (EXTRACT(EPOCH FROM updated_at) * 1000)::BIGINT AS updated_ms
FROM hawier_saved_view";

#[derive(Deserialize)]
struct ViewScope {
    tenant: String,
    username: String,
    resource: Option<String>,
}

async fn list_views(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Query(scope): Query<ViewScope>,
) -> AppResult<Json<Vec<SavedView>>> {
    let rows = match &scope.resource {
        Some(resource) => {
            sqlx::query(&format!(
                "{VIEW_SELECT} WHERE tenant = $1 AND username = $2 AND resource = $3
                 ORDER BY position, name"
            ))
            .bind(&scope.tenant)
            .bind(&scope.username)
            .bind(resource)
            .fetch_all(&state.pool)
            .await?
        }
        None => {
            sqlx::query(&format!(
                "{VIEW_SELECT} WHERE tenant = $1 AND username = $2 ORDER BY resource, position"
            ))
            .bind(&scope.tenant)
            .bind(&scope.username)
            .fetch_all(&state.pool)
            .await?
        }
    };
    Ok(Json(rows.iter().map(view_from_row).collect()))
}

async fn create_view(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Json(body): Json<SavedViewCreate>,
) -> AppResult<(StatusCode, Json<SavedView>)> {
    let id = state.incidents.next();
    let position: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM hawier_saved_view
         WHERE tenant = $1 AND username = $2 AND resource = $3",
    )
    .bind(&body.tenant)
    .bind(&body.username)
    .bind(&body.resource)
    .fetch_one(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO hawier_saved_view
         (id, tenant, username, resource, name, q, columns, sort, page_size, position)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(&body.tenant)
    .bind(&body.username)
    .bind(&body.resource)
    .bind(&body.name)
    .bind(&body.q)
    .bind(&body.columns)
    .bind(&body.sort)
    .bind(body.page_size.max(1) as i32)
    .bind(position)
    .execute(&state.pool)
    .await?;

    let row = sqlx::query(&format!("{VIEW_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(view_from_row(&row))))
}

async fn update_view(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(id): Path<i64>,
    Json(body): Json<SavedViewUpdate>,
) -> AppResult<Json<SavedView>> {
    let result = sqlx::query(
        "UPDATE hawier_saved_view SET
             name = COALESCE($1, name),
             q = COALESCE($2, q),
             columns = COALESCE($3, columns),
             sort = COALESCE($4, sort),
             page_size = COALESCE($5, page_size),
             updated_at = now()
         WHERE id = $6",
    )
    .bind(&body.name)
    .bind(&body.q)
    .bind(&body.columns)
    .bind(&body.sort)
    .bind(body.page_size.map(|p| p.max(1) as i32))
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("view {id} not found")));
    }
    let row = sqlx::query(&format!("{VIEW_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(view_from_row(&row)))
}

async fn delete_view(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM hawier_saved_view WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("view {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder_views(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Json(body): Json<SavedViewReorder>,
) -> AppResult<StatusCode> {
    let mut tx = state.pool.begin().await?;
    for (position, raw_id) in body.ids.iter().enumerate() {
        let id: i64 = raw_id
            .parse()
            .map_err(|_| AppError::BadRequest(format!("invalid view id '{raw_id}'")))?;
        sqlx::query(
            "UPDATE hawier_saved_view SET position = $1, updated_at = now()
             WHERE id = $2 AND tenant = $3 AND username = $4 AND resource = $5",
        )
        .bind(position as i32)
        .bind(id)
        .bind(&body.tenant)
        .bind(&body.username)
        .bind(&body.resource)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Action logs
// ---------------------------------------------------------------------------

fn log_from_row(row: &PgRow) -> ActionLog {
    ActionLog {
        id: row.get::<i64, _>("id").to_string(),
        tenant: row.get("tenant"),
        username: row.get("username"),
        resource: row.get("resource"),
        resource_id: row.get("resource_id"),
        action: row.get("action"),
        status: row.get("status"),
        request_summary: row.get("request_summary"),
        response_summary: row.get("response_summary"),
        error: row.get("error"),
        created_at: row.get::<i64, _>("created_ms"),
        finished_at: row.get::<Option<i64>, _>("finished_ms"),
    }
}

const LOG_SELECT: &str = "SELECT id, tenant, username, resource, resource_id, action, status,
       request_summary, response_summary, error,
       (EXTRACT(EPOCH FROM created_at) * 1000)::BIGINT AS created_ms,
       (EXTRACT(EPOCH FROM finished_at) * 1000)::BIGINT AS finished_ms
FROM hawier_action_log";

async fn start_log(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Json(body): Json<ActionLogStart>,
) -> AppResult<(StatusCode, Json<ActionLog>)> {
    let id = state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_action_log
         (id, tenant, username, resource, resource_id, action, status, request_summary)
         VALUES ($1, $2, $3, $4, $5, $6, 'running', $7)",
    )
    .bind(id)
    .bind(&body.tenant)
    .bind(&body.username)
    .bind(&body.resource)
    .bind(&body.resource_id)
    .bind(&body.action)
    .bind(&body.request_summary)
    .execute(&state.pool)
    .await?;

    // Retention trim: drop the oldest rows beyond the cap.
    sqlx::query(
        "DELETE FROM hawier_action_log WHERE id IN (
             SELECT id FROM hawier_action_log WHERE tenant = $1
             ORDER BY created_at DESC OFFSET $2)",
    )
    .bind(&body.tenant)
    .bind(LOG_RETENTION_PER_TENANT)
    .execute(&state.pool)
    .await?;

    let row = sqlx::query(&format!("{LOG_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(log_from_row(&row))))
}

async fn finish_log(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(id): Path<i64>,
    Json(body): Json<ActionLogFinish>,
) -> AppResult<Json<ActionLog>> {
    let result = sqlx::query(
        "UPDATE hawier_action_log SET
             status = $1, response_summary = $2, error = $3, finished_at = now()
         WHERE id = $4",
    )
    .bind(&body.status)
    .bind(&body.response_summary)
    .bind(&body.error)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("log {id} not found")));
    }
    let row = sqlx::query(&format!("{LOG_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(log_from_row(&row)))
}

// ---------------------------------------------------------------------------
// Server error journal (written by error_capture middleware)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Search history
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SearchScope {
    tenant: String,
    username: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct SearchRecord {
    tenant: String,
    username: String,
    q: String,
}

async fn list_searches(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Query(scope): Query<SearchScope>,
) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT id, q, uses FROM hawier_search_query
         WHERE tenant = $1 AND username = $2
         ORDER BY last_used_at DESC LIMIT $3",
    )
    .bind(&scope.tenant)
    .bind(&scope.username)
    .bind(scope.limit.unwrap_or(10).clamp(1, SEARCH_HISTORY_PER_USER))
    .fetch_all(&state.pool)
    .await?;
    let content: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<i64, _>("id").to_string(),
                "q": r.get::<String, _>("q"),
                "uses": r.get::<i32, _>("uses"),
            })
        })
        .collect();
    Ok(Json(serde_json::json!(content)))
}

async fn record_search(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Json(body): Json<SearchRecord>,
) -> AppResult<StatusCode> {
    let q = body.q.trim();
    if q.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Repeating a search bumps the existing row instead of duplicating it.
    sqlx::query(
        "INSERT INTO hawier_search_query (id, tenant, username, q)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant, username, q)
         DO UPDATE SET uses = hawier_search_query.uses + 1, last_used_at = now()",
    )
    .bind(state.incidents.next())
    .bind(&body.tenant)
    .bind(&body.username)
    .bind(q)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "DELETE FROM hawier_search_query WHERE id IN (
             SELECT id FROM hawier_search_query
             WHERE tenant = $1 AND username = $2
             ORDER BY last_used_at DESC OFFSET $3)",
    )
    .bind(&body.tenant)
    .bind(&body.username)
    .bind(SEARCH_HISTORY_PER_USER)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_search(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM hawier_search_query WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_searches(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Query(scope): Query<SearchScope>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM hawier_search_query WHERE tenant = $1 AND username = $2")
        .bind(&scope.tenant)
        .bind(&scope.username)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

fn session_from_row(row: &PgRow) -> UiSession {
    UiSession {
        id: row.get::<i64, _>("id").to_string(),
        token: row.get("token"),
        username: row.get("username"),
        tenant: row.get("tenant"),
        password: row.get("secret"),
        expires_at: row.get::<i64, _>("expires_ms"),
    }
}

const SESSION_SELECT: &str = "SELECT id, token, username, tenant, secret,
       (EXTRACT(EPOCH FROM expires_at) * 1000)::BIGINT AS expires_ms
FROM hawier_session";

async fn create_session(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Json(body): Json<UiSessionCreate>,
) -> AppResult<(StatusCode, Json<UiSession>)> {
    let id = state.incidents.next();
    // The cookie carries the token, not the id: whoever holds it can read
    // the secret below, so it has to be unguessable — a snowflake is not.
    let token = crate::db::model::generate_sec_token();
    // NARROW: stores the user's mgmt password so the UI can replay Basic
    // auth (SCHEMA_NOTES.md #18); pre-production hawier moves to
    // server-issued tokens and drops the secret column.
    sqlx::query(
        "INSERT INTO hawier_session (id, token, username, tenant, secret, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
    )
    .bind(id)
    .bind(&token)
    .bind(&body.username)
    .bind(&body.tenant)
    .bind(&body.password)
    .bind(SESSION_TTL_SECONDS as f64)
    .execute(&state.pool)
    .await?;

    // Opportunistic sweep of expired sessions.
    sqlx::query("DELETE FROM hawier_session WHERE expires_at < now()")
        .execute(&state.pool)
        .await
        .ok();

    let row = sqlx::query(&format!("{SESSION_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(session_from_row(&row))))
}

async fn get_session(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(token): Path<String>,
) -> AppResult<Json<UiSession>> {
    let row = sqlx::query(&format!(
        "{SESSION_SELECT} WHERE token = $1 AND expires_at >= now()"
    ))
    .bind(&token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("session not found or expired".to_string()))?;
    Ok(Json(session_from_row(&row)))
}

async fn delete_session(
    State(state): State<AppState>,
    _auth: InternalIdentity,
    Path(token): Path<String>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM hawier_session WHERE token = $1")
        .bind(&token)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What extensions put in the console tree.
///
/// The console is a separate process with no extension host of its own, so
/// it asks the node — the same way it gets every other piece of state.
/// Nothing configured means an empty declaration, not an error: a console
/// with only its built-in tree is a perfectly good console.
async fn surfaces(
    State(state): State<AppState>,
    _identity: InternalIdentity,
) -> AppResult<Json<hawier_api::ext::ConsoleDeclaration>> {
    let Some(extension) = state.ext.console.as_deref() else {
        return Ok(Json(hawier_api::ext::ConsoleDeclaration::default()));
    };
    let declaration = crate::ext::call(
        extension,
        hawier_api::ext::points::CONSOLE_SURFACES,
        &serde_json::json!({}),
    )
    .await?;
    Ok(Json(declaration))
}
