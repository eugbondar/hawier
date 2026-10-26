//! Shared application state.

use std::sync::Arc;

use sqlx::PgPool;

use crate::artifact::Storage;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    /// General pool: Mgmt API, /ui/v1, background engines.
    pub pool: PgPool,
    /// Dedicated pool for the DDI hot path so mgmt bursts cannot starve
    /// device polling/downloads.
    pub ddi_pool: PgPool,
    pub storage: Storage,
    /// The extensions this node delegates decisions to.
    pub ext: crate::ext::ExtRegistry,
    pub tenant_cache: crate::tenant::TenantConfigCache,
    /// Authorisation verdicts already decided by this node.
    pub authz_cache: crate::authz::VerdictCache,
    /// Per-node snowflake generator for incident numbers.
    pub incidents: Arc<crate::incident::IncidentGen>,
}

impl AppState {
    pub fn new(
        config: Config,
        pool: PgPool,
        ddi_pool: PgPool,
        storage: Storage,
        ext: crate::ext::ExtRegistry,
    ) -> Self {
        let incidents = Arc::new(crate::incident::IncidentGen::new(config.node_id));
        Self {
            config: Arc::new(config),
            pool,
            ddi_pool,
            storage,
            ext,
            tenant_cache: crate::tenant::TenantConfigCache::default(),
            authz_cache: crate::authz::VerdictCache::default(),
            incidents,
        }
    }
}

/// Current time as epoch milliseconds.
///
/// NARROW: the hawkBit schema stores all timestamps as BIGINT epoch millis
/// (no timestamptz); pre-production hawier moves to timestamptz columns.
pub fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as i64
}
