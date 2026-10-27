//! Rollout engine — control node only.
//!
//! A 2s tick drives rollouts through their lifecycle:
//! CREATING → (approval?) → READY → STARTING → RUNNING → FINISHED,
//! with PAUSED/STOPPING/DELETING side exits. Rollout rows are claimed with
//! `FOR UPDATE SKIP LOCKED`, so several control nodes would not double-run
//! a rollout (single control node is the supported topology for now).
//!
//! The DDI serving path never depends on this module: if the engine is down
//! devices keep polling/downloading/reporting; only *progression* pauses.

use hawier_api::codes::{ActionStatus, ActionType, RolloutGroupStatus, RolloutStatus};
use sqlx::{PgConnection, PgPool, Row};
use tracing::{info, warn};

use crate::actions;
use crate::db::model::SYSTEM_PRINCIPAL;
use crate::error::AppResult;
use crate::incident::IncidentGen;
use crate::rsql;
use crate::state::{now_millis, AppState};

const TICK: std::time::Duration = std::time::Duration::from_secs(2);

/// Spawn the engine loop (call on the control node only).
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = tick(&state).await {
                warn!(%error, "rollout engine tick failed");
            }
            if let Err(error) = auto_assign_tick(&state).await {
                warn!(%error, "auto-assign tick failed");
            }
        }
    });
}

/// One engine pass over all transitional rollouts.
pub async fn tick(state: &AppState) -> AppResult<()> {
    let pool = &state.pool;
    let transitional = sqlx::query(
        "SELECT id, tenant, status FROM sp_rollout
         WHERE status IN ($1, $2, $3, $4, $5) AND deleted IS NOT TRUE",
    )
    .bind(RolloutStatus::Creating.code())
    .bind(RolloutStatus::Starting.code())
    .bind(RolloutStatus::Running.code())
    .bind(RolloutStatus::Stopping.code())
    .bind(RolloutStatus::Deleting.code())
    .fetch_all(pool)
    .await?;

    for row in &transitional {
        let rollout_id: i64 = row.get("id");
        let tenant: String = row.get("tenant");
        let status = RolloutStatus::try_from(row.get::<i32, _>("status"))
            .map_err(|e| crate::error::AppError::Internal(e.into()))?;

        // CREATING and RUNNING ask a strategy, and an extension must never be
        // called inside the claim transaction — so those two take a snapshot
        // first and claim afterwards. The rest keep the plain claim path.
        let outcome = match status {
            RolloutStatus::Creating => materialize_rollout(state, &tenant, rollout_id).await,
            RolloutStatus::Running => evaluate_rollout(state, &tenant, rollout_id).await,
            _ => process_rollout(pool, &state.incidents, &tenant, rollout_id).await,
        };
        if let Err(error) = outcome {
            warn!(rollout_id, %error, "rollout processing failed");
        }
    }
    Ok(())
}

