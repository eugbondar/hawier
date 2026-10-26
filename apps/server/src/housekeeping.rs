//! Control-node housekeeping loop: per-tenant action cleanup (hawkBit's
//! `AutoActionCleanup`) and expired `/ui/v1` session sweep.

use hawier_api::codes::ActionStatus;
use sqlx::Row;

use crate::error::AppResult;
use crate::state::{now_millis, AppState};
use crate::tenant;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(
            state.config.housekeeping_interval_ms.max(50),
        ));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = tick(&state).await {
                tracing::warn!(%error, "housekeeping tick failed");
            }
        }
    });
}

pub async fn tick(state: &AppState) -> AppResult<()> {
    sweep_sessions(state).await?;
    // Retention rules carry their own cron, so the tick only asks which are
    // due rather than owning a schedule of its own.
    if let Err(error) = crate::retention::tick(state).await {
        tracing::warn!(%error, "retention tick failed");
    }
    let tenants: Vec<String> = sqlx::query("SELECT tenant FROM sp_tenant")
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|r| r.get("tenant"))
        .collect();
    for tenant_name in tenants {
        cleanup_actions(state, &tenant_name).await?;
    }
    Ok(())
}

async fn sweep_sessions(state: &AppState) -> AppResult<()> {
    sqlx::query("DELETE FROM hawier_session WHERE expires_at < now()")
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// hawkBit `AutoActionCleanup`: delete actions in the configured statuses
/// not modified for `action.cleanup.auto.expiry` millis. Disabled unless
/// both keys are set (expiry >= 0 and a non-empty status list).
async fn cleanup_actions(state: &AppState, tenant_name: &str) -> AppResult<()> {
    let expiry = tenant::config_value(
        &state.pool,
        tenant_name,
        tenant::KEY_ACTION_CLEANUP_EXPIRY,
        None,
    )
    .await?
    .and_then(|v| v.trim().parse::<i64>().ok())
    .unwrap_or(-1);
    if expiry < 0 {
        return Ok(());
    }

    let statuses: Vec<i32> = tenant::config_value(
        &state.pool,
        tenant_name,
        tenant::KEY_ACTION_CLEANUP_STATUS,
        None,
    )
    .await?
    .map(|list| {
        list.split([';', ','])
            .filter_map(|name| ActionStatus::from_name(name.trim()))
            .map(|s| s.code())
            .collect()
    })
    .unwrap_or_default();
    if statuses.is_empty() {
        return Ok(());
    }

    let cutoff = now_millis() - expiry;
    let mut tx = state.pool.begin().await?;
    // No ON DELETE CASCADE in the hawkBit schema: children first.
    sqlx::query(
        "DELETE FROM sp_action_status_messages WHERE action_status IN (
             SELECT s.id FROM sp_action_status s
             JOIN sp_action a ON a.id = s.action
             WHERE UPPER(a.tenant) = UPPER($1) AND a.status = ANY($2)
               AND a.last_modified_at < $3)",
    )
    .bind(tenant_name)
    .bind(&statuses)
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM sp_action_status WHERE action IN (
             SELECT id FROM sp_action
             WHERE UPPER(tenant) = UPPER($1) AND status = ANY($2)
               AND last_modified_at < $3)",
    )
    .bind(tenant_name)
    .bind(&statuses)
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;
    let deleted = sqlx::query(
        "DELETE FROM sp_action
         WHERE UPPER(tenant) = UPPER($1) AND status = ANY($2) AND last_modified_at < $3",
    )
    .bind(tenant_name)
    .bind(&statuses)
    .bind(cutoff)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    if deleted > 0 {
        tracing::info!(tenant = tenant_name, deleted, "action cleanup");
    }
    Ok(())
}
