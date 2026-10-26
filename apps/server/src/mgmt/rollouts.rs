//! `/rest/v1/rollouts` — CRUD + lifecycle. The background engine that
//! materializes groups and advances them lives in [`crate::rollout`]
//! (control node only); these handlers only flip statuses and validate.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hawier_api::codes::{
    rollout_group_ordinals as ords, ActionStatus, RolloutGroupStatus, RolloutStatus,
};
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{
    body_str, order_by, paged, parse_action_type, put_audit, request_base, require_str,
    rsql_filter, ListParams,
};
use crate::auth::MgmtIdentity;
use crate::deploy_logic;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};
use crate::tenant;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/rollouts", get(list).post(create))
        .route(
            "/rest/v1/rollouts/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route("/rest/v1/rollouts/{id}/start", post(start))
        .route("/rest/v1/rollouts/{id}/pause", post(pause))
        .route("/rest/v1/rollouts/{id}/resume", post(resume))
        .route("/rest/v1/rollouts/{id}/approve", post(approve))
        .route("/rest/v1/rollouts/{id}/deny", post(deny))
        .route("/rest/v1/rollouts/{id}/stop", post(stop))
        .route(
            "/rest/v1/rollouts/{id}/triggerNextGroup",
            post(trigger_next_group),
        )
        .route("/rest/v1/rollouts/{id}/retry", post(retry))
        .route("/rest/v1/rollouts/{id}/deploygroups", get(list_groups))
        .route(
            "/rest/v1/rollouts/{id}/deploygroups/{group_id}",
            get(get_group),
        )
        .route(
            "/rest/v1/rollouts/{id}/deploygroups/{group_id}/targets",
            get(group_targets),
        )
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "r.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "r.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "r.description",
                kind: Kind::Text,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "r.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "r.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            "distributionset",
            Field::Column {
                sql: "r.distribution_set",
                kind: Kind::Int,
            },
        ),
        (
            "totaltargets",
            Field::Column {
                sql: "r.total_targets",
                kind: Kind::Int,
            },
        ),
        (
            "status",
            Field::EnumColumn {
                sql: "r.status",
                map: &[
                    ("creating", 0),
                    ("ready", 1),
                    ("paused", 2),
                    ("starting", 3),
                    ("stopped", 4),
                    ("running", 5),
                    ("finished", 6),
                    ("deleting", 9),
                    ("deleted", 10),
                    ("waiting_for_approval", 11),
                    ("approval_denied", 12),
                    ("stopping", 13),
                ],
            },
        ),
    ])
});

pub fn rollout_status_name(code: i32) -> String {
    RolloutStatus::try_from(code)
        .map(|s| {
            match s {
                RolloutStatus::Creating => "creating",
                RolloutStatus::Ready => "ready",
                RolloutStatus::Paused => "paused",
                RolloutStatus::Starting => "starting",
                RolloutStatus::Stopped => "stopped",
                RolloutStatus::Running => "running",
                RolloutStatus::Finished => "finished",
                RolloutStatus::Deleting => "deleting",
                RolloutStatus::Deleted => "deleted",
                RolloutStatus::WaitingForApproval => "waiting_for_approval",
                RolloutStatus::ApprovalDenied => "approval_denied",
                RolloutStatus::Stopping => "stopping",
            }
            .to_string()
        })
        .unwrap_or_else(|_| code.to_string())
}

fn group_status_name(code: i32) -> String {
    RolloutGroupStatus::try_from(code)
        .map(|s| {
            match s {
                RolloutGroupStatus::Ready => "ready",
                RolloutGroupStatus::Scheduled => "scheduled",
                RolloutGroupStatus::Finished => "finished",
                RolloutGroupStatus::Error => "error",
                RolloutGroupStatus::Running => "running",
                RolloutGroupStatus::Creating => "creating",
            }
            .to_string()
        })
        .unwrap_or_else(|_| code.to_string())
}