async fn process_rollout(
    pool: &PgPool,
    ids: &IncidentGen,
    tenant: &str,
    rollout_id: i64,
) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    // Claim the rollout; skip if another node holds it.
    let Some(row) = sqlx::query(
        "SELECT id, status, distribution_set, target_filter, action_type, weight,
                forced_time, start_at, is_dynamic, name
         FROM sp_rollout WHERE id = $1 FOR UPDATE SKIP LOCKED",
    )
    .bind(rollout_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };

    let status = RolloutStatus::try_from(row.get::<i32, _>("status"))
        .map_err(|e| crate::error::AppError::Internal(e.into()))?;
    match status {
        RolloutStatus::Starting => start_rollout(&mut tx, ids, tenant, &row).await?,
        RolloutStatus::Stopping => stop_rollout(&mut tx, tenant, &row).await?,
        RolloutStatus::Deleting => delete_rollout(&mut tx, tenant, &row).await?,
        _ => {}
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CREATING → READY / WAITING_FOR_APPROVAL: distribute targets into groups
// ---------------------------------------------------------------------------

/// Targets matching `filter` that may receive `ds_id` — an incompatible
/// target is not a candidate at all, so nothing downstream has to skip it.
async fn matching_target_ids(
    tx: &mut PgConnection,
    tenant: &str,
    filter: &str,
    ds_id: i64,
) -> AppResult<Vec<i64>> {
    let clause = rsql::to_sql(filter, crate::mgmt::targets::target_fields(), 1)?;
    let typed_only = crate::deploy_logic::typed_only(&mut *tx, tenant).await?;
    let sql = format!(
        "SELECT t.id FROM sp_target t
         WHERE UPPER(t.tenant) = UPPER($1) AND {} AND {} ORDER BY t.id",
        clause.sql,
        crate::deploy_logic::compatible_with_set_sql(ds_id, typed_only)
    );
    let rows = rsql::bind_values(sqlx::query(&sql).bind(tenant), &clause.binds)
        .fetch_all(tx)
        .await?;
    Ok(rows.iter().map(|r| r.get("id")).collect())
}

async fn start_rollout(
    tx: &mut PgConnection,
    ids: &IncidentGen,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<()> {
    let rollout_id: i64 = rollout.get("id");
    if let Some(start_at) = rollout.get::<Option<i64>, _>("start_at") {
        if start_at > now_millis() {
            return Ok(());
        }
    }

    let groups: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_rollout_group WHERE rollout = $1 AND status = $2 ORDER BY id",
    )
    .bind(rollout_id)
    .bind(RolloutGroupStatus::Ready.code())
    .fetch_all(&mut *tx)
    .await?;

    // Create SCHEDULED actions for every group member (inactive until the
    // group starts, so DDI ignores them).
    for group_id in &groups {
        create_scheduled_actions(tx, tenant, rollout, *group_id).await?;
        sqlx::query("UPDATE sp_rollout_group SET status = $1 WHERE id = $2")
            .bind(RolloutGroupStatus::Scheduled.code())
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
    }

    if let Some(first) = groups.first() {
        activate_group(tx, tenant, *first).await?;
    }

    sqlx::query(
        "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3
         WHERE id = $4",
    )
    .bind(RolloutStatus::Running.code())
    .bind(now_millis())
    .bind(SYSTEM_PRINCIPAL)
    .bind(rollout_id)
    .execute(&mut *tx)
    .await?;
    crate::events::emit(
        tx,
        ids,
        tenant,
        hawier_api::ext::kinds::ROLLOUT_STARTED,
        serde_json::json!({
            "rolloutId": rollout_id,
            "name": rollout.get::<String, _>("name"),
            "groups": groups.len(),
        }),
    )
    .await?;
    info!(rollout_id, groups = groups.len(), "rollout started");
    Ok(())
}

async fn create_scheduled_actions(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
    group_id: i64,
) -> AppResult<()> {
    let rollout_id: i64 = rollout.get("id");
    let ds_id: i64 = rollout.get("distribution_set");
    let action_type: i32 = rollout.get("action_type");
    let weight: i32 = rollout.get("weight");
    let forced_time: Option<i64> = rollout.get("forced_time");
    let now = now_millis();

    let targets: Vec<i64> = sqlx::query_scalar(
        "SELECT target FROM sp_rollout_target_group WHERE rollout_group = $1
         AND NOT EXISTS (SELECT 1 FROM sp_action a
             WHERE a.target = sp_rollout_target_group.target AND a.rollout = $2)",
    )
    .bind(group_id)
    .bind(rollout_id)
    .fetch_all(&mut *tx)
    .await?;

    for target_id in &targets {
        let action_id: i64 = sqlx::query_scalar(
            "INSERT INTO sp_action
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, active, forced_time, status, distribution_set, target,
              rollout, rollout_group, action_type, weight, initiated_by)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), FALSE, $4, $5, $6, $7, $8, $9, $10, $11, $2)
             RETURNING id",
        )
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(tenant)
        .bind(forced_time)
        .bind(ActionStatus::Scheduled.code())
        .bind(ds_id)
        .bind(target_id)
        .bind(rollout_id)
        .bind(group_id)
        .bind(action_type)
        .bind(weight)
        .fetch_one(&mut *tx)
        .await?;
        actions::insert_status(
            tx,
            tenant,
            action_id,
            ActionStatus::Scheduled,
            None,
            None,
            SYSTEM_PRINCIPAL,
            &["Update Server: scheduled by rollout".to_string()],
        )
        .await?;
    }
    Ok(())
}

