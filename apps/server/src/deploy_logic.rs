//! Shared assignment engine: assigning a distribution set to a target
//! creates an `sp_action`. Used by mgmt `assignedDS`, target-filter
//! auto-assign and the rollout engine.

use hawier_api::codes::{ActionStatus, ActionType, TargetUpdateStatus};
use sqlx::{PgConnection, PgPool, Row};

use crate::actions;
use crate::error::{AppError, AppResult};
use crate::state::now_millis;
use crate::tenant;

/// Default action weight when none given (hawkBit's default).
pub const DEFAULT_WEIGHT: i32 = 1000;

pub struct AssignRequest {
    pub target_id: i64,
    pub ds_id: i64,
    pub action_type: ActionType,
    /// Epoch millis for TIMEFORCED switchover.
    pub forced_time: Option<i64>,
    pub weight: i32,
    pub initiated_by: String,
    /// Explicit confirmation requirement (rollout groups / target filters can
    /// override the tenant-wide confirmation flow flag).
    pub confirmation_required: Option<bool>,
    pub maintenance_window: Option<MaintenanceWindow>,
}

pub struct MaintenanceWindow {
    pub schedule: String,
    pub duration: String,
    pub time_zone: String,
}

pub struct AssignOutcome {
    pub action_id: i64,
    pub initial_status: ActionStatus,
}

// ---------------------------------------------------------------------------
// Target type compatibility
// ---------------------------------------------------------------------------
//
// hawkBit 1.0 offers a set only to a target whose type lists the set's type
// as compatible; an untyped target takes anything. One predicate, used by
// assignment, rollout materialization and auto-assignment, so the three can
// never disagree about who may receive what.

/// SQL condition over a target row aliased `t`: it may receive a set of the
/// distribution set type bound at `ds_type` (a placeholder such as `$3`, or
/// any expression yielding the type id). `typed_only` is the hardened rule —
/// an untyped target then receives nothing.
pub fn compatible_sql(ds_type: &str, typed_only: bool) -> String {
    let untyped = if typed_only {
        "FALSE"
    } else {
        "t.target_type IS NULL"
    };
    format!(
        "({untyped} OR EXISTS (SELECT 1 FROM sp_target_type_ds_type r \
          WHERE r.target_type = t.target_type AND r.distribution_set_type = {ds_type}))"
    )
}

/// [`compatible_sql`] for the type of a known set. The id is an `i64`, so
/// inlining it cannot inject anything, and it spares the caller a bind slot
/// in front of an RSQL filter whose binds are positional.
pub fn compatible_with_set_sql(ds_id: i64, typed_only: bool) -> String {
    compatible_sql(
        &format!("(SELECT d.ds_type FROM sp_distribution_set d WHERE d.id = {ds_id})"),
        typed_only,
    )
}

/// The hardened rule's switch: `target.type.required` of the tenant. Read
/// once per request or tick, like every other tenant key.
pub async fn typed_only<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    tenant_name: &str,
) -> AppResult<bool> {
    tenant::config_bool(
        executor,
        tenant_name,
        tenant::KEY_TARGET_TYPE_REQUIRED,
        false,
    )
    .await
}

/// hawkBit renders a set of names with Java's `Collection.toString`.
fn java_list(names: &[String]) -> String {
    format!("[{}]", names.join(", "))
}

/// Request-wide check for "one set, many targets": every listed target of
/// the tenant must be able to receive `ds_id`. Unknown controller ids are
/// left to the caller's 404. Refuses with hawkBit's wording.
pub async fn ensure_targets_compatible(
    conn: &mut PgConnection,
    tenant_name: &str,
    ds_id: i64,
    controller_ids: &[String],
    typed_only: bool,
) -> AppResult<()> {
    let sql = format!(
        "SELECT DISTINCT tt.name AS type_name, t.controller_id
         FROM sp_target t
         LEFT JOIN sp_target_type tt ON tt.id = t.target_type
         WHERE t.controller_id = ANY($1) AND UPPER(t.tenant) = UPPER($2)
           AND NOT {}
         ORDER BY t.controller_id",
        compatible_sql(
            "(SELECT d.ds_type FROM sp_distribution_set d WHERE d.id = $3)",
            typed_only
        )
    );
    let rows = sqlx::query(&sql)
        .bind(controller_ids)
        .bind(tenant_name)
        .bind(ds_id)
        .fetch_all(&mut *conn)
        .await?;
    if rows.is_empty() {
        return Ok(());
    }
    if let Some(untyped) = rows
        .iter()
        .find(|row| row.get::<Option<String>, _>("type_name").is_none())
    {
        return Err(untyped_refusal(&untyped.get::<String, _>("controller_id")));
    }
    let mut types: Vec<String> = rows.iter().map(|row| row.get("type_name")).collect();
    types.sort();
    types.dedup();
    let ds_type: String = sqlx::query_scalar(
        "SELECT y.name FROM sp_distribution_set d
         JOIN sp_distribution_set_type y ON y.id = d.ds_type WHERE d.id = $1",
    )
    .bind(ds_id)
    .fetch_one(&mut *conn)
    .await?;
    Err(AppError::IncompatibleTargetType(format!(
        "Targets of types {} are not compatible with distribution set of type {ds_type}",
        java_list(&types)
    )))
}

