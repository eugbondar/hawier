//! deploymentBase (GET + feedback) and installedBase (GET + offline PUT).

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use hawier_api::codes::{ActionStatus, ActionType, TargetUpdateStatus};
use hawier_api::ddi;
use serde::Deserialize;
use sqlx::Row;

use crate::actions;
use crate::auth::authenticate_ddi;
use crate::db::model::CONTROLLER_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::{now_millis, AppState};

#[derive(Deserialize)]
pub struct HistoryQuery {
    #[serde(rename = "actionHistory")]
    pub action_history: Option<String>,
    /// Cache-buster etag echoed by devices; accepted and ignored.
    #[allow(dead_code)]
    pub c: Option<String>,
}

pub async fn deployment_base(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiDeploymentBase>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    let action = actions::load_action(&state.ddi_pool, &tenant, action_id)
        .await?
        .filter(|a| a.target == target.id)
        .ok_or_else(|| AppError::NotFound(format!("action {action_id} not found")))?;

    // A canceling action is served via cancelAction, not deploymentBase.
    if action.status == ActionStatus::Canceling.code() {
        return Err(AppError::NotFound(format!(
            "action {action_id} is canceling"
        )));
    }

    // Record RETRIEVED once per action.
    if action.is_active() {
        let mut tx = state.ddi_pool.begin().await?;
        // Lock the action row first: two devices sharing a controller id (a
        // cloned image, a restarted process racing itself) both fetch this
        // endpoint at once, and an unlocked EXISTS let both conclude the
        // marker was missing and write one. Duplicate history is not
        // cosmetic — it feeds the 800-row action-status quota.
        //
        // This is deploymentBase, not the poll path: the lock is taken only
        // when the device actually has an action to fetch.
        sqlx::query("SELECT id FROM sp_action WHERE id = $1 FOR UPDATE")
            .bind(action.id)
            .fetch_optional(&mut *tx)
            .await?;
        let already: bool = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM sp_action_status
             WHERE action = $1 AND status = $2) AS e",
        )
        .bind(action.id)
        .bind(ActionStatus::Retrieved.code())
        .fetch_one(&mut *tx)
        .await?
        .get("e");
        if !already {
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Retrieved,
                None,
                None,
                CONTROLLER_PRINCIPAL,
                &["Target retrieved update action and should start now the download.".to_string()],
            )
            .await?;
        }
        tx.commit().await?;
    }

    let deployment =
        super::build_deployment(&state, &headers, &tenant, &controller_id, &action).await?;
    let action_history =
        super::build_action_history(&state, &action, query.action_history.as_deref()).await?;

    Ok(Json(ddi::DdiDeploymentBase {
        id: action.id.to_string(),
        deployment,
        action_history,
    }))
}

pub async fn deployment_feedback(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(feedback): Json<ddi::DdiActionFeedback>,
) -> AppResult<StatusCode> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?.clone();

    let mut tx = state.ddi_pool.begin().await?;
    let action = actions::lock_action(&mut tx, &tenant, action_id)
        .await?
        .filter(|a| a.target == target.id)
        .ok_or_else(|| AppError::NotFound(format!("action {action_id} not found")))?;

    if !action.is_active() {
        return Err(AppError::Gone(format!(
            "action {action_id} is already closed"
        )));
    }

    let mut messages: Vec<String> = feedback.status.details.clone();
    let code = feedback.status.code;
    let occurred_at = feedback.timestamp;
    let execution = feedback.status.execution;
    let finished = feedback.status.result.finished;
    let action_type = ActionType::try_from(action.action_type).ok();

    use ddi::ExecutionStatus as Ex;
    use ddi::FinalResult as Fr;
    match execution {
        Ex::Proceeding | Ex::Resumed | Ex::Scheduled => {
            let status = ActionStatus::Running;
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                status,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            actions::set_action_status(&mut tx, action.id, status, true, CONTROLLER_PRINCIPAL)
                .await?;
        }
        Ex::Download => {
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Download,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            actions::set_action_status(
                &mut tx,
                action.id,
                ActionStatus::Download,
                true,
                CONTROLLER_PRINCIPAL,
            )
            .await?;
        }
        Ex::Downloaded => {
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Downloaded,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            let download_only = action_type == Some(ActionType::DownloadOnly);
            // DOWNLOAD_ONLY actions close at DOWNLOADED.
            actions::set_action_status(
                &mut tx,
                action.id,
                ActionStatus::Downloaded,
                !download_only,
                CONTROLLER_PRINCIPAL,
            )
            .await?;
            if download_only {
                finish_target_state(&mut tx, &tenant, &target, &action, true).await?;
            }
        }
        Ex::Closed => {
            let (status, success) = match finished {
                Fr::Success => (ActionStatus::Finished, true),
                Fr::Failure | Fr::None => (ActionStatus::Error, false),
            };
            if messages.is_empty() {
                messages.push(format!(
                    "Target reported update {}.",
                    if success { "successful" } else { "failed" }
                ));
            }
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                status,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            actions::set_action_status(&mut tx, action.id, status, false, CONTROLLER_PRINCIPAL)
                .await?;
            finish_target_state(&mut tx, &tenant, &target, &action, success).await?;
        }
        Ex::Canceled | Ex::Rejected => {
            // Cancel outcomes belong to the cancelAction feedback channel.
            return Err(AppError::BadRequest(format!(
                "execution status '{execution:?}' is not valid for deploymentBase feedback"
            )));
        }
    }

    tx.commit().await?;
    Ok(StatusCode::OK)
}

