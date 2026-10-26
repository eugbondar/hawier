//! Rollout strategy: how a rollout is divided, and when it may move on.
//!
//! The engine keeps its mechanics — creating SCHEDULED actions, claiming a
//! rollout, activating a group, growing a dynamic one, finishing — and hands
//! out exactly two decisions: how to split the fleet, and whether a running
//! group is done. hawkBit's percentage arithmetic is one strategy among
//! others now (`apps/ext-rollout-classic`), which is what lets a hardened
//! node simply not load it.
//!
//! Two rules shape the contract:
//!
//! **The extension is never called inside the claim transaction.** The host
//! reads a snapshot without a lock, asks, then claims the rollout and applies
//! the answer — re-checking the state it planned against, so a rollout that
//! moved meanwhile is skipped rather than half-applied.
//!
//! **The extension never sees the targets.** A rollout can cover tens of
//! millions of devices, so it is told how many there are and answers with
//! selectors the host runs as SQL. That is also why this path no longer loads
//! every matching target id into memory the way the built-in split did.

use hawier_api::codes::RolloutGroupStatus;
use hawier_api::ext::{
    points, GroupDefinition, GroupPlan, GroupSelector, GroupStats, PlannedGroup, RolloutPlanInput,
    RolloutStats, RolloutVerdict,
};
use serde_json::Value;
use sqlx::{PgConnection, Row};

use crate::error::{AppError, AppResult};
use crate::ext;
use crate::state::AppState;

/// Strategy for a rollout: its own if it has one, else the node's default.
pub async fn strategy_for(conn: &mut PgConnection, rollout_id: i64) -> AppResult<(String, Value)> {
    let row = sqlx::query("SELECT strategy, params FROM hawier_rollout_ext WHERE rollout = $1")
        .bind(rollout_id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(match row {
        Some(row) => (row.get("strategy"), row.get("params")),
        None => ("default".to_string(), Value::Object(Default::default())),
    })
}

fn extension(state: &AppState) -> AppResult<&ext::wasm::Extension> {
    state.ext.rollout.as_deref().ok_or_else(|| {
        // Deliberately not a fallback to the old built-in split: a node given
        // no strategy has not been told how to divide a fleet, and guessing
        // is how a hardened deployment quietly gets hawkBit's semantics back.
        AppError::Unavailable("no rollout strategy extension configured".to_string())
    })
}

/// Ask the strategy how to divide this rollout.
/// The strategy is passed in rather than re-read here: `create` validates a
/// choice it has not committed yet, and a second connection would not see it.
pub async fn plan(
    state: &AppState,
    rollout_id: i64,
    name: &str,
    strategy: &str,
    params: &Value,
    total_targets: i64,
    groups: Vec<GroupDefinition>,
) -> AppResult<GroupPlan> {
    ext::call(
        extension(state)?,
        points::ROLLOUT_PLAN,
        &RolloutPlanInput {
            rollout: rollout_id.to_string(),
            name: name.to_string(),
            strategy: strategy.to_string(),
            params: params.clone(),
            total_targets,
            groups,
        },
    )
    .await
}

/// Ask the strategy what it makes of the running groups.
pub async fn evaluate(
    state: &AppState,
    rollout_id: i64,
    strategy: &str,
    params: &Value,
    dynamic: bool,
    groups: Vec<GroupStats>,
) -> AppResult<RolloutVerdict> {
    ext::call(
        extension(state)?,
        points::ROLLOUT_EVALUATE,
        &RolloutStats {
            rollout: rollout_id.to_string(),
            strategy: strategy.to_string(),
            params: params.clone(),
            dynamic,
            groups,
        },
    )
    .await
}

/// The bucket a device sits in, derived from its controller id.
///
/// Stable across nodes and restarts, needs nothing from the device, and does
/// not move when one re-registers. 28 bits keeps the value non-negative
/// without the overflow `abs()` on a 32-bit hash would risk.
fn bucket_sql(modulo: i64) -> String {
    format!("mod(('x' || substr(md5(t.controller_id), 1, 7))::bit(28)::int, {modulo})")
}

/// Fill the planned groups, in order, from the rollout's candidates.
///
/// Runs entirely in SQL: no target id is ever carried in memory, and a target
/// already taken by an earlier group is excluded, so overlapping selectors
/// (two bucket windows that happen to cross) cannot double-assign anyone.
pub async fn apply_plan(
    tx: &mut PgConnection,
    tenant: &str,
    rollout_id: i64,
    rollout_filter: &str,
    total_targets: i64,
    plan: &GroupPlan,
) -> AppResult<()> {
    // A strategy may invent its own groups instead of filling the declared
    // ones (a canary decides its own waves). Whatever it did not claim is
    // removed, or the rollout carries phantom groups with no targets that an
    // operator has to work out the meaning of.
    let claimed: Vec<i64> = plan
        .groups
        .iter()
        .filter_map(|g| g.id.as_ref().and_then(|id| id.parse::<i64>().ok()))
        .collect();
    sqlx::query(
        "DELETE FROM sp_rollout_group
         WHERE rollout = $1 AND is_dynamic = FALSE AND NOT (id = ANY($2))",
    )
    .bind(rollout_id)
    .bind(&claimed)
    .execute(&mut *tx)
    .await?;

    for planned in &plan.groups {
        let group_id = match &planned.id {
            Some(id) => id
                .parse::<i64>()
                .map_err(|_| AppError::Internal(anyhow::anyhow!("unusable group id '{id}'")))?,
            // A strategy may invent groups; a declared one is reused.
            None => create_group(tx, tenant, rollout_id, planned).await?,
        };
        let taken = fill_group(
            tx,
            tenant,
            rollout_id,
            group_id,
            rollout_filter,
            total_targets,
            &planned.selector,
        )
        .await?;

        sqlx::query(
            "UPDATE sp_rollout_group
             SET total_targets = $1, status = $2,
                 success_condition_exp = $4, error_condition_exp = $5
             WHERE id = $3",
        )
        .bind(taken)
        .bind(RolloutGroupStatus::Ready.code())
        .bind(group_id)
        .bind(planned.success_threshold.to_string())
        .bind(planned.error_threshold.map(|t| t.to_string()))
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn create_group(
    tx: &mut PgConnection,
    tenant: &str,
    rollout_id: i64,
    planned: &PlannedGroup,
) -> AppResult<i64> {
    use hawier_api::codes::rollout_group_ordinals as ords;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_rollout_group
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, status, rollout,
          success_condition, success_condition_exp, success_action,
          error_condition, error_condition_exp, error_action,
          target_percentage, confirmation_required, is_dynamic)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6,
                 $7, $8, $9, $10, $11, $12, NULL, NULL, FALSE)
         RETURNING id",
    )
    .bind(crate::state::now_millis())
    .bind(crate::db::model::SYSTEM_PRINCIPAL)
    .bind(tenant)
    .bind(&planned.name)
    .bind(RolloutGroupStatus::Creating.code())
    .bind(rollout_id)
    .bind(ords::SUCCESS_CONDITION_THRESHOLD)
    .bind(planned.success_threshold.to_string())
    .bind(ords::SUCCESS_ACTION_NEXTGROUP)
    .bind(
        planned
            .error_threshold
            .map(|_| ords::ERROR_CONDITION_THRESHOLD),
    )
    .bind(planned.error_threshold.map(|t| t.to_string()))
    .bind(ords::ERROR_ACTION_PAUSE)
    .fetch_one(&mut *tx)
    .await?;
    Ok(id)
}