const SELECT: &str = "SELECT r.id, r.created_at, r.created_by, r.last_modified_at,
       r.last_modified_by, r.description, r.name, r.status, r.distribution_set,
       r.target_filter, r.forced_time, r.total_targets, r.rollout_groups_created,
       r.start_at, r.deleted, r.action_type, r.approval_decided_by, r.approval_remark,
       r.weight, r.is_dynamic
FROM sp_rollout r";

async fn rollout_json(state: &AppState, row: &PgRow, base: &str) -> AppResult<Value> {
    let id: i64 = row.get("id");
    // Per-status action counts for totalTargetsPerStatus.
    let counts = sqlx::query(
        "SELECT status, active, COUNT(*) AS c FROM sp_action WHERE rollout = $1
         GROUP BY status, active",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let mut running = 0i64;
    let mut scheduled = 0i64;
    let mut finished = 0i64;
    let mut cancelled = 0i64;
    let mut error = 0i64;
    for c in &counts {
        let count: i64 = c.get("c");
        match ActionStatus::try_from(c.get::<i32, _>("status")) {
            Ok(ActionStatus::Scheduled) => scheduled += count,
            Ok(ActionStatus::Finished) => finished += count,
            Ok(ActionStatus::Error) => error += count,
            Ok(ActionStatus::Canceled) | Ok(ActionStatus::Canceling) => cancelled += count,
            _ => running += count,
        }
    }
    let total_targets = row.get::<Option<i64>, _>("total_targets").unwrap_or(0);
    let not_started = (total_targets - running - scheduled - finished - cancelled - error).max(0);

    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "distributionSetId": row.get::<i64, _>("distribution_set"),
        "targetFilterQuery": row.get::<Option<String>, _>("target_filter"),
        "status": rollout_status_name(row.get::<i32, _>("status")),
        "totalTargets": total_targets,
        "totalTargetsPerStatus": {
            "running": running,
            "notstarted": not_started,
            "scheduled": scheduled,
            "cancelled": cancelled,
            "finished": finished,
            "error": error,
        },
        "totalGroups": row.get::<Option<i64>, _>("rollout_groups_created"),
        "type": super::action_type_name(row.get::<i32, _>("action_type")),
        "weight": row.get::<i32, _>("weight"),
        "forcetime": row.get::<Option<i64>, _>("forced_time"),
        "startAt": row.get::<Option<i64>, _>("start_at"),
        "deleted": row.get::<Option<bool>, _>("deleted").unwrap_or(false),
        "dynamic": row.get::<Option<bool>, _>("is_dynamic").unwrap_or(false),
        "approvalRemark": row.get::<Option<String>, _>("approval_remark"),
        "approveDecidedBy": row.get::<Option<String>, _>("approval_decided_by"),
        "_links": {
            "self": { "href": format!("{base}/rest/v1/rollouts/{id}") },
            "start": { "href": format!("{base}/rest/v1/rollouts/{id}/start") },
            "pause": { "href": format!("{base}/rest/v1/rollouts/{id}/pause") },
            "resume": { "href": format!("{base}/rest/v1/rollouts/{id}/resume") },
            "approve": { "href": format!("{base}/rest/v1/rollouts/{id}/approve") },
            "deny": { "href": format!("{base}/rest/v1/rollouts/{id}/deny") },
            "triggerNextGroup": { "href": format!("{base}/rest/v1/rollouts/{id}/triggerNextGroup") },
            "groups": { "href": format!("{base}/rest/v1/rollouts/{id}/deploygroups") }
        }
    });
    put_audit(&mut value, row);
    Ok(value)
}

