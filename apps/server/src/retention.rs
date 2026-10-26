//! Retention rules: delete on a schedule, but only what an extension says.
//!
//! Three parties, deliberately: the rule narrows the field (RSQL plus a
//! minimum age), the extension decides which of those may go, and the host
//! does the deleting. None of them can do another's job — the SQL cannot
//! express "keep the newest three", the extension cannot reach a file, and
//! the host has no policy.
//!
//! What keeps this safe when an extension is wrong:
//!
//! - only ids the host itself offered are honoured, because a descriptor is
//!   data and not authority, and this descriptor deletes things;
//! - the blob goes only when no other `sp_artifact` row references that
//!   sha1, exactly as a manual delete does (content-addressed sharing);
//! - `dry_run` is the default, so a rule says what it would do until someone
//!   decides it may.
//!
//! Runs on the control node's housekeeping tick. A rule is claimed with
//! `FOR UPDATE SKIP LOCKED`, so several control nodes never run one twice.

use chrono::{DateTime, Utc};
use hawier_api::ext::{kinds, points, RetentionCandidate, RetentionPlan, RetentionRequest};
use serde_json::Value;
use sqlx::Row;

use std::sync::LazyLock;

use crate::error::{AppError, AppResult};
use crate::ext;
use crate::rsql::{Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

/// What a rule's filter may narrow by. Deliberately about the artifact and
/// its module: a rule says *which* things are in scope, and the extension
/// says which of those may go.
static CANDIDATE_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "filename",
            Field::Column {
                sql: "a.provided_file_name",
                kind: Kind::Text,
            },
        ),
        (
            "size",
            Field::Column {
                sql: "a.file_size",
                kind: Kind::Int,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "a.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "module.name",
            Field::Column {
                sql: "m.name",
                kind: Kind::Text,
            },
        ),
        (
            "module.version",
            Field::Column {
                sql: "m.version",
                kind: Kind::Text,
            },
        ),
    ])
});

/// The fields a retention rule's filter may use, for validation on write.
pub fn candidate_fields() -> &'static FieldMap {
    &CANDIDATE_FIELDS
}

/// How many candidates one rule may consider per run.
///
/// A cap rather than a stream: the whole point is that the extension sees the
/// set at once (it decides "the newest N of these"), and a run that would
/// consider more than this is a rule that needs a narrower filter.
const MAX_CANDIDATES: i64 = 5_000;

pub async fn tick(state: &AppState) -> AppResult<()> {
    let rules = sqlx::query(
        "SELECT id, tenant, name, scope, filter_rsql, min_age_ms, cron, params, dry_run,
                (EXTRACT(EPOCH FROM last_run_at) * 1000)::BIGINT AS last_run_ms
         FROM hawier_retention_rule WHERE active = TRUE ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;

    for rule in rules {
        let id: i64 = rule.get("id");
        if !is_due(&rule)? {
            continue;
        }
        if let Err(error) = run_rule(state, id).await {
            // A broken rule must not stop the others, and must be visible.
            tracing::warn!(rule = id, %error, "retention rule failed");
            record(state, id, &format!("failed: {error}")).await.ok();
        }
    }
    Ok(())
}

/// Due when the schedule has fired since the last run. A rule that has never
/// run waits for its first scheduled time rather than firing immediately —
/// creating a deletion rule should not delete anything that same second.
fn is_due(rule: &sqlx::postgres::PgRow) -> AppResult<bool> {
    let schedule = crate::maintenance::parse_schedule(rule.get::<String, _>("cron").as_str())?;
    let last: Option<i64> = rule.get("last_run_ms");
    let since = match last {
        Some(millis) => DateTime::<Utc>::from_timestamp_millis(millis),
        None => DateTime::<Utc>::from_timestamp_millis(rule_created_fallback()),
    }
    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("unusable rule timestamp")))?;
    Ok(schedule
        .after(&since)
        .next()
        .is_some_and(|next| next <= Utc::now()))
}

/// A rule with no recorded run is measured from now, so its first firing is
/// its first scheduled time.
fn rule_created_fallback() -> i64 {
    now_millis()
}

