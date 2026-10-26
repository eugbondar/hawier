//! `GET /{tenant}/controller/v1/{controllerId}` — the poll resource.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use hawier_api::codes::ActionStatus;
use hawier_api::ddi;
use sqlx::Row;

use super::controller_href;
use crate::auth::{authenticate_ddi, DdiActor};
use crate::db::model::{generate_sec_token, TargetRow, CONTROLLER_PRINCIPAL};
use crate::error::AppResult;
use crate::state::{now_millis, AppState};
use crate::tenant;

pub async fn controller_base(
    State(state): State<AppState>,
    Path((tenant, controller_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> AppResult<Json<ddi::DdiControllerBase>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;

    // Plug-and-play registration (gateway token / anonymous only).
    let target = match actor.target() {
        Some(t) => t.clone(),
        None => {
            debug_assert!(actor.may_register());
            register_target(&state, &tenant, &controller_id).await?
        }
    };

    // Lazy poll touch: writing last_target_query on EVERY poll would make
    // the whole fleet a constant write load; skip if touched recently.
    let address = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|ip| format!("http://{}", ip.trim()));
    let now = now_millis();
    let touch_before = now - state.config.poll_touch_interval_ms;
    sqlx::query(
        "UPDATE sp_target SET last_target_query = $1, address = COALESCE($2, address)
         WHERE id = $3 AND (last_target_query IS NULL OR last_target_query < $4)",
    )
    .bind(now)
    .bind(address)
    .bind(target.id)
    .bind(touch_before)
    .execute(&state.ddi_pool)
    .await?;

    let base = controller_href(&headers, &tenant, &controller_id);
    let mut links = ddi::Links::new();

    let confirmation_flow = tenant::cached_bool(
        &state,
        &tenant,
        tenant::KEY_CONFIRMATION_FLOW_ENABLED,
        false,
    )
    .await?;
    if confirmation_flow {
        links.insert(
            "confirmationBase".into(),
            ddi::Link::new(format!("{base}/confirmationBase")),
        );
    }

    // One round-trip for both "current action" and "installed action"
    // (LEFT JOIN LATERAL; the installed branch short-circuits on NULL).
    let snapshot = sqlx::query(
        "SELECT act.id AS action_id, act.status AS action_status,
                act.maintenance_cron_schedule AS mw_schedule,
                act.maintenance_duration AS mw_duration,
                act.maintenance_time_zone AS mw_time_zone,
                inst.id AS installed_action_id
         FROM (SELECT 1) one
         LEFT JOIN LATERAL (
             SELECT id, status, maintenance_cron_schedule, maintenance_duration,
                    maintenance_time_zone
             FROM sp_action
             WHERE target = $1 AND active = TRUE
             ORDER BY weight DESC, id ASC LIMIT 1
         ) act ON TRUE
         LEFT JOIN LATERAL (
             SELECT id FROM sp_action
             WHERE $2::BIGINT IS NOT NULL AND target = $1
               AND distribution_set = $2 AND status = $3
             ORDER BY id DESC LIMIT 1
         ) inst ON TRUE",
    )
    .bind(target.id)
    .bind(target.installed_distribution_set)
    .bind(ActionStatus::Finished.code())
    .fetch_one(&state.ddi_pool)
    .await?;

    let active: Option<(i64, i32)> = snapshot
        .get::<Option<i64>, _>("action_id")
        .map(|id| (id, snapshot.get::<i32, _>("action_status")));
    if let Some((action_id, action_status)) = active {
        match ActionStatus::try_from(action_status) {
            Ok(ActionStatus::Canceling) => {
                links.insert(
                    "cancelAction".into(),
                    ddi::Link::new(format!("{base}/cancelAction/{action_id}")),
                );
            }
            Ok(ActionStatus::WaitForConfirmation) => {
                // Only the confirmationBase link (already added above when
                // the flow is on) leads the device to the pending action.
            }
            _ => {
                links.insert(
                    "deploymentBase".into(),
                    ddi::Link::new(format!(
                        "{base}/deploymentBase/{action_id}?c={}",
                        etag_of(action_id, action_status)
                    )),
                );
            }
        }
    }

    if let Some(installed_action_id) = snapshot.get::<Option<i64>, _>("installed_action_id") {
        links.insert(
            "installedBase".into(),
            ddi::Link::new(format!("{base}/installedBase/{installed_action_id}")),
        );
    }

    if target.request_controller_attributes {
        links.insert(
            "configData".into(),
            ddi::Link::new(format!("{base}/configData")),
        );
    }

    let mut sleep = tenant::cached_value(&state, &tenant, tenant::KEY_POLLING_TIME, None)
        .await?
        .unwrap_or_else(|| "00:05:00".to_string());

    // If the current action waits for a maintenance window, shorten the
    // sleep so the device polls ~maintenanceWindowPollCount times before
    // the window opens (hawkBit's behaviour; floored at 30s).
    let window = crate::maintenance::action_window_state(
        &snapshot.get("mw_schedule"),
        &snapshot.get("mw_duration"),
        &snapshot.get("mw_time_zone"),
        now,
    );
    if let Some(window) = window {
        if !window.available {
            if let Some(next_start) = window.next_start {
                let poll_count = tenant::cached_value(
                    &state,
                    &tenant,
                    tenant::KEY_MAINTENANCE_WINDOW_POLL_COUNT,
                    None,
                )
                .await?
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(3);
                let default_ms = tenant::parse_polling_millis(&sleep).unwrap_or(300_000);
                let shortened = ((next_start - now) / poll_count).clamp(30_000, default_ms);
                if shortened < default_ms {
                    sleep = tenant::format_polling_millis(shortened);
                }
            }
        }
    }

    Ok(Json(ddi::DdiControllerBase {
        config: ddi::DdiConfig {
            polling: ddi::DdiPolling { sleep },
        },
        links,
    }))
}

fn etag_of(action_id: i64, status: i32) -> i64 {
    // Cache-buster the device echoes back; hawkBit hashes the action state.
    action_id ^ (status as i64) << 32
}

async fn register_target(
    state: &AppState,
    tenant_name: &str,
    controller_id: &str,
) -> AppResult<TargetRow> {
    tenant::ensure_tenant(&state.pool, tenant_name).await?;
    let now = now_millis();
    // Plug-and-play: unknown targets register as REGISTERED and are asked
    // for their controller attributes.
    sqlx::query(
        "INSERT INTO sp_target
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, controller_id, sec_token, request_controller_attributes, update_status)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $4, $5, TRUE, $6)
         ON CONFLICT DO NOTHING",
    )
    .bind(now)
    .bind(CONTROLLER_PRINCIPAL)
    .bind(tenant_name)
    .bind(controller_id)
    .bind(generate_sec_token())
    .bind(hawier_api::codes::TargetUpdateStatus::Registered.code())
    .execute(&state.ddi_pool)
    .await?;

    let row = sqlx::query_as::<_, TargetRow>(
        "SELECT id, tenant, controller_id, name, description, sec_token, address,
                last_target_query, install_date, request_controller_attributes,
                update_status, target_group, assigned_distribution_set,
                installed_distribution_set, target_type,
                created_at, created_by, last_modified_at, last_modified_by
         FROM sp_target WHERE controller_id = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(controller_id)
    .bind(tenant_name)
    .fetch_one(&state.ddi_pool)
    .await?;
    Ok(row)
}

impl DdiActor {
    /// The actor's target row or a 404 (target must exist for sub-resources).
    pub fn require_target(&self) -> AppResult<&TargetRow> {
        self.target()
            .ok_or_else(|| crate::error::AppError::NotFound("target not registered".to_string()))
    }
}
