//! `PUT configData` — controller attributes (merge | replace | remove).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use hawier_api::ddi::{DdiConfigData, UpdateMode};

use crate::auth::authenticate_ddi;
use crate::db::model::CONTROLLER_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::{now_millis, AppState};

/// hawkBit's quota for controller attributes per target.
const MAX_ATTRIBUTES: usize = 100;

pub async fn put_config_data(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(config_data): Json<DdiConfigData>,
) -> AppResult<StatusCode> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    for (key, value) in &config_data.data {
        if key.len() > 128 || value.len() > 128 {
            return Err(AppError::BadRequest(format!(
                "attribute '{key}' exceeds the 128 character limit"
            )));
        }
    }

    let mode = config_data.mode.unwrap_or(UpdateMode::Merge);
    let mut tx = state.ddi_pool.begin().await?;

    match mode {
        UpdateMode::Replace => {
            sqlx::query("DELETE FROM sp_target_attributes WHERE target = $1")
                .bind(target.id)
                .execute(&mut *tx)
                .await?;
            for (key, value) in &config_data.data {
                sqlx::query(
                    "INSERT INTO sp_target_attributes (target, attribute_key, attribute_value)
                     VALUES ($1, $2, $3)",
                )
                .bind(target.id)
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
            }
        }
        UpdateMode::Merge => {
            for (key, value) in &config_data.data {
                sqlx::query(
                    "INSERT INTO sp_target_attributes (target, attribute_key, attribute_value)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (target, attribute_key)
                     DO UPDATE SET attribute_value = EXCLUDED.attribute_value",
                )
                .bind(target.id)
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
            }
        }
        UpdateMode::Remove => {
            for key in config_data.data.keys() {
                sqlx::query(
                    "DELETE FROM sp_target_attributes WHERE target = $1 AND attribute_key = $2",
                )
                .bind(target.id)
                .bind(key)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sp_target_attributes WHERE target = $1")
            .bind(target.id)
            .fetch_one(&mut *tx)
            .await?;
    if count as usize > MAX_ATTRIBUTES {
        return Err(AppError::Quota(format!(
            "quota of {MAX_ATTRIBUTES} controller attributes exceeded"
        )));
    }

    // Attributes received: stop asking for them.
    sqlx::query(
        "UPDATE sp_target
         SET request_controller_attributes = FALSE,
             last_modified_at = $1, last_modified_by = $2,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $3",
    )
    .bind(now_millis())
    .bind(CONTROLLER_PRINCIPAL)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(StatusCode::OK)
}
