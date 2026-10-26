//! Tenant bootstrap and per-tenant configuration.
//!
//! hawkBit auto-creates tenant metadata (standard SM/DS types + `sp_tenant`
//! row) on first access; hawier mirrors that so the resulting rows look the
//! same to a stock hawkBit.

use sqlx::{PgExecutor, PgPool, Row};

use crate::db::model::SYSTEM_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::now_millis;

// Tenant configuration keys (subset of hawkBit's TenantConfigurationProperties).
pub const KEY_POLLING_TIME: &str = "pollingTime";
pub const KEY_POLLING_OVERDUE_TIME: &str = "pollingOverdueTime";
pub const KEY_MAINTENANCE_WINDOW_POLL_COUNT: &str = "maintenanceWindowPollCount";
pub const KEY_TARGETTOKEN_ENABLED: &str = "authentication.targettoken.enabled";
pub const KEY_GATEWAYTOKEN_ENABLED: &str = "authentication.gatewaytoken.enabled";
pub const KEY_GATEWAYTOKEN_KEY: &str = "authentication.gatewaytoken.key";
pub const KEY_ROLLOUT_APPROVAL_ENABLED: &str = "rollout.approval.enabled";
pub const KEY_ACTIONS_AUTOCLOSE_ENABLED: &str = "repository.actions.autoclose.enabled";
pub const KEY_CONFIRMATION_FLOW_ENABLED: &str = "user.confirmation.flow.enabled";
pub const KEY_MULTI_ASSIGNMENTS_ENABLED: &str = "multi.assignments.enabled";
pub const KEY_BATCH_ASSIGNMENTS_ENABLED: &str = "batch.assignments.enabled";
pub const KEY_IMPLICIT_LOCK_ENABLED: &str = "implicit.lock.enabled";
pub const KEY_ACTION_CLEANUP_EXPIRY: &str = "action.cleanup.auto.expiry";
pub const KEY_ACTION_CLEANUP_STATUS: &str = "action.cleanup.auto.status";
/// hawier addition, part of the hardened profile (API_NOTES §6): a target
/// without a target type receives no assignment at all. hawkBit has no such
/// key — an untyped target there accepts any set.
pub const KEY_TARGET_TYPE_REQUIRED: &str = "target.type.required";