/// Activate a group's scheduled actions: supersede other active actions of
/// each target, honour the confirmation gate, mark the group RUNNING.
async fn activate_group(tx: &mut PgConnection, tenant: &str, group_id: i64) -> AppResult<()> {
    let confirmation_flow = crate::tenant::config_bool(
        &mut *tx,
        tenant,
        crate::tenant::KEY_CONFIRMATION_FLOW_ENABLED,
        false,
    )
    .await?;
    let group_confirmation: Option<bool> =
        sqlx::query_scalar("SELECT confirmation_required FROM sp_rollout_group WHERE id = $1")
            .bind(group_id)
            .fetch_one(&mut *tx)
            .await?;
    let multi_assignments = crate::tenant::config_bool(
        &mut *tx,
        tenant,
        crate::tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        false,
    )
    .await?;
    let autoclose = crate::tenant::config_bool(
        &mut *tx,
        tenant,
        crate::tenant::KEY_ACTIONS_AUTOCLOSE_ENABLED,
        false,
    )
    .await?;
    let (supersede_status, supersede_active) = if autoclose {
        (ActionStatus::Canceled, false)
    } else {
        (ActionStatus::Canceling, true)
    };

    let scheduled = sqlx::query(
        "SELECT a.id, a.target, a.distribution_set FROM sp_action a
         WHERE a.rollout_group = $1 AND a.status = $2 FOR UPDATE",
    )
    .bind(group_id)
    .bind(ActionStatus::Scheduled.code())
    .fetch_all(&mut *tx)
    .await?;

    for action in &scheduled {
        let action_id: i64 = action.get("id");
        let target_id: i64 = action.get("target");
        let ds_id: i64 = action.get("distribution_set");

        // Supersede other active actions of this target (unless the tenant
        // runs multiple assignments in parallel).
        if !multi_assignments {
            let others = sqlx::query(
                "SELECT id FROM sp_action
                 WHERE target = $1 AND active = TRUE AND id <> $2 AND status <> $3",
            )
            .bind(target_id)
            .bind(action_id)
            .bind(ActionStatus::Canceling.code())
            .fetch_all(&mut *tx)
            .await?;
            for other in &others {
                let other_id: i64 = other.get("id");
                actions::set_action_status(
                    tx,
                    other_id,
                    supersede_status,
                    supersede_active,
                    SYSTEM_PRINCIPAL,
                )
                .await?;
                actions::insert_status(
                    tx,
                    tenant,
                    other_id,
                    supersede_status,
                    None,
                    None,
                    SYSTEM_PRINCIPAL,
                    &["Update Server: cancel obsolete action due to rollout".to_string()],
                )
                .await?;
            }
        }

        let auto_confirmed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sp_target_conf_status WHERE target = $1)",
        )
        .bind(target_id)
        .fetch_one(&mut *tx)
        .await?;
        let gate = confirmation_flow && group_confirmation.unwrap_or(true) && !auto_confirmed;
        let status = if gate {
            ActionStatus::WaitForConfirmation
        } else {
            ActionStatus::Running
        };

        actions::set_action_status(tx, action_id, status, true, SYSTEM_PRINCIPAL).await?;
        actions::insert_status(
            tx,
            tenant,
            action_id,
            status,
            None,
            None,
            SYSTEM_PRINCIPAL,
            &["Update Server: rollout group started".to_string()],
        )
        .await?;

        sqlx::query(
            "UPDATE sp_target SET assigned_distribution_set = $1, update_status = $2,
                 last_modified_at = $3, last_modified_by = $4,
                 optlock_revision = COALESCE(optlock_revision, 0) + 1
             WHERE id = $5",
        )
        .bind(ds_id)
        .bind(hawier_api::codes::TargetUpdateStatus::Pending.code())
        .bind(now_millis())
        .bind(SYSTEM_PRINCIPAL)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("UPDATE sp_rollout_group SET status = $1 WHERE id = $2")
        .bind(RolloutGroupStatus::Running.code())
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Public hook for mgmt `triggerNextGroup`.
pub async fn start_next_group(pool: &PgPool, tenant: &str, rollout_id: i64) -> AppResult<bool> {
    let mut tx = pool.begin().await?;
    let next: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_rollout_group WHERE rollout = $1 AND status = $2
         ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .bind(rollout_id)
    .bind(RolloutGroupStatus::Scheduled.code())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(group_id) = next else {
        return Ok(false);
    };
    activate_group(&mut tx, tenant, group_id).await?;
    tx.commit().await?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// RUNNING: threshold evaluation, next-group, dynamic groups
// ---------------------------------------------------------------------------

/// The engine's own state machine, unchanged: when nothing is running,
/// start the next scheduled group, then grow a dynamic rollout, then finish.
///
/// Deliberately not the strategy's to decide — a strategy that could leave a
/// rollout in a state the engine does not know about would be a strategy that
/// can wedge the engine.
async fn advance_rollout(
    tx: &mut PgConnection,
    ids: &IncidentGen,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<()> {
    let rollout_id: i64 = rollout.get("id");
    // NEXTGROUP: if nothing is running, start the next scheduled group.
    let still_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_rollout_group WHERE rollout = $1 AND status = $2",
    )
    .bind(rollout_id)
    .bind(RolloutGroupStatus::Running.code())
    .fetch_one(&mut *tx)
    .await?;
    if still_running == 0 {
        let next: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM sp_rollout_group WHERE rollout = $1 AND status = $2
             ORDER BY id LIMIT 1",
        )
        .bind(rollout_id)
        .bind(RolloutGroupStatus::Scheduled.code())
        .fetch_optional(&mut *tx)
        .await?;
        match next {
            Some(group_id) => activate_group(tx, tenant, group_id).await?,
            None => {
                // All static groups done; dynamic rollouts absorb newcomers.
                let dynamic = rollout
                    .get::<Option<bool>, _>("is_dynamic")
                    .unwrap_or(false);
                let grew = if dynamic {
                    grow_dynamic_group(tx, tenant, rollout).await?
                } else {
                    false
                };
                if !grew && !dynamic {
                    finish_rollout(tx, ids, tenant, rollout_id).await?;
                }
                // Dynamic rollouts stay RUNNING, waiting for new targets.
            }
        }
    }
    Ok(())
}
async fn finish_rollout(
    tx: &mut PgConnection,
    ids: &IncidentGen,
    tenant: &str,
    rollout_id: i64,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3
         WHERE id = $4",
    )
    .bind(RolloutStatus::Finished.code())
    .bind(now_millis())
    .bind(SYSTEM_PRINCIPAL)
    .bind(rollout_id)
    .execute(&mut *tx)
    .await?;
    crate::events::emit(
        tx,
        ids,
        tenant,
        hawier_api::ext::kinds::ROLLOUT_FINISHED,
        serde_json::json!({ "rolloutId": rollout_id }),
    )
    .await?;
    info!(rollout_id, "rollout finished");
    Ok(())
}

