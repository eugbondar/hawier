//! cancelAction resource: GET the cancel order + feedback on it.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use hawier_api::codes::ActionStatus;
use hawier_api::ddi;

use crate::actions;
use crate::auth::authenticate_ddi;
use crate::db::model::CONTROLLER_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn cancel_action(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiCancel>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    let action = actions::load_action(&state.ddi_pool, &tenant, action_id)
        .await?
        .filter(|a| a.target == target.id && a.status == ActionStatus::Canceling.code())
        .ok_or_else(|| AppError::NotFound(format!("cancel action {action_id} not found")))?;

    Ok(Json(ddi::DdiCancel {
        id: action.id.to_string(),
        cancel_action: ddi::DdiCancelActionToStop {
            stop_id: action.id.to_string(),
        },
    }))
}

pub async fn cancel_feedback(
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
    if action.status != ActionStatus::Canceling.code() {
        return Err(AppError::BadRequest(format!(
            "action {action_id} is not being canceled"
        )));
    }

    let messages = feedback.status.details.clone();
    let code = feedback.status.code;
    let occurred_at = feedback.timestamp;

    use ddi::ExecutionStatus as Ex;
    match feedback.status.execution {
        Ex::Canceled | Ex::Closed => {
            // hawkBit treats closed+success on the cancel channel as a
            // completed cancellation as well.
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Canceled,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            actions::set_action_status(
                &mut tx,
                action.id,
                ActionStatus::Canceled,
                false,
                CONTROLLER_PRINCIPAL,
            )
            .await?;
            // If no other actions remain, the target settles: keep its
            // installed state authoritative.
            let pending = actions::has_other_active_actions(&mut tx, target.id, action.id).await?;
            if !pending {
                let settled = if target.installed_distribution_set.is_some() {
                    hawier_api::codes::TargetUpdateStatus::InSync
                } else {
                    hawier_api::codes::TargetUpdateStatus::Registered
                };
                actions::set_target_update_status(
                    &mut tx,
                    target.id,
                    settled,
                    CONTROLLER_PRINCIPAL,
                )
                .await?;
            }
        }
        Ex::Rejected => {
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::CancelRejected,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            // Device refused: the action resumes running.
            actions::set_action_status(
                &mut tx,
                action.id,
                ActionStatus::Running,
                true,
                CONTROLLER_PRINCIPAL,
            )
            .await?;
        }
        Ex::Proceeding | Ex::Resumed | Ex::Scheduled | Ex::Download | Ex::Downloaded => {
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Canceling,
                code,
                occurred_at,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(StatusCode::OK)
}