/// Candidates: matching the rollout's filter, not already taken by one of its
/// groups. Everything a selector does narrows this further.
async fn fill_group(
    tx: &mut PgConnection,
    tenant: &str,
    rollout_id: i64,
    group_id: i64,
    rollout_filter: &str,
    total_targets: i64,
    selector: &GroupSelector,
) -> AppResult<i64> {
    // $1 tenant, $2 rollout, $3 group — the filter's binds follow those.
    let (filter_sql, binds) = crate::mgmt::rsql_filter(
        Some(rollout_filter).filter(|f| !f.trim().is_empty()),
        crate::mgmt::targets::target_fields(),
        3,
    )?;

    let mut extra = String::new();
    let mut limit = String::new();
    match selector {
        GroupSelector::Share { percentage } => {
            // Share of the rollout's whole population, taken from what is
            // left — the same arithmetic the built-in split used.
            let take = ((total_targets as f64) * percentage / 100.0)
                .ceil()
                .max(0.0) as i64;
            limit = format!(" LIMIT {take}");
        }
        GroupSelector::Filter { rsql } => {
            let (group_sql, group_binds) = crate::mgmt::rsql_filter(
                Some(rsql.as_str()).filter(|f| !f.trim().is_empty()),
                crate::mgmt::targets::target_fields(),
                3 + binds.len(),
            )?;
            if !group_binds.is_empty() || !group_sql.is_empty() {
                extra = group_sql;
            }
            // Binds are positional, so the group's follow the rollout's.
            return run_fill(
                tx,
                tenant,
                rollout_id,
                group_id,
                &filter_sql,
                &extra,
                &limit,
                &binds,
                &group_binds,
            )
            .await;
        }
        GroupSelector::Bucket { from, to, modulo } => {
            let modulo = (*modulo).max(2);
            extra = format!(
                " AND {b} >= {from} AND {b} < {to}",
                b = bucket_sql(modulo),
                from = (*from).max(0),
                to = (*to).max(0)
            );
        }
        GroupSelector::Rest => {}
    }

    run_fill(
        tx,
        tenant,
        rollout_id,
        group_id,
        &filter_sql,
        &extra,
        &limit,
        &binds,
        &[],
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_fill(
    tx: &mut PgConnection,
    tenant: &str,
    rollout_id: i64,
    group_id: i64,
    filter_sql: &str,
    extra: &str,
    limit: &str,
    binds: &[crate::rsql::Value],
    extra_binds: &[crate::rsql::Value],
) -> AppResult<i64> {
    // Only targets that may receive the rollout's set are candidates: the
    // strategy decides how to split the population, never who is in it.
    let compatible = crate::deploy_logic::compatible_sql(
        "(SELECT d.ds_type FROM sp_rollout ro
          JOIN sp_distribution_set d ON d.id = ro.distribution_set WHERE ro.id = $2)",
        crate::deploy_logic::typed_only(&mut *tx, tenant).await?,
    );
    let sql = format!(
        "INSERT INTO sp_rollout_target_group (target, rollout_group)
         SELECT t.id, $3 FROM sp_target t
         WHERE UPPER(t.tenant) = UPPER($1){filter_sql}{extra}
           AND {compatible}
           AND NOT EXISTS (
             SELECT 1 FROM sp_rollout_target_group rtg
             JOIN sp_rollout_group g ON g.id = rtg.rollout_group
             WHERE rtg.target = t.id AND g.rollout = $2)
         ORDER BY t.id{limit}
         ON CONFLICT DO NOTHING"
    );
    let query = sqlx::query(&sql)
        .bind(tenant)
        .bind(rollout_id)
        .bind(group_id);
    let query = crate::rsql::bind_values(query, binds);
    let query = crate::rsql::bind_values(query, extra_binds);
    Ok(query.execute(&mut *tx).await?.rows_affected() as i64)
}