/// Dynamic rollout: pull in newly matching targets as a fresh group.
async fn grow_dynamic_group(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<bool> {
    let rollout_id: i64 = rollout.get("id");
    let name: String = rollout.get("name");
    let filter: String = rollout
        .get::<Option<String>, _>("target_filter")
        .unwrap_or_default();
    if filter.is_empty() {
        return Ok(false);
    }

    let ds_id: i64 = rollout.get("distribution_set");
    let matching = matching_target_ids(tx, tenant, &filter, ds_id).await?;
    let mut fresh = Vec::new();
    for target_id in matching {
        let already: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sp_rollout_target_group g
             JOIN sp_rollout_group rg ON rg.id = g.rollout_group
             WHERE rg.rollout = $1 AND g.target = $2)",
        )
        .bind(rollout_id)
        .bind(target_id)
        .fetch_one(&mut *tx)
        .await?;
        if !already {
            fresh.push(target_id);
        }
    }
    if fresh.is_empty() {
        return Ok(false);
    }

    let dynamic_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_rollout_group WHERE rollout = $1 AND is_dynamic = TRUE",
    )
    .bind(rollout_id)
    .fetch_one(&mut *tx)
    .await?;
    let now = now_millis();
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_rollout_group
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, status, rollout,
          success_condition, success_condition_exp, success_action, error_action,
          target_percentage, is_dynamic, total_targets)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, 0, '100', 0, 0, 100, TRUE, $7)
         RETURNING id",
    )
    .bind(now)
    .bind(SYSTEM_PRINCIPAL)
    .bind(tenant)
    .bind(format!("{name}-dynamic-{}", dynamic_count + 1))
    .bind(RolloutGroupStatus::Ready.code())
    .bind(rollout_id)
    .bind(fresh.len() as i64)
    .fetch_one(&mut *tx)
    .await?;

    for target_id in &fresh {
        sqlx::query("INSERT INTO sp_rollout_target_group (target, rollout_group) VALUES ($1, $2)")
            .bind(target_id)
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "UPDATE sp_rollout SET total_targets = COALESCE(total_targets, 0) + $1,
             rollout_groups_created = COALESCE(rollout_groups_created, 0) + 1
         WHERE id = $2",
    )
    .bind(fresh.len() as i64)
    .bind(rollout_id)
    .execute(&mut *tx)
    .await?;

    create_scheduled_actions(tx, tenant, rollout, group_id).await?;
    activate_group(tx, tenant, group_id).await?;
    info!(
        rollout_id,
        group_id,
        targets = fresh.len(),
        "dynamic group created"
    );
    Ok(true)
}