/// Request-wide check for "one target, many sets". Unknown set ids are left
/// to the caller's 404.
pub async fn ensure_sets_compatible(
    conn: &mut PgConnection,
    target_id: i64,
    ds_ids: &[i64],
    typed_only: bool,
) -> AppResult<()> {
    let target = sqlx::query(
        "SELECT t.controller_id, tt.name AS type_name
         FROM sp_target t LEFT JOIN sp_target_type tt ON tt.id = t.target_type
         WHERE t.id = $1",
    )
    .bind(target_id)
    .fetch_one(&mut *conn)
    .await?;
    let Some(type_name) = target.get::<Option<String>, _>("type_name") else {
        return if typed_only {
            Err(untyped_refusal(&target.get::<String, _>("controller_id")))
        } else {
            Ok(())
        };
    };
    let incompatible: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT y.name FROM sp_distribution_set d
         JOIN sp_distribution_set_type y ON y.id = d.ds_type
         JOIN sp_target t ON t.id = $2
         WHERE d.id = ANY($1)
           AND NOT EXISTS (SELECT 1 FROM sp_target_type_ds_type r
                           WHERE r.target_type = t.target_type
                             AND r.distribution_set_type = d.ds_type)
         ORDER BY y.name",
    )
    .bind(ds_ids)
    .bind(target_id)
    .fetch_all(&mut *conn)
    .await?;
    if incompatible.is_empty() {
        return Ok(());
    }
    Err(AppError::IncompatibleTargetType(format!(
        "Target of type {type_name} is not compatible with distribution set of types {}",
        java_list(&incompatible)
    )))
}

/// The hardened refusal has no hawkBit wording to copy: hawkBit has no such
/// rule. Same code, so a client switching on it needs one branch.
fn untyped_refusal(controller_id: &str) -> AppError {
    AppError::IncompatibleTargetType(format!(
        "Target {controller_id} has no target type, and this tenant \
         (target.type.required) assigns only to typed targets"
    ))
}

/// Assign a DS to a target inside an existing transaction.
///
/// Default mode supersedes prior active update actions (soft-cancel →
/// CANCELING; with `repository.actions.autoclose.enabled` they are closed
/// as CANCELED immediately, no cancel handshake). With
/// `multi.assignments.enabled` prior actions are left running and the DDI
/// layer picks by weight DESC, id ASC. The new action starts RUNNING or
/// WAIT_FOR_CONFIRMATION.
pub async fn assign_in_tx(
    tx: &mut PgConnection,
    ids: &crate::incident::IncidentGen,
    tenant_name: &str,
    req: &AssignRequest,
) -> AppResult<AssignOutcome> {
    if let Some(window) = &req.maintenance_window {
        crate::maintenance::validate(
            Some(&window.schedule),
            Some(&window.duration),
            Some(&window.time_zone),
        )?;
    }

    let now = now_millis();

    // Lock the target row to serialize concurrent assignments.
    sqlx::query("SELECT id FROM sp_target WHERE id = $1 FOR UPDATE")
        .bind(req.target_id)
        .fetch_optional(&mut *tx)
        .await?;

    // Checked again under the lock: the request-wide precheck ran before it,
    // and a type relation may have changed in between.
    let typed_only = typed_only(&mut *tx, tenant_name).await?;
    ensure_sets_compatible(tx, req.target_id, &[req.ds_id], typed_only).await?;

    let multi_assignments = tenant::config_bool(
        &mut *tx,
        tenant_name,
        tenant::KEY_MULTI_ASSIGNMENTS_ENABLED,
        false,
    )
    .await?;

    if !multi_assignments {
        let autoclose = tenant::config_bool(
            &mut *tx,
            tenant_name,
            tenant::KEY_ACTIONS_AUTOCLOSE_ENABLED,
            false,
        )
        .await?;
        supersede_active_actions(tx, tenant_name, req, autoclose).await?;
    }

    // Implicit lock: freeze the assigned DS + its modules so the content a
    // device receives can no longer change under it.
    let implicit_lock = tenant::config_bool(
        &mut *tx,
        tenant_name,
        tenant::KEY_IMPLICIT_LOCK_ENABLED,
        true,
    )
    .await?;
    if implicit_lock {
        implicitly_lock_ds(tx, req.ds_id, &req.initiated_by).await?;
    }

    // Confirmation gating.
    let confirmation_flow = tenant::config_bool(
        &mut *tx,
        tenant_name,
        tenant::KEY_CONFIRMATION_FLOW_ENABLED,
        false,
    )
    .await?;
    let auto_confirmed =
        sqlx::query("SELECT EXISTS (SELECT 1 FROM sp_target_conf_status WHERE target = $1) AS e")
            .bind(req.target_id)
            .fetch_one(&mut *tx)
            .await?
            .get::<bool, _>("e");
    let needs_confirmation = match req.confirmation_required {
        Some(explicit) => explicit && confirmation_flow && !auto_confirmed,
        None => confirmation_flow && !auto_confirmed,
    };
    let initial_status = if needs_confirmation {
        ActionStatus::WaitForConfirmation
    } else {
        ActionStatus::Running
    };

    let action_id: i64 = sqlx::query(
        "INSERT INTO sp_action
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, active, forced_time, status, distribution_set, target,
          rollout, rollout_group, action_type,
          maintenance_cron_schedule, maintenance_duration, maintenance_time_zone,
          external_ref, weight, initiated_by)
         VALUES ($1, $2, $1, $2, 1, $3, TRUE, $4, $5, $6, $7, NULL, NULL, $8,
                 $9, $10, $11, NULL, $12, $2)
         RETURNING id",
    )
    .bind(now)
    .bind(&req.initiated_by)
    .bind(tenant_name)
    .bind(req.forced_time)
    .bind(initial_status.code())
    .bind(req.ds_id)
    .bind(req.target_id)
    .bind(req.action_type.code())
    .bind(req.maintenance_window.as_ref().map(|m| m.schedule.as_str()))
    .bind(req.maintenance_window.as_ref().map(|m| m.duration.as_str()))
    .bind(
        req.maintenance_window
            .as_ref()
            .map(|m| m.time_zone.as_str()),
    )
    .bind(req.weight)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    actions::insert_status(
        tx,
        tenant_name,
        action_id,
        initial_status,
        None,
        None,
        &req.initiated_by,
        &[format!(
            "Update Server: Assignment initiated by user '{}'",
            req.initiated_by
        )],
    )
    .await?;

    sqlx::query(
        "UPDATE sp_target
         SET assigned_distribution_set = $1, update_status = $2,
             last_modified_at = $3, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $5",
    )
    .bind(req.ds_id)
    .bind(TargetUpdateStatus::Pending.code())
    .bind(now)
    .bind(&req.initiated_by)
    .bind(req.target_id)
    .execute(&mut *tx)
    .await?;

    crate::events::emit(
        tx,
        ids,
        tenant_name,
        hawier_api::ext::kinds::ASSIGNMENT_CREATED,
        serde_json::json!({
            "actionId": action_id,
            "targetId": req.target_id,
            "distributionSetId": req.ds_id,
            "initiatedBy": req.initiated_by,
        }),
    )
    .await?;

    Ok(AssignOutcome {
        action_id,
        initial_status,
    })
}