async fn load_rollout(state: &AppState, tenant: &str, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{SELECT} WHERE r.id = $1 AND UPPER(r.tenant) = UPPER($2) AND r.deleted IS NOT TRUE"
    ))
    .bind(id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("rollout {id} not found")))
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "r.id ASC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_rollout r
             WHERE UPPER(r.tenant) = UPPER($1) AND r.deleted IS NOT TRUE{filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(r.tenant) = UPPER($1) AND r.deleted IS NOT TRUE{filter_sql} \
             {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut content = Vec::with_capacity(rows.len());
    for row in &rows {
        content.push(rollout_json(&state, row, &base).await?);
    }
    Ok(paged(content, total))
}

/// POST /rest/v1/rollouts — creates the rollout row + group definitions;
/// the engine materializes targets asynchronously (CREATING → READY, or
/// WAITING_FOR_APPROVAL when the approval workflow is on).
async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = require_str(&body, "name")?;
    let ds_id = body
        .get("distributionSetId")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::BadRequest("'distributionSetId' is required".into()))?;
    let target_filter = require_str(&body, "targetFilterQuery")?;
    super::targets::validate_rsql(&target_filter)?;

    let ds: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) AND deleted IS NOT TRUE
           AND valid IS NOT FALSE",
    )
    .bind(ds_id)
    .bind(&identity.tenant)
    .fetch_optional(&state.pool)
    .await?;
    if ds.is_none() {
        return Err(AppError::NotFound(format!(
            "distribution set {ds_id} not found or invalid"
        )));
    }

    // Implicit lock: a rollout freezes its DS content just like a direct
    // assignment does.
    let implicit_lock = crate::tenant::config_bool(
        &state.pool,
        &identity.tenant,
        crate::tenant::KEY_IMPLICIT_LOCK_ENABLED,
        true,
    )
    .await?;
    if implicit_lock {
        let mut tx = state.pool.begin().await?;
        deploy_logic::implicitly_lock_ds(&mut tx, ds_id, &identity.username).await?;
        tx.commit().await?;
    }

    let action_type = match body_str(&body, "type") {
        Some(t) => parse_action_type(&t)?,
        None => hawier_api::codes::ActionType::Forced,
    };
    let weight = body
        .get("weight")
        .and_then(|v| v.as_i64())
        .map(|w| w as i32)
        .unwrap_or(deploy_logic::DEFAULT_WEIGHT);
    let dynamic = body
        .get("dynamic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let now = now_millis();

    // Rollout, its groups and the group count are one transaction: a
    // half-created rollout would be picked up by the engine as if it were
    // whole, and it is also what lets the "created" event share the fate of
    // the thing it announces.
    let mut tx = state.pool.begin().await?;

    let rollout_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_rollout
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, description, name, status, distribution_set, target_filter,
          forced_time, start_at, deleted, action_type, weight, is_dynamic)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7, $8, $9, $10, FALSE, $11, $12, $13)
         RETURNING id",
    )
    .bind(now)
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(body_str(&body, "description"))
    .bind(&name)
    .bind(RolloutStatus::Creating.code())
    .bind(ds_id)
    .bind(&target_filter)
    .bind(body.get("forcetime").and_then(|v| v.as_i64()))
    .bind(body.get("startAt").and_then(|v| v.as_i64()))
    .bind(action_type.code())
    .bind(weight)
    .bind(dynamic)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::Conflict(format!("rollout '{name}' already exists"))
        }
        _ => AppError::Db(e),
    })?;

    // Group definitions: explicit `groups` list wins over `amountGroups`.
    let default_success_expr = body
        .pointer("/successCondition/expression")
        .and_then(|v| v.as_str())
        .unwrap_or("100")
        .to_string();
    let default_error_expr = body
        .pointer("/errorCondition/expression")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let confirmation_required = body.get("confirmationRequired").and_then(|v| v.as_bool());

    let group_defs: Vec<Value> = match body.get("groups").and_then(|v| v.as_array()) {
        Some(groups) if !groups.is_empty() => groups.clone(),
        _ => {
            let amount = body
                .get("amountGroups")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .clamp(1, 500);
            let percentage = 100.0 / amount as f64;
            (1..=amount)
                .map(|n| {
                    json!({
                        "name": format!("group-{n}"),
                        "targetPercentage": percentage,
                    })
                })
                .collect()
        }
    };

    for (index, def) in group_defs.iter().enumerate() {
        let group_name = body_str(def, "name").unwrap_or_else(|| format!("group-{}", index + 1));
        let success_expr = def
            .pointer("/successCondition/expression")
            .and_then(|v| v.as_str())
            .unwrap_or(&default_success_expr);
        let error_expr = def
            .pointer("/errorCondition/expression")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| default_error_expr.clone());
        // NARROW: three of the columns written here are write-only.
        // `target_filter` is stored and never read (rollout::materialize
        // splits by percentage alone), and the success/error *action*
        // ordinals are constants that evaluate_running never branches on —
        // it always behaves as THRESHOLD + NEXTGROUP. The columns look
        // functional and are not. See SCHEMA_NOTES.md #22 and API_NOTES.md §7.
        sqlx::query(
            "INSERT INTO sp_rollout_group
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, description, name, status, rollout,
              success_condition, success_condition_exp, success_action, success_action_exp,
              error_condition, error_condition_exp, error_action, error_action_exp,
              target_percentage, target_filter, confirmation_required, is_dynamic)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7,
                     $8, $9, $10, NULL, $11, $12, $13, NULL, $14, $15, $16, FALSE)",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(body_str(def, "description"))
        .bind(&group_name)
        .bind(RolloutGroupStatus::Creating.code())
        .bind(rollout_id)
        .bind(ords::SUCCESS_CONDITION_THRESHOLD)
        .bind(success_expr)
        .bind(ords::SUCCESS_ACTION_NEXTGROUP)
        .bind(error_expr.as_ref().map(|_| ords::ERROR_CONDITION_THRESHOLD))
        .bind(error_expr)
        .bind(ords::ERROR_ACTION_PAUSE)
        .bind(def.get("targetPercentage").and_then(|v| v.as_f64()))
        .bind(body_str(def, "targetFilterQuery"))
        .bind(
            def.get("confirmationRequired")
                .and_then(|v| v.as_bool())
                .or(confirmation_required),
        )
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("UPDATE sp_rollout SET rollout_groups_created = $1 WHERE id = $2")
        .bind(group_defs.len() as i64)
        .bind(rollout_id)
        .execute(&mut *tx)
        .await?;

    // A strategy other than the node's default is recorded beside the
    // rollout; `sp_rollout` is hawkBit's and stays untouched.
    if let Some(strategy) = body.get("strategy").and_then(|v| v.as_str()) {
        let strategy = strategy.trim();
        if !strategy.is_empty() {
            let params = body
                .get("strategyParams")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            sqlx::query(
                "INSERT INTO hawier_rollout_ext (rollout, tenant, strategy, params)
                 VALUES ($1, UPPER($2), $3, $4)",
            )
            .bind(rollout_id)
            .bind(&identity.tenant)
            .bind(strategy)
            .bind(&params)
            .execute(&mut *tx)
            .await?;

            // Ask the strategy whether it can plan this at all. A name no
            // loaded module answers to would otherwise produce a rollout that
            // sits in CREATING forever, looking merely slow.
            crate::rollout_strategy::plan(
                &state,
                rollout_id,
                &name,
                strategy,
                &params,
                0,
                Vec::new(),
            )
            .await
            .map_err(|error| {
                AppError::BadRequest(format!(
                    "strategy '{strategy}' cannot plan this rollout: {error}"
                ))
            })?;
        }
    }

    crate::events::emit(
        &mut tx,
        &state.incidents,
        &identity.tenant,
        hawier_api::ext::kinds::ROLLOUT_CREATED,
        serde_json::json!({
            "rolloutId": rollout_id,
            "name": name,
            "distributionSetId": ds_id,
            "targetFilterQuery": target_filter,
            "groups": group_defs.len(),
            "createdBy": identity.username,
        }),
    )
    .await?;

    tx.commit().await?;

    let row = load_rollout(&state, &identity.tenant, rollout_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(rollout_json(&state, &row, &request_base(&headers)).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    Ok(Json(
        rollout_json(&state, &row, &request_base(&headers)).await?,
    ))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_rollout(&state, &identity.tenant, id).await?;
    sqlx::query(
        "UPDATE sp_rollout SET
             name = COALESCE($1, name),
             description = COALESCE($2, description),
             last_modified_at = $3, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $5",
    )
    .bind(body_str(&body, "name"))
    .bind(body_str(&body, "description"))
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&state.pool)
    .await?;
    let row = load_rollout(&state, &identity.tenant, id).await?;
    Ok(Json(
        rollout_json(&state, &row, &request_base(&headers)).await?,
    ))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    let status: i32 = row.get("status");
    let has_actions: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sp_action WHERE rollout = $1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if !has_actions
        && (status == RolloutStatus::Creating.code()
            || status == RolloutStatus::Ready.code()
            || status == RolloutStatus::WaitingForApproval.code()
            || status == RolloutStatus::ApprovalDenied.code())
    {
        // Nothing ran yet: hard delete (groups cascade).
        sqlx::query("DELETE FROM sp_rollout WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await?;
    } else {
        // Engine finishes the soft delete (closes scheduled actions).
        set_status(&state, &identity, id, RolloutStatus::Deleting).await?;
    }
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

async fn set_status(
    state: &AppState,
    identity: &MgmtIdentity,
    id: i64,
    status: RolloutStatus,
) -> AppResult<()> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, last_modified_at = $2, last_modified_by = $3,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $4",
    )
    .bind(status.code())
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    // The engine emits this when a threshold pauses a rollout; an operator
    // pressing pause is the same fact about the same rollout, and a
    // subscriber asking to hear about pauses means both. Emitted here rather
    // than in the handler so the next transition that deserves an event
    // cannot be added one path at a time.
    if status == RolloutStatus::Paused {
        crate::events::emit(
            &mut tx,
            &state.incidents,
            &identity.tenant,
            hawier_api::ext::kinds::ROLLOUT_PAUSED,
            serde_json::json!({
                "rolloutId": id,
                "reason": format!("paused by {}", identity.username),
            }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn expect_status(row: &PgRow, allowed: &[RolloutStatus]) -> AppResult<()> {
    let status: i32 = row.get("status");
    if allowed.iter().any(|s| s.code() == status) {
        Ok(())
    } else {
        Err(AppError::Conflict(format!(
            "rollout is in state '{}', expected one of {:?}",
            rollout_status_name(status),
            allowed
                .iter()
                .map(|s| rollout_status_name(s.code()))
                .collect::<Vec<_>>()
        )))
    }
}

async fn start(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::Ready])?;
    set_status(&state, &identity, id, RolloutStatus::Starting).await?;
    Ok(StatusCode::OK)
}

async fn pause(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::Running])?;
    set_status(&state, &identity, id, RolloutStatus::Paused).await?;
    Ok(StatusCode::OK)
}