// ---------------------------------------------------------------------------
// STOPPING / DELETING
// ---------------------------------------------------------------------------

async fn cancel_rollout_actions(
    tx: &mut PgConnection,
    tenant: &str,
    rollout_id: i64,
) -> AppResult<()> {
    // Scheduled (inactive) actions die immediately; running ones get the
    // cancel handshake.
    sqlx::query(
        "UPDATE sp_action SET status = $1, active = FALSE WHERE rollout = $2 AND status = $3",
    )
    .bind(ActionStatus::Canceled.code())
    .bind(rollout_id)
    .bind(ActionStatus::Scheduled.code())
    .execute(&mut *tx)
    .await?;

    let active = sqlx::query(
        "SELECT id FROM sp_action WHERE rollout = $1 AND active = TRUE AND status <> $2",
    )
    .bind(rollout_id)
    .bind(ActionStatus::Canceling.code())
    .fetch_all(&mut *tx)
    .await?;
    for row in &active {
        let action_id: i64 = row.get("id");
        actions::set_action_status(
            tx,
            action_id,
            ActionStatus::Canceling,
            true,
            SYSTEM_PRINCIPAL,
        )
        .await?;
        actions::insert_status(
            tx,
            tenant,
            action_id,
            ActionStatus::Canceling,
            None,
            None,
            SYSTEM_PRINCIPAL,
            &["Update Server: rollout stopped".to_string()],
        )
        .await?;
    }
    Ok(())
}

async fn stop_rollout(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<()> {
    let rollout_id: i64 = rollout.get("id");
    cancel_rollout_actions(tx, tenant, rollout_id).await?;
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3
         WHERE id = $4",
    )
    .bind(RolloutStatus::Stopped.code())
    .bind(now_millis())
    .bind(SYSTEM_PRINCIPAL)
    .bind(rollout_id)
    .execute(tx)
    .await?;
    info!(rollout_id, "rollout stopped");
    Ok(())
}

async fn delete_rollout(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<()> {
    let rollout_id: i64 = rollout.get("id");
    cancel_rollout_actions(tx, tenant, rollout_id).await?;
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, deleted = TRUE,
             last_modified_at = $2, last_modified_by = $3
         WHERE id = $4",
    )
    .bind(RolloutStatus::Deleted.code())
    .bind(now_millis())
    .bind(SYSTEM_PRINCIPAL)
    .bind(rollout_id)
    .execute(tx)
    .await?;
    info!(rollout_id, "rollout deleted");
    Ok(())
}

// ---------------------------------------------------------------------------
// Target-filter auto-assignment
// ---------------------------------------------------------------------------

