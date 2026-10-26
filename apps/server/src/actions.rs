//! `sp_action` / `sp_action_status` primitives shared by DDI handlers,
//! mgmt handlers and the rollout engine.

use hawier_api::codes::ActionStatus;
use sqlx::{PgConnection, PgExecutor, Row};

use crate::db::model::TargetUpdateStatus;
use crate::error::{AppError, AppResult};
use crate::state::now_millis;

/// Max status rows per action (hawkBit's default quota).
pub const STATUS_QUOTA_PER_ACTION: i64 = 800;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ActionRow {
    pub id: i64,
    pub tenant: String,
    pub active: Option<bool>,
    pub status: i32,
    pub action_type: i32,
    pub forced_time: Option<i64>,
    pub distribution_set: i64,
    pub target: i64,
    pub rollout: Option<i64>,
    pub rollout_group: Option<i64>,
    pub weight: i32,
    pub initiated_by: String,
    pub external_ref: Option<String>,
    pub maintenance_cron_schedule: Option<String>,
    pub maintenance_duration: Option<String>,
    pub maintenance_time_zone: Option<String>,
    pub last_action_status_code: Option<i32>,
}

impl ActionRow {
    pub fn is_active(&self) -> bool {
        self.active.unwrap_or(false)
    }
}

const ACTION_COLUMNS: &str = "id, tenant, active, status, action_type, forced_time,
    distribution_set, target, rollout, rollout_group, weight, initiated_by,
    external_ref, maintenance_cron_schedule, maintenance_duration,
    maintenance_time_zone, last_action_status_code";

pub async fn load_action<'e>(
    executor: impl PgExecutor<'e>,
    tenant: &str,
    action_id: i64,
) -> AppResult<Option<ActionRow>> {
    Ok(sqlx::query_as::<_, ActionRow>(&format!(
        "SELECT {ACTION_COLUMNS} FROM sp_action WHERE id = $1 AND UPPER(tenant) = UPPER($2)"
    ))
    .bind(action_id)
    .bind(tenant)
    .fetch_optional(executor)
    .await?)
}

/// Same, but takes `FOR UPDATE` inside an open transaction.
pub async fn lock_action(
    tx: &mut PgConnection,
    tenant: &str,
    action_id: i64,
) -> AppResult<Option<ActionRow>> {
    Ok(sqlx::query_as::<_, ActionRow>(&format!(
        "SELECT {ACTION_COLUMNS} FROM sp_action
         WHERE id = $1 AND UPPER(tenant) = UPPER($2) FOR UPDATE"
    ))
    .bind(action_id)
    .bind(tenant)
    .fetch_optional(tx)
    .await?)
}

/// The action the controller sees next: highest weight wins, then oldest
/// id (hawkBit `JpaControllerManagement`: Sort weight DESC, id ASC).
pub async fn next_active_action<'e>(
    executor: impl PgExecutor<'e>,
    target_id: i64,
) -> AppResult<Option<ActionRow>> {
    Ok(sqlx::query_as::<_, ActionRow>(&format!(
        "SELECT {ACTION_COLUMNS} FROM sp_action
         WHERE target = $1 AND active = TRUE ORDER BY weight DESC, id ASC LIMIT 1"
    ))
    .bind(target_id)
    .fetch_optional(executor)
    .await?)
}