async fn resume(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::Paused])?;
    set_status(&state, &identity, id, RolloutStatus::Running).await?;
    Ok(StatusCode::OK)
}

async fn approve(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    body: Option<Json<Value>>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::WaitingForApproval])?;
    let remark = body.as_ref().and_then(|Json(b)| body_str(b, "remark"));
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, approval_decided_by = $2, approval_remark = $3,
             last_modified_at = $4, last_modified_by = $2
         WHERE id = $5",
    )
    .bind(RolloutStatus::Ready.code())
    .bind(&identity.username)
    .bind(remark)
    .bind(now_millis())
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::OK)
}

async fn deny(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    body: Option<Json<Value>>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::WaitingForApproval])?;
    let remark = body.as_ref().and_then(|Json(b)| body_str(b, "remark"));
    sqlx::query(
        "UPDATE sp_rollout SET status = $1, approval_decided_by = $2, approval_remark = $3,
             last_modified_at = $4, last_modified_by = $2
         WHERE id = $5",
    )
    .bind(RolloutStatus::ApprovalDenied.code())
    .bind(&identity.username)
    .bind(remark)
    .bind(now_millis())
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::OK)
}

async fn stop(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::Running, RolloutStatus::Paused])?;
    set_status(&state, &identity, id, RolloutStatus::Stopping).await?;
    Ok(StatusCode::OK)
}