/// Assign the configured DS to every filter-matching target that doesn't
/// already run/have it.
pub async fn auto_assign_tick(state: &AppState) -> AppResult<()> {
    let filters = sqlx::query(
        "SELECT id, tenant, query, auto_assign_distribution_set, auto_assign_action_type,
                auto_assign_weight, auto_assign_initiated_by, confirmation_required
         FROM sp_target_filter_query
         WHERE auto_assign_distribution_set IS NOT NULL",
    )
    .fetch_all(&state.pool)
    .await?;

    for filter in &filters {
        let tenant: String = filter.get("tenant");
        let ds_id: i64 = filter.get("auto_assign_distribution_set");
        let query: String = filter.get("query");
        let initiated_by = filter
            .get::<Option<String>, _>("auto_assign_initiated_by")
            .unwrap_or_else(|| SYSTEM_PRINCIPAL.to_string());
        let action_type = filter
            .get::<Option<i32>, _>("auto_assign_action_type")
            .and_then(|c| ActionType::try_from(c).ok())
            .unwrap_or(ActionType::Forced);
        let weight: i32 = filter.get("auto_assign_weight");
        let confirmation_required: Option<bool> = filter.get("confirmation_required");

        let mut tx = state.pool.begin().await?;
        let candidates = matching_target_ids(&mut tx, &tenant, &query, ds_id).await?;
        for target_id in candidates {
            // Skip targets that already point at this DS (assigned or
            // installed) — hawkBit only auto-assigns newcomers.
            let already: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM sp_target
                 WHERE id = $1 AND (assigned_distribution_set = $2
                                    OR installed_distribution_set = $2))",
            )
            .bind(target_id)
            .bind(ds_id)
            .fetch_one(&mut *tx)
            .await?;
            if already {
                continue;
            }
            crate::deploy_logic::assign_in_tx(
                &mut tx,
                &state.incidents,
                &tenant,
                &crate::deploy_logic::AssignRequest {
                    target_id,
                    ds_id,
                    action_type,
                    forced_time: None,
                    weight,
                    initiated_by: initiated_by.clone(),
                    confirmation_required,
                    maintenance_window: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Strategy-driven states: CREATING and RUNNING
// ---------------------------------------------------------------------------

/// CREATING → READY: ask the strategy how to divide the fleet, then apply it.
///
/// The snapshot is read without a lock and the plan applied under one, with
/// the status re-checked: a rollout another node already handled leaves this
/// one with nothing to do rather than a half-applied split.
async fn materialize_rollout(state: &AppState, tenant: &str, rollout_id: i64) -> AppResult<()> {
    let Some(rollout) = sqlx::query(
        "SELECT id, name, target_filter, distribution_set FROM sp_rollout
             WHERE id = $1 AND status = $2",
    )
    .bind(rollout_id)
    .bind(RolloutStatus::Creating.code())
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(());
    };
    let name: String = rollout.get("name");
    let filter: String = rollout
        .get::<Option<String>, _>("target_filter")
        .unwrap_or_default();

    let ds_id: i64 = rollout.get("distribution_set");
    let total_targets = count_matching_targets(state, tenant, &filter, ds_id).await?;
    let definitions = group_definitions(state, rollout_id).await?;
    let mut conn = state.pool.acquire().await?;
    let (strategy, params) = crate::rollout_strategy::strategy_for(&mut conn, rollout_id).await?;
    drop(conn);
    let plan = crate::rollout_strategy::plan(
        state,
        rollout_id,
        &name,
        &strategy,
        &params,
        total_targets,
        definitions,
    )
    .await?;

    let mut tx = state.pool.begin().await?;
    let claimed = sqlx::query(
        "SELECT id FROM sp_rollout WHERE id = $1 AND status = $2 FOR UPDATE SKIP LOCKED",
    )
    .bind(rollout_id)
    .bind(RolloutStatus::Creating.code())
    .fetch_optional(&mut *tx)
    .await?;
    if claimed.is_none() {
        return Ok(());
    }

    crate::rollout_strategy::apply_plan(&mut tx, tenant, rollout_id, &filter, total_targets, &plan)
        .await?;

    let approval = crate::tenant::config_bool(
        &mut *tx,
        tenant,
        crate::tenant::KEY_ROLLOUT_APPROVAL_ENABLED,
        false,
    )
    .await?;
    let next = if approval {
        RolloutStatus::WaitingForApproval
    } else {
        RolloutStatus::Ready
    };
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, total_targets = $2, rollout_groups_created = $3,
             last_modified_at = $4, last_modified_by = $5
         WHERE id = $6",
    )
    .bind(next.code())
    .bind(total_targets)
    .bind(plan.groups.len() as i64)
    .bind(now_millis())
    .bind(SYSTEM_PRINCIPAL)
    .bind(rollout_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    info!(
        rollout_id,
        groups = plan.groups.len(),
        total_targets,
        "rollout materialized"
    );
    Ok(())
}

/// How many targets the rollout's own filter matches and may receive its
/// set — the same population the groups are filled from.
///
/// A count, not a list: the built-in split loaded every matching id into
/// memory, which is not a thing to do to a fleet of millions.
async fn count_matching_targets(
    state: &AppState,
    tenant: &str,
    filter: &str,
    ds_id: i64,
) -> AppResult<i64> {
    if filter.trim().is_empty() {
        return Ok(0);
    }
    let (filter_sql, binds) =
        crate::mgmt::rsql_filter(Some(filter), crate::mgmt::targets::target_fields(), 1)?;
    let typed_only = crate::deploy_logic::typed_only(&state.pool, tenant).await?;
    Ok(crate::rsql::bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_target t WHERE UPPER(t.tenant) = UPPER($1){filter_sql} AND {}",
            crate::deploy_logic::compatible_with_set_sql(ds_id, typed_only)
        ))
        .bind(tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0))
}