/// Update target rows after an action closed (or a DOWNLOAD_ONLY completed).
async fn finish_target_state(
    tx: &mut sqlx::PgConnection,
    _tenant: &str,
    target: &crate::db::model::TargetRow,
    action: &actions::ActionRow,
    success: bool,
) -> AppResult<()> {
    if success {
        // As hawkBit does: what a device runs after an update is the device's
        // to say, so a finished (or download-only) action asks for its
        // attributes again on the next poll.
        sqlx::query(
            "UPDATE sp_target
             SET installed_distribution_set = $1, install_date = $2,
                 request_controller_attributes = TRUE,
                 last_modified_at = $2, last_modified_by = $3,
                 optlock_revision = COALESCE(optlock_revision, 0) + 1
             WHERE id = $4",
        )
        .bind(action.distribution_set)
        .bind(now_millis())
        .bind(CONTROLLER_PRINCIPAL)
        .bind(target.id)
        .execute(&mut *tx)
        .await?;
        let pending = actions::has_other_active_actions(tx, target.id, action.id).await?;
        let status = if pending {
            TargetUpdateStatus::Pending
        } else {
            TargetUpdateStatus::InSync
        };
        actions::set_target_update_status(tx, target.id, status, CONTROLLER_PRINCIPAL).await?;
    } else {
        actions::set_target_update_status(
            tx,
            target.id,
            TargetUpdateStatus::Error,
            CONTROLLER_PRINCIPAL,
        )
        .await?;
    }
    Ok(())
}

pub async fn installed_base(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiDeploymentBase>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    let action = actions::load_action(&state.ddi_pool, &tenant, action_id)
        .await?
        .filter(|a| a.target == target.id && a.status == ActionStatus::Finished.code())
        .ok_or_else(|| AppError::NotFound(format!("installed action {action_id} not found")))?;

    let deployment =
        super::build_deployment(&state, &headers, &tenant, &controller_id, &action).await?;
    let action_history =
        super::build_action_history(&state, &action, query.action_history.as_deref()).await?;

    Ok(Json(ddi::DdiDeploymentBase {
        id: action.id.to_string(),
        deployment,
        action_history,
    }))
}

/// Offline report: the device tells us which DS name/version it already runs.
pub async fn put_installed_base(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(version): Json<ddi::DdiAssignedVersion>,
) -> AppResult<StatusCode> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?.clone();

    let ds = sqlx::query(
        "SELECT id FROM sp_distribution_set
         WHERE UPPER(tenant) = UPPER($1) AND name = $2 AND version = $3
           AND (deleted IS NOT TRUE)",
    )
    .bind(&tenant)
    .bind(&version.name)
    .bind(&version.version)
    .fetch_optional(&state.ddi_pool)
    .await?
    .ok_or_else(|| {
        AppError::NotFound(format!(
            "distribution set {}:{} not found",
            version.name, version.version
        ))
    })?;
    let ds_id: i64 = ds.get("id");

    if target.installed_distribution_set == Some(ds_id) {
        return Ok(StatusCode::OK);
    }

    let now = now_millis();
    let mut tx = state.ddi_pool.begin().await?;
    // Offline-installed: record a closed FINISHED action for traceability.
    let action_id: i64 = sqlx::query(
        "INSERT INTO sp_action
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, active, status, distribution_set, target, action_type, weight, initiated_by)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), FALSE, $4, $5, $6, $7, $8, $2)
         RETURNING id",
    )
    .bind(now)
    .bind(CONTROLLER_PRINCIPAL)
    .bind(&tenant)
    .bind(ActionStatus::Finished.code())
    .bind(ds_id)
    .bind(target.id)
    .bind(ActionType::Forced.code())
    .bind(crate::deploy_logic::DEFAULT_WEIGHT)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    actions::insert_status(
        &mut tx,
        &tenant,
        action_id,
        ActionStatus::Finished,
        None,
        None,
        CONTROLLER_PRINCIPAL,
        &["Target reported offline-installed distribution set.".to_string()],
    )
    .await?;

    sqlx::query(
        "UPDATE sp_target
         SET installed_distribution_set = $1, assigned_distribution_set = COALESCE(assigned_distribution_set, $1),
             install_date = $2, update_status = $3,
             last_modified_at = $2, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $5",
    )
    .bind(ds_id)
    .bind(now)
    .bind(TargetUpdateStatus::InSync.code())
    .bind(CONTROLLER_PRINCIPAL)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(StatusCode::OK)
}