/// Server-side defaults for unset tenant config keys (mirrors hawkBit's
/// property defaults; the two DDI auth toggles get their defaults from
/// hawier env config instead — see `TenantConfig::load`).
pub fn default_value(key: &str) -> Option<&'static str> {
    match key {
        KEY_POLLING_TIME => Some("00:05:00"),
        KEY_POLLING_OVERDUE_TIME => Some("00:05:00"),
        KEY_MAINTENANCE_WINDOW_POLL_COUNT => Some("3"),
        KEY_TARGETTOKEN_ENABLED | KEY_GATEWAYTOKEN_ENABLED => Some("false"),
        KEY_GATEWAYTOKEN_KEY => Some(""),
        KEY_ROLLOUT_APPROVAL_ENABLED
        | KEY_ACTIONS_AUTOCLOSE_ENABLED
        | KEY_CONFIRMATION_FLOW_ENABLED
        | KEY_MULTI_ASSIGNMENTS_ENABLED
        | KEY_BATCH_ASSIGNMENTS_ENABLED
        | KEY_TARGET_TYPE_REQUIRED => Some("false"),
        KEY_IMPLICIT_LOCK_ENABLED => Some("true"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tenant-config cache (DDI hot path: every poll checks auth toggles and
// polling time — reading them from the DB per request caps poll throughput)
// ---------------------------------------------------------------------------

/// Whole-tenant config snapshots with a short TTL. TTL 0 disables caching
/// entirely (used by tests that write `sp_tenant_configuration` directly).
use std::collections::HashMap;

type CacheEntries = HashMap<String, (std::time::Instant, HashMap<String, String>)>;

#[derive(Clone, Default)]
pub struct TenantConfigCache {
    entries: std::sync::Arc<std::sync::RwLock<CacheEntries>>,
}

impl TenantConfigCache {
    fn get(&self, tenant: &str, ttl: std::time::Duration) -> Option<HashMap<String, String>> {
        let entries = self.entries.read().ok()?;
        let (at, values) = entries.get(&tenant.to_uppercase())?;
        (at.elapsed() < ttl).then(|| values.clone())
    }

    fn put(&self, tenant: &str, values: HashMap<String, String>) {
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(tenant.to_uppercase(), (std::time::Instant::now(), values));
        }
    }

    /// Drop a tenant's snapshot (mgmt config writes call this).
    pub fn invalidate(&self, tenant: &str) {
        if let Ok(mut entries) = self.entries.write() {
            entries.remove(&tenant.to_uppercase());
        }
    }
}

/// Cached read of one tenant config value (whole tenant loaded in ONE query
/// on miss). Falls back like [`config_value`].
pub async fn cached_value(
    state: &crate::state::AppState,
    tenant: &str,
    key: &str,
    fallback: Option<&str>,
) -> AppResult<Option<String>> {
    let ttl = std::time::Duration::from_millis(state.config.tenant_cache_ttl_ms);
    if ttl.is_zero() {
        return config_value(&state.ddi_pool, tenant, key, fallback).await;
    }

    let values = match state.tenant_cache.get(tenant, ttl) {
        Some(values) => values,
        None => {
            let rows = sqlx::query(
                "SELECT conf_key, conf_value FROM sp_tenant_configuration
                 WHERE UPPER(tenant) = UPPER($1)",
            )
            .bind(tenant)
            .fetch_all(&state.ddi_pool)
            .await?;
            let values: HashMap<String, String> = rows
                .iter()
                .map(|r| {
                    (
                        r.get::<String, _>("conf_key"),
                        r.get::<String, _>("conf_value"),
                    )
                })
                .collect();
            state.tenant_cache.put(tenant, values.clone());
            values
        }
    };

    Ok(values
        .get(key)
        .cloned()
        .or_else(|| fallback.map(str::to_string))
        .or_else(|| default_value(key).map(str::to_string)))
}

pub async fn cached_bool(
    state: &crate::state::AppState,
    tenant: &str,
    key: &str,
    fallback: bool,
) -> AppResult<bool> {
    let value = cached_value(
        state,
        tenant,
        key,
        Some(if fallback { "true" } else { "false" }),
    )
    .await?;
    Ok(matches!(
        value.as_deref(),
        Some("true") | Some("TRUE") | Some("1")
    ))
}

/// Parse hawkBit's `HH:mm:ss` polling duration into milliseconds.
pub fn parse_polling_millis(value: &str) -> Option<i64> {
    let mut parts = value.split(':');
    let hours: i64 = parts.next()?.trim().parse().ok()?;
    let minutes: i64 = parts.next()?.trim().parse().ok()?;
    let seconds: i64 = parts.next()?.trim().parse().ok()?;
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000)
}

/// Format milliseconds back into hawkBit's `HH:mm:ss` polling duration.
pub fn format_polling_millis(millis: i64) -> String {
    let total_seconds = (millis.max(0) / 1000).min(99 * 3600 + 59 * 60 + 59);
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

/// Read one tenant config value, falling back to `fallback` and then to the
/// server default.
pub async fn config_value<'e>(
    executor: impl PgExecutor<'e>,
    tenant: &str,
    key: &str,
    fallback: Option<&str>,
) -> AppResult<Option<String>> {
    let row = sqlx::query(
        "SELECT conf_value FROM sp_tenant_configuration WHERE tenant = $1 AND conf_key = $2",
    )
    .bind(tenant)
    .bind(key)
    .fetch_optional(executor)
    .await?;
    Ok(row
        .map(|r| r.get::<String, _>("conf_value"))
        .or_else(|| fallback.map(str::to_string))
        .or_else(|| default_value(key).map(str::to_string)))
}

pub async fn config_bool<'e>(
    executor: impl PgExecutor<'e>,
    tenant: &str,
    key: &str,
    fallback: bool,
) -> AppResult<bool> {
    let value = config_value(
        executor,
        tenant,
        key,
        Some(if fallback { "true" } else { "false" }),
    )
    .await?;
    Ok(matches!(
        value.as_deref(),
        Some("true") | Some("TRUE") | Some("1")
    ))
}