/// Supersede prior active update actions: soft-cancel (CANCELING, the
/// device sees a cancelAction and confirms) or — with autoclose — close
/// them as CANCELED right away without the handshake.
async fn supersede_active_actions(
    tx: &mut PgConnection,
    tenant_name: &str,
    req: &AssignRequest,
    autoclose: bool,
) -> AppResult<()> {
    let prior = sqlx::query(
        "SELECT id FROM sp_action
         WHERE target = $1 AND active = TRUE AND status <> $2",
    )
    .bind(req.target_id)
    .bind(ActionStatus::Canceling.code())
    .fetch_all(&mut *tx)
    .await?;
    let (status, active, message) = if autoclose {
        (
            ActionStatus::Canceled,
            false,
            "Update Server: cancel obsolete action due to new update (autoclose)",
        )
    } else {
        (
            ActionStatus::Canceling,
            true,
            "Update Server: cancel obsolete action due to new update",
        )
    };
    for row in &prior {
        let action_id: i64 = row.get("id");
        actions::set_action_status(tx, action_id, status, active, &req.initiated_by).await?;
        actions::insert_status(
            tx,
            tenant_name,
            action_id,
            status,
            None,
            None,
            &req.initiated_by,
            &[message.to_string()],
        )
        .await?;
    }
    Ok(())
}

/// Lock the DS and all its software modules (idempotent).
pub async fn implicitly_lock_ds(
    tx: &mut PgConnection,
    ds_id: i64,
    modified_by: &str,
) -> AppResult<()> {
    let now = now_millis();
    sqlx::query(
        "UPDATE sp_distribution_set
         SET locked = TRUE, last_modified_at = $1, last_modified_by = $2,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $3 AND locked IS NOT TRUE",
    )
    .bind(now)
    .bind(modified_by)
    .bind(ds_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE sp_software_module
         SET locked = TRUE, last_modified_at = $1, last_modified_by = $2,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE locked IS NOT TRUE
           AND id IN (SELECT sm_id FROM sp_ds_sm WHERE ds_id = $3)",
    )
    .bind(now)
    .bind(modified_by)
    .bind(ds_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Convenience wrapper opening its own transaction.
pub async fn assign(
    pool: &PgPool,
    ids: &crate::incident::IncidentGen,
    tenant_name: &str,
    req: &AssignRequest,
) -> AppResult<AssignOutcome> {
    let mut tx = pool.begin().await?;
    let outcome = assign_in_tx(&mut tx, ids, tenant_name, req).await?;
    tx.commit().await?;
    Ok(outcome)
}
