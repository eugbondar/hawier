//! confirmationBase resource group: confirmation flow + auto-confirmation.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use hawier_api::codes::ActionStatus;
use hawier_api::ddi;
use sqlx::Row;

use super::controller_href;
use crate::actions;
use crate::auth::authenticate_ddi;
use crate::db::model::CONTROLLER_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::{now_millis, AppState};

async fn auto_confirm_state(
    state: &AppState,
    target_id: i64,
) -> AppResult<ddi::DdiAutoConfirmationState> {
    let row = sqlx::query(
        "SELECT initiator, remark, created_at FROM sp_target_conf_status
         WHERE target = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(target_id)
    .fetch_optional(&state.ddi_pool)
    .await?;
    Ok(match row {
        Some(row) => ddi::DdiAutoConfirmationState {
            active: true,
            initiator: row.get("initiator"),
            remark: row.get("remark"),
            activated_at: row.get("created_at"),
        },
        None => ddi::DdiAutoConfirmationState {
            active: false,
            initiator: None,
            remark: None,
            activated_at: None,
        },
    })
}

pub async fn confirmation_base(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiConfirmationBase>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    let auto_confirm = auto_confirm_state(&state, target.id).await?;
    let base = controller_href(&headers, &tenant, &controller_id);
    let mut links = ddi::Links::new();
    if auto_confirm.active {
        links.insert(
            "deactivateAutoConfirm".into(),
            ddi::Link::new(format!("{base}/confirmationBase/deactivateAutoConfirm")),
        );
    } else {
        links.insert(
            "activateAutoConfirm".into(),
            ddi::Link::new(format!("{base}/confirmationBase/activateAutoConfirm")),
        );
    }

    // Link the pending wait-for-confirmation action, if any.
    if let Some(action) = super::next_active_or_none(&state, target.id).await? {
        if action.status == ActionStatus::WaitForConfirmation.code() {
            links.insert(
                "confirmationBase".into(),
                ddi::Link::new(format!("{base}/confirmationBase/{}", action.id)),
            );
        }
    }

    Ok(Json(ddi::DdiConfirmationBase {
        auto_confirm,
        links,
    }))
}

pub async fn confirmation_base_action(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    Query(query): Query<super::deployment::HistoryQuery>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiConfirmationBaseAction>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    let action = actions::load_action(&state.ddi_pool, &tenant, action_id)
        .await?
        .filter(|a| a.target == target.id && a.status == ActionStatus::WaitForConfirmation.code())
        .ok_or_else(|| AppError::NotFound(format!("confirmation action {action_id} not found")))?;

    let confirmation =
        super::build_deployment(&state, &headers, &tenant, &controller_id, &action).await?;
    let action_history =
        super::build_action_history(&state, &action, query.action_history.as_deref()).await?;

    Ok(Json(ddi::DdiConfirmationBaseAction {
        id: action.id.to_string(),
        confirmation,
        action_history,
    }))
}

pub async fn confirmation_feedback(
    State(state): State<AppState>,
    Path((tenant, controller_id, action_id)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Json(feedback): Json<ddi::DdiConfirmationFeedback>,
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
    if action.status != ActionStatus::WaitForConfirmation.code() {
        return Err(AppError::BadRequest(format!(
            "action {action_id} is not waiting for confirmation"
        )));
    }

    let mut messages = feedback.details.clone();
    match feedback.confirmation {
        ddi::Confirmation::Confirmed => {
            if messages.is_empty() {
                messages.push("Target confirmed the action.".to_string());
            }
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::Running,
                feedback.code,
                None,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
            actions::set_action_status(
                &mut tx,
                action.id,
                ActionStatus::Running,
                true,
                CONTROLLER_PRINCIPAL,
            )
            .await?;
        }
        ddi::Confirmation::Denied => {
            if messages.is_empty() {
                messages.push("Target denied the action.".to_string());
            }
            // Denied: the action stays gated (device may confirm later).
            actions::insert_status(
                &mut tx,
                &tenant,
                action.id,
                ActionStatus::WaitForConfirmation,
                feedback.code,
                None,
                CONTROLLER_PRINCIPAL,
                &messages,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(StatusCode::OK)
}

pub async fn activate_auto_confirm(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Option<Json<ddi::DdiActivateAutoConfirmation>>,
) -> AppResult<StatusCode> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?.clone();

    let payload = payload
        .map(|Json(p)| p)
        .unwrap_or(ddi::DdiActivateAutoConfirmation {
            initiator: None,
            remark: None,
        });

    let mut tx = state.ddi_pool.begin().await?;
    let already: bool =
        sqlx::query("SELECT EXISTS (SELECT 1 FROM sp_target_conf_status WHERE target = $1) AS e")
            .bind(target.id)
            .fetch_one(&mut *tx)
            .await?
            .get("e");
    if already {
        return Err(AppError::Conflict(
            "auto-confirmation already active".to_string(),
        ));
    }

    sqlx::query(
        "INSERT INTO sp_target_conf_status
         (target, initiator, remark, created_at, created_by,
          last_modified_at, last_modified_by, optlock_revision, tenant)
         VALUES ($1, $2, $3, $4, $5, $4, $5, 1, UPPER($6))",
    )
    .bind(target.id)
    .bind(payload.initiator.as_deref().unwrap_or(&controller_id))
    .bind(payload.remark.as_deref())
    .bind(now_millis())
    .bind(CONTROLLER_PRINCIPAL)
    .bind(&tenant)
    .execute(&mut *tx)
    .await?;

    // Auto-confirm any pending wait-for-confirmation actions right away.
    let pending = sqlx::query(
        "SELECT id FROM sp_action
         WHERE target = $1 AND active = TRUE AND status = $2
         FOR UPDATE",
    )
    .bind(target.id)
    .bind(ActionStatus::WaitForConfirmation.code())
    .fetch_all(&mut *tx)
    .await?;
    for row in &pending {
        let pending_id: i64 = row.get("id");
        actions::insert_status(
            &mut tx,
            &tenant,
            pending_id,
            ActionStatus::Running,
            None,
            None,
            CONTROLLER_PRINCIPAL,
            &["Action auto-confirmed by activated auto-confirmation.".to_string()],
        )
        .await?;
        actions::set_action_status(
            &mut tx,
            pending_id,
            ActionStatus::Running,
            true,
            CONTROLLER_PRINCIPAL,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(StatusCode::OK)
}

pub async fn deactivate_auto_confirm(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;

    sqlx::query("DELETE FROM sp_target_conf_status WHERE target = $1")
        .bind(target.id)
        .execute(&state.ddi_pool)
        .await?;
    Ok(StatusCode::OK)
}