/// Ensure the tenant exists, creating hawkBit's standard type set on first
/// access (implicit tenant creation, like hawkBit's monolith default).
/// Returns the `sp_tenant.id`.
pub async fn ensure_tenant(pool: &PgPool, tenant: &str) -> AppResult<i64> {
    if tenant.is_empty() || tenant.len() > 40 {
        return Err(AppError::BadRequest(format!("invalid tenant '{tenant}'")));
    }
    if let Some(row) = sqlx::query("SELECT id FROM sp_tenant WHERE UPPER(tenant) = UPPER($1)")
        .bind(tenant)
        .fetch_optional(pool)
        .await?
    {
        return Ok(row.get::<i64, _>("id"));
    }
    create_tenant(pool, tenant).await
}

async fn create_tenant(pool: &PgPool, tenant: &str) -> AppResult<i64> {
    // hawkBit stores tenants upper-cased (DEFAULT); keep the same convention.
    let tenant = tenant.to_uppercase();
    let now = now_millis();
    let mut tx = pool.begin().await?;

    // Serialize concurrent creation of the same tenant.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(&tenant)
        .execute(&mut *tx)
        .await?;
    if let Some(row) = sqlx::query("SELECT id FROM sp_tenant WHERE UPPER(tenant) = UPPER($1)")
        .bind(&tenant)
        .fetch_optional(&mut *tx)
        .await?
    {
        tx.commit().await?;
        return Ok(row.get::<i64, _>("id"));
    }

    let insert_smt = "INSERT INTO sp_software_module_type
        (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
         tenant, description, name, colour, deleted, type_key, max_ds_assignments, min_artifacts)
        VALUES ($1, $2, $1, $2, 1, $3, $4, $5, NULL, FALSE, $6, $7, 0)
        RETURNING id";
    let app_type: i64 = sqlx::query(insert_smt)
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(&tenant)
        .bind("Application Addons")
        .bind("Application")
        .bind("application")
        .bind(i32::MAX)
        .fetch_one(&mut *tx)
        .await?
        .get("id");
    let os_type: i64 = sqlx::query(insert_smt)
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(&tenant)
        .bind("Core firmware or operating system")
        .bind("OS")
        .bind("os")
        .bind(1)
        .fetch_one(&mut *tx)
        .await?
        .get("id");

    let insert_dst = "INSERT INTO sp_distribution_set_type
        (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
         tenant, description, name, colour, deleted, type_key)
        VALUES ($1, $2, $1, $2, 1, $3, $4, $5, NULL, FALSE, $6)
        RETURNING id";
    let dst_os: i64 = sqlx::query(insert_dst)
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(&tenant)
        .bind("Default type with Firmware/OS only.")
        .bind("OS only")
        .bind("os")
        .fetch_one(&mut *tx)
        .await?
        .get("id");
    let dst_app: i64 = sqlx::query(insert_dst)
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(&tenant)
        .bind("Default type with app(s) only.")
        .bind("App(s) only")
        .bind("app")
        .fetch_one(&mut *tx)
        .await?
        .get("id");
    let dst_os_app: i64 = sqlx::query(insert_dst)
        .bind(now)
        .bind(SYSTEM_PRINCIPAL)
        .bind(&tenant)
        .bind("Default type with Firmware/OS and optional app(s).")
        .bind("OS with app(s)")
        .bind("os_app")
        .fetch_one(&mut *tx)
        .await?
        .get("id");

    let insert_element = "INSERT INTO sp_ds_type_element
        (mandatory, distribution_set_type, software_module_type) VALUES ($1, $2, $3)";
    for (mandatory, dst, smt) in [
        (true, dst_os, os_type),
        (true, dst_app, app_type),
        (true, dst_os_app, os_type),
        (false, dst_os_app, app_type),
    ] {
        sqlx::query(insert_element)
            .bind(mandatory)
            .bind(dst)
            .bind(smt)
            .execute(&mut *tx)
            .await?;
    }

    let tenant_id: i64 = sqlx::query(
        "INSERT INTO sp_tenant
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, default_ds_type)
         VALUES ($1, $2, $1, $2, 1, $3, $4)
         RETURNING id",
    )
    .bind(now)
    .bind(SYSTEM_PRINCIPAL)
    .bind(&tenant)
    .bind(dst_os_app)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    tx.commit().await?;
    tracing::info!(tenant, "created tenant with standard type set");
    Ok(tenant_id)
}
