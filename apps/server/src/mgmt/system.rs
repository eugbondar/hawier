//! `/rest/v1/system/configs` — tenant configuration key/value store.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use sqlx::Row;

use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::state::{now_millis, AppState};
use crate::tenant;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/system/configs", get(list).put(update_batch))
        .route(
            "/rest/v1/system/configs/{key}",
            get(get_one).put(update_one).delete(delete_one),
        )
}

/// Every key hawier understands, with its default (used when the tenant has
/// no explicit row — hawkBit answers `global: true` in that case).
const KNOWN_KEYS: &[&str] = &[
    tenant::KEY_POLLING_TIME,
    tenant::KEY_POLLING_OVERDUE_TIME,
    tenant::KEY_MAINTENANCE_WINDOW_POLL_COUNT,
    tenant::KEY_TARGETTOKEN_ENABLED,
    tenant::KEY_GATEWAYTOKEN_ENABLED,
    tenant::KEY_GATEWAYTOKEN_KEY,
    tenant::KEY_ROLLOUT_APPROVAL_ENABLED,
    tenant::KEY_ACTIONS_AUTOCLOSE_ENABLED,
    tenant::KEY_CONFIRMATION_FLOW_ENABLED,
    tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
    tenant::KEY_BATCH_ASSIGNMENTS_ENABLED,
    tenant::KEY_IMPLICIT_LOCK_ENABLED,
    tenant::KEY_TARGET_TYPE_REQUIRED,
];

/// Typed JSON value: booleans and integers are not stringified on the wire.
fn typed(raw: &str) -> Value {
    match raw {
        "true" => json!(true),
        "false" => json!(false),
        other => match other.parse::<i64>() {
            Ok(number) => json!(number),
            Err(_) => json!(other),
        },
    }
}

fn entry(value: &str, global: bool) -> Value {
    json!({ "value": typed(value), "global": global })
}

async fn stored_values(
    state: &AppState,
    tenant_name: &str,
) -> AppResult<std::collections::HashMap<String, String>> {
    let rows = sqlx::query(
        "SELECT conf_key, conf_value FROM sp_tenant_configuration WHERE UPPER(tenant) = UPPER($1)",
    )
    .bind(tenant_name)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("conf_key"),
                r.get::<String, _>("conf_value"),
            )
        })
        .collect())
}

async fn list(State(state): State<AppState>, identity: MgmtIdentity) -> AppResult<Json<Value>> {
    let stored = stored_values(&state, &identity.tenant).await?;
    let mut out = Map::new();
    for key in KNOWN_KEYS {
        match stored.get(*key) {
            Some(value) => {
                out.insert((*key).to_string(), entry(value, false));
            }
            None => {
                if let Some(default) = tenant::default_value(key) {
                    out.insert((*key).to_string(), entry(default, true));
                }
            }
        }
    }
    // Custom keys the tenant set beyond the known list.
    for (key, value) in &stored {
        out.entry(key.clone())
            .or_insert_with(|| entry(value, false));
    }
    Ok(Json(Value::Object(out)))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(key): Path<String>,
) -> AppResult<Json<Value>> {
    let stored: Option<String> = sqlx::query_scalar(
        "SELECT conf_value FROM sp_tenant_configuration
         WHERE UPPER(tenant) = UPPER($1) AND conf_key = $2",
    )
    .bind(&identity.tenant)
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?;

    match stored {
        Some(value) => Ok(Json(entry(&value, false))),
        None => match tenant::default_value(&key) {
            Some(default) => Ok(Json(entry(default, true))),
            None => Err(AppError::NotFound(format!(
                "configuration key '{key}' not found"
            ))),
        },
    }
}

/// Values arrive either bare (`{"value": x}`) or as a raw scalar.
fn extract_value(body: &Value) -> AppResult<String> {
    let raw = body.get("value").unwrap_or(body);
    Ok(match raw {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => {
            return Err(AppError::BadRequest(
                "configuration value must be scalar".into(),
            ))
        }
    })
}

async fn write_key(
    state: &AppState,
    identity: &MgmtIdentity,
    key: &str,
    value: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO sp_tenant_configuration
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, conf_key, conf_value)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5)
         ON CONFLICT (conf_key, tenant) DO UPDATE
             SET conf_value = EXCLUDED.conf_value,
                 last_modified_at = EXCLUDED.last_modified_at,
                 last_modified_by = EXCLUDED.last_modified_by,
                 optlock_revision = COALESCE(sp_tenant_configuration.optlock_revision, 0) + 1",
    )
    .bind(now_millis())
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(key)
    .bind(value)
    .execute(&state.pool)
    .await?;
    state.tenant_cache.invalidate(&identity.tenant);
    Ok(())
}

async fn update_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let value = extract_value(&body)?;
    write_key(&state, &identity, &key, &value).await?;
    Ok(Json(entry(&value, false)))
}

/// Batch update: `{"key": {"value": x}, "key2": y}`.
async fn update_batch(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let object = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("expected a JSON object of config entries".into()))?;
    let mut out = Map::new();
    for (key, raw) in object {
        let value = extract_value(raw)?;
        write_key(&state, &identity, key, &value).await?;
        out.insert(key.clone(), entry(&value, false));
    }
    Ok(Json(Value::Object(out)))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(key): Path<String>,
) -> AppResult<StatusCode> {
    sqlx::query(
        "DELETE FROM sp_tenant_configuration WHERE UPPER(tenant) = UPPER($1) AND conf_key = $2",
    )
    .bind(&identity.tenant)
    .bind(&key)
    .execute(&state.pool)
    .await?;
    state.tenant_cache.invalidate(&identity.tenant);
    Ok(StatusCode::NO_CONTENT)
}