/// Insert one `sp_action_status` row plus its detail messages.
/// Enforces the per-action status quota (hawkBit default 800).
///
/// A controller repeating the status it already reported does not get a new
/// row: see [`repeat_of_last_status`].
#[allow(clippy::too_many_arguments)]
pub async fn insert_status(
    tx: &mut PgConnection,
    tenant: &str,
    action_id: i64,
    status: ActionStatus,
    code: Option<i32>,
    occurred_at: Option<i64>,
    created_by: &str,
    messages: &[String],
) -> AppResult<i64> {
    // The quota guards against device feedback spam, so it applies to the
    // controller path only (hawkBit: assertActionStatusQuota lives in
    // JpaControllerManagement) — the server must always be able to write
    // its own statuses, e.g. to force-quit a spammed action.
    if created_by == crate::db::model::CONTROLLER_PRINCIPAL {
        if let Some(existing) =
            repeat_of_last_status(&mut *tx, action_id, status, code, messages).await?
        {
            return Ok(existing);
        }

        let count: i64 =
            sqlx::query("SELECT COUNT(*) AS c FROM sp_action_status WHERE action = $1")
                .bind(action_id)
                .fetch_one(&mut *tx)
                .await?
                .get("c");
        if count >= STATUS_QUOTA_PER_ACTION {
            return Err(AppError::Quota(format!(
                "quota of {STATUS_QUOTA_PER_ACTION} status entries per action exceeded"
            )));
        }
    }

    let now = now_millis();
    let status_id: i64 = sqlx::query(
        "INSERT INTO sp_action_status
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, target_occurred_at, status, action, code)
         VALUES ($1, $2, $1, $2, 1, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(now)
    .bind(created_by)
    .bind(tenant)
    .bind(occurred_at.unwrap_or(now))
    .bind(status.code())
    .bind(action_id)
    .bind(code)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    for message in messages {
        // NARROW: sp_action_status_messages has no PK and no order column;
        // VARCHAR(512) forces truncation. See SCHEMA_NOTES.md #5.
        let truncated = truncate_message(message);
        sqlx::query(
            "INSERT INTO sp_action_status_messages (action_status, detail_message)
             VALUES ($1, $2)",
        )
        .bind(status_id)
        .bind(truncated)
        .execute(&mut *tx)
        .await?;
    }

    if code.is_some() {
        sqlx::query("UPDATE sp_action SET last_action_status_code = $1 WHERE id = $2")
            .bind(code)
            .bind(action_id)
            .execute(&mut *tx)
            .await?;
    }

    Ok(status_id)
}

/// If this controller feedback repeats the action's current last status
/// verbatim, count the repeat on the existing row and return its id instead
/// of inserting a duplicate.
///
/// Why: a device reports `Proceeding` before it starts downloading, and if
/// the download does not fit into one cycle it reports it again on the next
/// one. Under load that closes an amplification loop — saturation makes the
/// download miss the window, the miss produces another report, the report is
/// another row, the row is more work. A single benchmark run wrote 2.7M rows
/// this way and pushed every action into the 800-row quota (`bench/NOTES.md`
/// §5). Nothing was learned from rows 2..800: they are the same state.
///
/// Verbatim means same status, same code and the same set of messages. A
/// device that adds a message ("retrying, 3rd attempt") is telling us
/// something new and still gets its own row.
///
/// NARROW: the repeat counter rides in `optlock_revision`, which is exactly
/// what that column counts (row modifications) but is a legacy JPA field we
/// cannot rename. hawier's own schema revision should give `sp_action_status`
/// a named `repeats` column. See SCHEMA_NOTES.md #20.
async fn repeat_of_last_status(
    tx: &mut PgConnection,
    action_id: i64,
    status: ActionStatus,
    code: Option<i32>,
    messages: &[String],
) -> AppResult<Option<i64>> {
    // The row is locked because the counter is read-modify-write. The
    // deployment feedback path already holds the action row (`lock_action`),
    // and the lock order is the same everywhere — action, then status.
    let Some(last) = sqlx::query(
        "SELECT id, status, code FROM sp_action_status
         WHERE action = $1 ORDER BY id DESC LIMIT 1 FOR UPDATE",
    )
    .bind(action_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };

    if last.get::<i32, _>("status") != status.code() || last.get::<Option<i32>, _>("code") != code {
        return Ok(None);
    }

    let status_id: i64 = last.get("id");
    // NARROW: sp_action_status_messages has no order column (SCHEMA_NOTES #5),
    // so the comparison is over sorted multisets, not sequences.
    let mut previous: Vec<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT detail_message FROM sp_action_status_messages WHERE action_status = $1",
    )
    .bind(status_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .flatten()
    .collect();
    let mut incoming: Vec<String> = messages.iter().map(|m| truncate_message(m)).collect();
    previous.sort_unstable();
    incoming.sort_unstable();
    if previous != incoming {
        return Ok(None);
    }

    sqlx::query(
        "UPDATE sp_action_status
         SET last_modified_at = $1, last_modified_by = $2,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $3",
    )
    .bind(now_millis())
    .bind(crate::db::model::CONTROLLER_PRINCIPAL)
    .bind(status_id)
    .execute(&mut *tx)
    .await?;

    Ok(Some(status_id))
}

/// NARROW: `sp_action_status_messages.detail_message` is VARCHAR(512).
/// See SCHEMA_NOTES.md #5.
fn truncate_message(message: &str) -> String {
    message.chars().take(512).collect()
}

/// Update the action's own status column (+ audit/optlock bump).
pub async fn set_action_status(
    tx: &mut PgConnection,
    action_id: i64,
    status: ActionStatus,
    active: bool,
    modified_by: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE sp_action
         SET status = $1, active = $2, last_modified_at = $3, last_modified_by = $4,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $5",
    )
    .bind(status.code())
    .bind(active)
    .bind(now_millis())
    .bind(modified_by)
    .bind(action_id)
    .execute(tx)
    .await?;
    Ok(())
}

/// Recompute the target's `update_status` after an action closed.
pub async fn set_target_update_status(
    tx: &mut PgConnection,
    target_id: i64,
    status: TargetUpdateStatus,
    modified_by: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE sp_target
         SET update_status = $1, last_modified_at = $2, last_modified_by = $3,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $4",
    )
    .bind(status.code())
    .bind(now_millis())
    .bind(modified_by)
    .bind(target_id)
    .execute(tx)
    .await?;
    Ok(())
}

pub async fn has_other_active_actions(
    tx: &mut PgConnection,
    target_id: i64,
    except_action: i64,
) -> AppResult<bool> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM sp_action
         WHERE target = $1 AND active = TRUE AND id <> $2) AS e",
    )
    .bind(target_id)
    .bind(except_action)
    .fetch_one(tx)
    .await?;
    Ok(row.get::<bool, _>("e"))
}