async fn group_definitions(
    state: &AppState,
    rollout_id: i64,
) -> AppResult<Vec<hawier_api::ext::GroupDefinition>> {
    let rows = sqlx::query(
        "SELECT id, name, target_percentage, target_filter,
                success_condition_exp, error_condition_exp
         FROM sp_rollout_group WHERE rollout = $1 AND is_dynamic = FALSE ORDER BY id",
    )
    .bind(rollout_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| hawier_api::ext::GroupDefinition {
            id: row.get::<i64, _>("id").to_string(),
            name: row.get("name"),
            target_percentage: row.get("target_percentage"),
            target_filter: row
                .get::<Option<String>, _>("target_filter")
                .unwrap_or_default(),
            success_threshold: row
                .get::<Option<String>, _>("success_condition_exp")
                .and_then(|e| e.trim().parse().ok())
                .unwrap_or(100.0),
            error_threshold: row
                .get::<Option<String>, _>("error_condition_exp")
                .and_then(|e| e.trim().parse().ok()),
        })
        .collect())
}

/// RUNNING: ask the strategy what it makes of the running groups, then apply.
async fn evaluate_rollout(state: &AppState, tenant: &str, rollout_id: i64) -> AppResult<()> {
    let stats = running_group_stats(state, rollout_id).await?;
    let dynamic: bool =
        sqlx::query_scalar("SELECT COALESCE(is_dynamic, FALSE) FROM sp_rollout WHERE id = $1")
            .bind(rollout_id)
            .fetch_one(&state.pool)
            .await?;

    let verdict = if stats.is_empty() {
        // Nothing running: no judgement to ask for, just the state machine.
        hawier_api::ext::RolloutVerdict::default()
    } else {
        let mut conn = state.pool.acquire().await?;
        let (strategy, params) =
            crate::rollout_strategy::strategy_for(&mut conn, rollout_id).await?;
        drop(conn);
        crate::rollout_strategy::evaluate(state, rollout_id, &strategy, &params, dynamic, stats)
            .await?
    };

    let mut tx = state.pool.begin().await?;
    let Some(rollout) = sqlx::query(
        // advance_rollout reads target_filter to grow a dynamic rollout;
        // without it here that read panicked on the first tick after the
        // static groups finished, and a dynamic rollout never grew.
        "SELECT id, is_dynamic, distribution_set, target_filter, action_type, weight,
                forced_time, name
         FROM sp_rollout WHERE id = $1 AND status = $2 FOR UPDATE SKIP LOCKED",
    )
    .bind(rollout_id)
    .bind(RolloutStatus::Running.code())
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };

    for group in &verdict.errored {
        set_group_status(&mut tx, group, RolloutGroupStatus::Error).await?;
    }

    if let Some(reason) = &verdict.pause {
        sqlx::query(
            "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3
             WHERE id = $4",
        )
        .bind(RolloutStatus::Paused.code())
        .bind(now_millis())
        .bind(SYSTEM_PRINCIPAL)
        .bind(rollout_id)
        .execute(&mut *tx)
        .await?;
        crate::events::emit(
            &mut tx,
            &state.incidents,
            tenant,
            hawier_api::ext::kinds::ROLLOUT_PAUSED,
            serde_json::json!({ "rolloutId": rollout_id, "reason": reason }),
        )
        .await?;
        tx.commit().await?;
        info!(rollout_id, reason, "rollout paused by its strategy");
        return Ok(());
    }

    // Success starts the next group and leaves this one running: it is
    // finished only once its stragglers settle, below. Treating the two as
    // one put rollouts into FINISHED with devices still updating.
    for group in &verdict.succeeded {
        if !verdict.errored.contains(group) {
            start_successor(&mut tx, tenant, &rollout, group).await?;
        }
    }
    finish_settled_groups(&mut tx, tenant, &rollout).await?;

    advance_rollout(&mut tx, &state.incidents, tenant, &rollout).await?;
    tx.commit().await?;
    Ok(())
}

/// hawkBit's NEXTGROUP success action: start the group after `group_id`, if
/// it is still waiting. Groups are created in order, so "after" is the next
/// id. A group that has no successor is the last one, and in a dynamic
/// rollout its success is what lets newcomers in — otherwise one device of
/// the last group that never polls would keep them out for good.
///
/// Idempotent: the same group succeeds again on every tick until it
/// settles, and by then its successor is no longer SCHEDULED.
async fn start_successor(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
    group_id: &str,
) -> AppResult<()> {
    let id: i64 = group_id
        .parse()
        .map_err(|_| crate::error::AppError::Internal(anyhow::anyhow!("unusable group id")))?;
    let rollout_id: i64 = rollout.get("id");
    // Only a group of this rollout that is still running: the verdict
    // applies to the state it was asked about, not to whatever it names.
    let Some(row) = sqlx::query(
        "SELECT n.id, n.status FROM sp_rollout_group g
         LEFT JOIN LATERAL (
             SELECT id, status FROM sp_rollout_group
             WHERE rollout = g.rollout AND id > g.id ORDER BY id LIMIT 1
         ) n ON TRUE
         WHERE g.id = $1 AND g.rollout = $2 AND g.status = $3",
    )
    .bind(id)
    .bind(rollout_id)
    .bind(RolloutGroupStatus::Running.code())
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };
    match row.get::<Option<i64>, _>("id") {
        Some(next) => {
            if row.get::<Option<i32>, _>("status") == Some(RolloutGroupStatus::Scheduled.code()) {
                activate_group(tx, tenant, next).await?;
            }
        }
        None => {
            if rollout
                .get::<Option<bool>, _>("is_dynamic")
                .unwrap_or(false)
            {
                grow_dynamic_group(tx, tenant, rollout).await?;
            }
        }
    }
    Ok(())
}