/// Force-start the next scheduled group regardless of thresholds.
async fn trigger_next_group(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(&row, &[RolloutStatus::Running])?;
    let started = crate::rollout::start_next_group(&state.pool, &identity.tenant, id).await?;
    if !started {
        return Err(AppError::Conflict(
            "no scheduled group left to start".into(),
        ));
    }
    Ok(StatusCode::OK)
}

/// Retry: new rollout targeting the failed targets of this one.
async fn retry(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<(StatusCode, Json<Value>)> {
    let row = load_rollout(&state, &identity.tenant, id).await?;
    expect_status(
        &row,
        &[
            RolloutStatus::Finished,
            RolloutStatus::Stopped,
            RolloutStatus::Paused,
        ],
    )?;

    let failed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE rollout = $1 AND status = $2")
            .bind(id)
            .bind(ActionStatus::Error.code())
            .fetch_one(&state.pool)
            .await?;
    if failed == 0 {
        return Err(AppError::Conflict(
            "rollout has no failed targets to retry".into(),
        ));
    }

    let name: String = row.get("name");
    let now = now_millis();
    // The retry rollout matches the failed targets by id list snapshot;
    // stored as an RSQL id filter.
    let controller_ids: Vec<String> = sqlx::query_scalar(
        "SELECT t.controller_id FROM sp_action a JOIN sp_target t ON t.id = a.target
         WHERE a.rollout = $1 AND a.status = $2",
    )
    .bind(id)
    .bind(ActionStatus::Error.code())
    .fetch_all(&state.pool)
    .await?;
    let filter = format!(
        "controllerid=in=({})",
        controller_ids
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "")))
            .collect::<Vec<_>>()
            .join(",")
    );

    let retry_name = format!("{name}_retry_{now}");
    let retry_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_rollout
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, description, name, status, distribution_set, target_filter,
          deleted, action_type, weight, is_dynamic)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7, $8, FALSE, $9, $10, FALSE)
         RETURNING id",
    )
    .bind(now)
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(format!("Retry of rollout '{name}'"))
    .bind(&retry_name)
    .bind(RolloutStatus::Creating.code())
    .bind(row.get::<i64, _>("distribution_set"))
    .bind(&filter)
    .bind(row.get::<i32, _>("action_type"))
    .bind(row.get::<i32, _>("weight"))
    .fetch_one(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO sp_rollout_group
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, status, rollout,
          success_condition, success_condition_exp, success_action,
          error_action, target_percentage, is_dynamic)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), 'group-1', $4, $5, $6, '100', $7, $8, 100, FALSE)",
    )
    .bind(now)
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(RolloutGroupStatus::Creating.code())
    .bind(retry_id)
    .bind(ords::SUCCESS_CONDITION_THRESHOLD)
    .bind(ords::SUCCESS_ACTION_NEXTGROUP)
    .bind(ords::ERROR_ACTION_PAUSE)
    .execute(&state.pool)
    .await?;
    sqlx::query("UPDATE sp_rollout SET rollout_groups_created = 1 WHERE id = $1")
        .bind(retry_id)
        .execute(&state.pool)
        .await?;

    let retry_row = load_rollout(&state, &identity.tenant, retry_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(rollout_json(&state, &retry_row, &request_base(&headers)).await?),
    ))
}

