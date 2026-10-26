//! Database access: pools, migrations, row models.

pub mod flyway;
pub mod migrate;
pub mod model;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;

/// Build the two pools: `pool` for Mgmt/UI/background work, `ddi_pool` for
/// the device-facing hot path (isolated so mgmt bursts can't starve DDI).
pub async fn connect(config: &Config) -> anyhow::Result<(PgPool, PgPool)> {
    let pool = PgPoolOptions::new()
        .max_connections(config.mgmt_pool_size)
        .connect(&config.database_url)
        .await?;
    let ddi_pool = PgPoolOptions::new()
        .max_connections(config.ddi_pool_size)
        .connect(&config.database_url)
        .await?;
    Ok((pool, ddi_pool))
}