/// A running group is finished when none of its actions is left to wait
/// for — hawkBit's `isRolloutGroupComplete`, met threshold or not. A fact
/// about rows the host owns, so it is not asked of the strategy: a module
/// that could withhold it could keep a settled group running forever.
async fn finish_settled_groups(
    tx: &mut PgConnection,
    tenant: &str,
    rollout: &sqlx::postgres::PgRow,
) -> AppResult<()> {
    let mut terminal = vec![
        ActionStatus::Finished.code(),
        ActionStatus::Error.code(),
        ActionStatus::Canceled.code(),
    ];
    // A download-only action is done once downloaded; a forced one is not.
    if rollout.get::<i32, _>("action_type") == ActionType::DownloadOnly.code() {
        terminal.push(ActionStatus::Downloaded.code());
    }
    sqlx::query(
        "UPDATE sp_rollout_group g SET status = $1
         WHERE g.rollout = $2 AND g.status = $3
           AND NOT EXISTS (SELECT 1 FROM sp_action a
               WHERE a.tenant = UPPER($4) AND a.rollout_group = g.id
                 AND a.status <> ALL($5))",
    )
    .bind(RolloutGroupStatus::Finished.code())
    .bind(rollout.get::<i64, _>("id"))
    .bind(RolloutGroupStatus::Running.code())
    .bind(tenant)
    .bind(&terminal)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn set_group_status(
    tx: &mut PgConnection,
    group_id: &str,
    status: RolloutGroupStatus,
) -> AppResult<()> {
    let id: i64 = group_id
        .parse()
        .map_err(|_| crate::error::AppError::Internal(anyhow::anyhow!("unusable group id")))?;
    sqlx::query("UPDATE sp_rollout_group SET status = $1 WHERE id = $2")
        .bind(status.code())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Counts per running group — what a strategy judges by, and all it gets.
async fn running_group_stats(
    state: &AppState,
    rollout_id: i64,
) -> AppResult<Vec<hawier_api::ext::GroupStats>> {
    let groups = sqlx::query(
        "SELECT id, name, total_targets, success_condition_exp, error_condition_exp
         FROM sp_rollout_group WHERE rollout = $1 AND status = $2 ORDER BY id",
    )
    .bind(rollout_id)
    .bind(RolloutGroupStatus::Running.code())
    .fetch_all(&state.pool)
    .await?;

    let mut stats = Vec::new();
    for group in &groups {
        let group_id: i64 = group.get("id");
        let counts = sqlx::query(
            "SELECT status, COUNT(*) AS c FROM sp_action WHERE rollout_group = $1 GROUP BY status",
        )
        .bind(group_id)
        .fetch_all(&state.pool)
        .await?;
        let mut finished = 0i64;
        let mut errors = 0i64;
        for c in &counts {
            let count: i64 = c.get("c");
            match ActionStatus::try_from(c.get::<i32, _>("status")) {
                Ok(ActionStatus::Finished) | Ok(ActionStatus::Downloaded) => finished += count,
                Ok(ActionStatus::Error) => errors += count,
                _ => {}
            }
        }
        stats.push(hawier_api::ext::GroupStats {
            id: group_id.to_string(),
            name: group.get("name"),
            total: group.get::<Option<i64>, _>("total_targets").unwrap_or(0),
            finished,
            errors,
            success_threshold: group
                .get::<Option<String>, _>("success_condition_exp")
                .and_then(|e| e.trim().parse().ok())
                .unwrap_or(100.0),
            error_threshold: group
                .get::<Option<String>, _>("error_condition_exp")
                .and_then(|e| e.trim().parse().ok()),
        });
    }
    Ok(stats)
}