// ---------------------------------------------------------------------------
// Deploy groups
// ---------------------------------------------------------------------------

fn group_json(row: &PgRow, base: &str, rollout_id: i64) -> Value {
    let id: i64 = row.get("id");
    let mut value = json!({
        "id": id,
        "name": row.get::<String, _>("name"),
        "description": row.get::<Option<String>, _>("description"),
        "status": group_status_name(row.get::<i32, _>("status")),
        "totalTargets": row.get::<Option<i64>, _>("total_targets").unwrap_or(0),
        "targetPercentage": row.get::<Option<f64>, _>("target_percentage"),
        "targetFilterQuery": row.get::<Option<String>, _>("target_filter"),
        "confirmationRequired": row.get::<Option<bool>, _>("confirmation_required"),
        "successCondition": {
            "condition": "THRESHOLD",
            "expression": row.get::<String, _>("success_condition_exp"),
        },
        "successAction": { "action": "NEXTGROUP", "expression": "" },
        "errorCondition": row.get::<Option<String>, _>("error_condition_exp").map(|exp| json!({
            "condition": "THRESHOLD",
            "expression": exp,
        })),
        "errorAction": { "action": "PAUSE", "expression": "" },
        "_links": {
            "self": {
                "href": format!("{base}/rest/v1/rollouts/{rollout_id}/deploygroups/{id}")
            }
        }
    });
    put_audit(&mut value, row);
    value
}