async fn run_rule(state: &AppState, id: i64) -> AppResult<()> {
    let mut tx = state.pool.begin().await?;
    // Claim it; another control node may be looking at the same row.
    let Some(rule) = sqlx::query(
        "SELECT id, tenant, name, scope, filter_rsql, min_age_ms, params, dry_run
         FROM hawier_retention_rule WHERE id = $1 AND active = TRUE
         FOR UPDATE SKIP LOCKED",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };
    let tenant: String = rule.get("tenant");
    let name: String = rule.get("name");
    let scope: String = rule.get("scope");
    let dry_run: bool = rule.get("dry_run");
    let params: Value = rule.get("params");
    let filter: String = rule.get("filter_rsql");
    let min_age_ms: i64 = rule.get("min_age_ms");
    tx.commit().await?;

    if scope != "artifact" {
        return Err(AppError::BadRequest(format!(
            "rule '{name}' has scope '{scope}', which nothing implements"
        )));
    }

    let candidates = artifact_candidates(state, &tenant, &filter, min_age_ms).await?;
    if candidates.is_empty() {
        record(state, id, "nothing eligible").await?;
        return Ok(());
    }

    let extension =
        state.ext.retention.as_deref().ok_or_else(|| {
            AppError::Unavailable("no retention extension configured".to_string())
        })?;
    let plan: RetentionPlan = ext::call(
        extension,
        points::RETENTION_SELECT,
        &RetentionRequest {
            scope,
            rule: name.clone(),
            params,
            now: now_millis(),
            candidates: candidates.clone(),
        },
    )
    .await?;

    // Only what we offered. An extension naming anything else is a bug or
    // worse, and either way it does not get to delete by naming.
    let offered: std::collections::HashSet<&str> =
        candidates.iter().map(|c| c.id.as_str()).collect();
    let mut chosen = Vec::new();
    for wanted in &plan.delete {
        match candidates.iter().find(|c| &c.id == wanted) {
            Some(candidate) if offered.contains(wanted.as_str()) => chosen.push(candidate),
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "retention extension named artifact {wanted}, which it was not offered"
                )))
            }
        }
    }

    let freed: i64 = chosen.iter().map(|c| c.size).sum();
    if dry_run {
        record(
            state,
            id,
            &format!(
                "dry run: would delete {} of {} candidates ({} bytes) — {}",
                chosen.len(),
                candidates.len(),
                freed,
                plan.reason
            ),
        )
        .await?;
        return Ok(());
    }

    let mut deleted = 0usize;
    for candidate in chosen {
        if delete_artifact(state, &tenant, candidate, &name).await? {
            deleted += 1;
        }
    }
    record(
        state,
        id,
        &format!(
            "deleted {deleted} of {} candidates ({freed} bytes) — {}",
            candidates.len(),
            plan.reason
        ),
    )
    .await?;
    Ok(())
}

/// Delete one artifact the way the mgmt endpoint does: the row goes inside a
/// transaction with its event, and the blob only afterwards, and only when
/// nothing else references that content.
async fn delete_artifact(
    state: &AppState,
    tenant: &str,
    candidate: &RetentionCandidate,
    rule: &str,
) -> AppResult<bool> {
    let artifact_id: i64 = candidate
        .id
        .parse()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("unusable artifact id")))?;

    let mut tx = state.pool.begin().await?;
    let removed =
        sqlx::query("DELETE FROM sp_artifact WHERE id = $1 AND UPPER(tenant) = UPPER($2)")
            .bind(artifact_id)
            .bind(tenant)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed == 0 {
        // Something else deleted it between the snapshot and now.
        tx.rollback().await?;
        return Ok(false);
    }

    let referenced: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_artifact WHERE sha1_hash = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(&candidate.sha1)
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await?;

    crate::events::emit(
        &mut tx,
        &state.incidents,
        tenant,
        kinds::ARTIFACT_DELETED,
        serde_json::json!({
            "artifactId": artifact_id,
            "softwareModuleId": candidate.software_module,
            "sha1": candidate.sha1,
            "blobRemoved": referenced == 0,
            "deletedBy": format!("retention:{rule}"),
        }),
    )
    .await?;
    tx.commit().await?;

    if referenced == 0 {
        state.storage.delete(tenant, &candidate.sha1).await.ok();
    }
    Ok(true)
}

/// Everything the rule allows the extension to consider.
///
/// The two flags are the questions a policy asks that RSQL cannot: is this
/// module still offered by a distribution set, and was it ever actually sent
/// to a device.
async fn artifact_candidates(
    state: &AppState,
    tenant: &str,
    filter: &str,
    min_age_ms: i64,
) -> AppResult<Vec<RetentionCandidate>> {
    // $1 tenant, $2 the age cutoff — the filter's binds follow those.
    let (filter_sql, binds) = crate::mgmt::rsql_filter(
        Some(filter).filter(|f| !f.trim().is_empty()),
        &CANDIDATE_FIELDS,
        2,
    )?;

    let rows = crate::rsql::bind_values(
        sqlx::query(&format!(
            "SELECT a.id, a.sha1_hash, a.provided_file_name, a.file_size, a.created_at,
                    m.id AS module_id, m.name AS module_name, m.version AS module_version,
                    EXISTS (SELECT 1 FROM sp_ds_sm l WHERE l.sm_id = m.id) AS in_ds,
                    EXISTS (
                        SELECT 1 FROM sp_ds_sm l
                        JOIN sp_action act ON act.distribution_set = l.ds_id
                        WHERE l.sm_id = m.id
                    ) AS ever_assigned
             FROM sp_artifact a
             JOIN sp_software_module m ON m.id = a.software_module
             WHERE UPPER(a.tenant) = UPPER($1) AND a.created_at <= $2{filter_sql}
             ORDER BY a.id LIMIT {MAX_CANDIDATES}"
        ))
        .bind(tenant)
        .bind(now_millis().saturating_sub(min_age_ms)),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| RetentionCandidate {
            id: row.get::<i64, _>("id").to_string(),
            sha1: row.get("sha1_hash"),
            filename: row.get("provided_file_name"),
            size: row.get::<Option<i64>, _>("file_size").unwrap_or(0),
            created_at: row.get::<Option<i64>, _>("created_at").unwrap_or(0),
            software_module: row.get::<i64, _>("module_id").to_string(),
            module_name: row.get("module_name"),
            module_version: row.get("module_version"),
            in_distribution_set: row.get("in_ds"),
            ever_assigned: row.get("ever_assigned"),
        })
        .collect())
}

async fn record(state: &AppState, id: i64, result: &str) -> AppResult<()> {
    sqlx::query(
        "UPDATE hawier_retention_rule SET last_run_at = now(), last_result = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(result)
    .execute(&state.pool)
    .await?;
    Ok(())
}