const GROUP_SELECT: &str = "SELECT g.id, g.created_at, g.created_by, g.last_modified_at,
       g.last_modified_by, g.description, g.name, g.status, g.rollout,
       g.total_targets, g.target_percentage, g.target_filter, g.confirmation_required,
       g.success_condition_exp, g.error_condition_exp, g.is_dynamic
FROM sp_rollout_group g";

async fn list_groups(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_rollout(&state, &identity.tenant, id).await?;
    let base = request_base(&headers);
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_rollout_group WHERE rollout = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query(&format!(
        "{GROUP_SELECT} WHERE g.rollout = $1 ORDER BY g.id LIMIT {} OFFSET {}",
        params.limit()?,
        params.offset()?
    ))
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(paged(
        rows.iter().map(|r| group_json(r, &base, id)).collect(),
        total,
    ))
}

async fn get_group(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, group_id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_rollout(&state, &identity.tenant, id).await?;
    let row = sqlx::query(&format!(
        "{GROUP_SELECT} WHERE g.id = $1 AND g.rollout = $2"
    ))
    .bind(group_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("rollout group {group_id} not found")))?;
    Ok(Json(group_json(&row, &request_base(&headers), id)))
}

async fn group_targets(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, group_id)): Path<(i64, i64)>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_rollout(&state, &identity.tenant, id).await?;
    let base = request_base(&headers);
    // hawkBit declares rsql and sort here too; ignoring them silently left
    // the console unable to page or order a group's devices.
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &GROUP_TARGET_FIELDS, 1)?;
    let order = super::order_by(params.sort.as_deref(), &GROUP_TARGET_FIELDS, "t.id ASC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_target t
             JOIN sp_rollout_target_group l ON l.target = t.id
             WHERE l.rollout_group = $1{filter_sql}"
        ))
        .bind(group_id),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "SELECT t.controller_id, t.name, t.update_status
             FROM sp_target t JOIN sp_rollout_target_group l ON l.target = t.id
             WHERE l.rollout_group = $1{filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(group_id),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;
    let content = rows
        .iter()
        .map(|r| {
            let controller_id: String = r.get("controller_id");
            json!({
                "controllerId": controller_id,
                "name": r.get::<String, _>("name"),
                "updateStatus": super::update_status_name(r.get::<i32, _>("update_status")),
                "_links": { "self": { "href": format!("{base}/rest/v1/targets/{controller_id}") } }
            })
        })
        .collect();
    Ok(paged(content, total))
}

static GROUP_TARGET_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "controllerid",
            Field::Column {
                sql: "t.controller_id",
                kind: Kind::Text,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "t.name",
                kind: Kind::Text,
            },
        ),
        (
            "updatestatus",
            Field::EnumColumn {
                sql: "t.update_status",
                map: &[
                    ("unknown", 0),
                    ("in_sync", 1),
                    ("pending", 2),
                    ("error", 3),
                    ("registered", 4),
                ],
            },
        ),
    ])
});

/// Tenant-config check used by the engine: is the approval workflow on?
pub async fn approval_enabled(pool: &sqlx::PgPool, tenant_name: &str) -> AppResult<bool> {
    tenant::config_bool(
        pool,
        tenant_name,
        tenant::KEY_ROLLOUT_APPROVAL_ENABLED,
        false,
    )
    .await
}
