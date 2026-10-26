//! Migration runner.
//!
//! Two tracks:
//! 1. hawkBit compatibility track — applies the verbatim hawkBit SQL and
//!    records Flyway-shaped rows in `schema_version` (see [`crate::db::flyway`]).
//!    TEMPORARY dev-stage decision: keeps a stock hawkBit bootable against
//!    the same database.
//! 2. hawier track — hawier's own tables (`hawier_*` prefix), recorded in
//!    `hawier_schema_version`.

use std::time::{Duration, Instant};

use anyhow::Context;
use sqlx::{Connection, PgConnection, Row};
use tracing::info;

use super::flyway::{self, Migration, HAWKBIT_MIGRATIONS, HISTORY_TABLE_DDL};

/// hawier's own migrations (never touch `sp_*` tables here).
///
/// Development stage: this track is kept as ONE file that is edited in
/// place. There is no earlier hawier release to upgrade from, so a schema
/// change means recreating the database, not an ALTER chain. The hawkBit
/// track above is the opposite — it must stay byte-exact.
const HAWIER_MIGRATIONS: &[(i32, &str, &str)] = &[(
    1,
    "0001_hawier",
    include_str!("../../migrations/hawier/0001_hawier.sql"),
)];

/// Advisory lock key so concurrent nodes don't race migrations.
const MIGRATE_LOCK_KEY: i64 = 0x6861_7769_6572_0001; // "hawier\x00\x01"

/// Connect, allowing for a database that is still starting.
///
/// On a single host Postgres usually is starting alongside the node, and after
/// a reboot nothing decides the order at all. A node that exits because its
/// database was two seconds late needs something outside itself to bring it
/// back — a restart policy — and not every engine's compose implements one:
/// under podman-compose the container stayed `Exited (1)` and the deploy waited
/// for a node that was never coming. Waiting here costs nothing when the
/// database is already up and removes the dependency entirely.
async fn connect_waiting(database_url: &str, wait: Duration) -> anyhow::Result<PgConnection> {
    let started = Instant::now();
    let mut said_so = false;
    loop {
        match PgConnection::connect(database_url).await {
            Ok(conn) => {
                if said_so {
                    info!("database reachable after {:?}", started.elapsed());
                }
                return Ok(conn);
            }
            Err(err) => {
                if started.elapsed() >= wait {
                    return Err(err).context(format!(
                        "connecting for migration, after waiting {}s                          (HAWIER_DB_WAIT_SECS)",
                        wait.as_secs()
                    ));
                }
                if !said_so {
                    info!(
                        "database not reachable yet, retrying for up to {}s",
                        wait.as_secs()
                    );
                    said_so = true;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

pub async fn run(database_url: &str, wait: Duration) -> anyhow::Result<()> {
    let mut conn = connect_waiting(database_url, wait).await?;

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATE_LOCK_KEY)
        .execute(&mut conn)
        .await?;

    let result = run_locked(&mut conn).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATE_LOCK_KEY)
        .execute(&mut conn)
        .await
        .ok();

    result
}

async fn run_locked(conn: &mut PgConnection) -> anyhow::Result<()> {
    run_hawkbit_track(conn).await?;
    run_hawier_track(conn).await?;
    Ok(())
}

async fn table_exists(conn: &mut PgConnection, name: &str) -> anyhow::Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables
         WHERE table_schema = current_schema() AND table_name = $1) AS e",
    )
    .bind(name)
    .fetch_one(conn)
    .await?;
    Ok(row.get::<bool, _>("e"))
}

async fn run_hawkbit_track(conn: &mut PgConnection) -> anyhow::Result<()> {
    if !table_exists(conn, "schema_version").await? {
        info!("fresh database: creating Flyway history table `schema_version`");
        sqlx::raw_sql(HISTORY_TABLE_DDL).execute(&mut *conn).await?;
    }

    for migration in HAWKBIT_MIGRATIONS {
        let existing = sqlx::query(
            r#"SELECT "checksum", "success" FROM "schema_version" WHERE "version" = $1"#,
        )
        .bind(migration.version)
        .fetch_optional(&mut *conn)
        .await?;

        match existing {
            Some(row) => {
                anyhow::ensure!(
                    row.get::<bool, _>("success"),
                    "hawkbit migration {} recorded as failed; repair the database first",
                    migration.version
                );
                let recorded: Option<i32> = row.get("checksum");
                let computed = flyway::checksum(migration.sql);
                if recorded != Some(computed) {
                    // A stock hawkBit wrote this row: trust it, but surface
                    // the drift — it means our embedded SQL differs from the
                    // one the DB was migrated with.
                    tracing::warn!(
                        version = migration.version,
                        recorded = ?recorded,
                        computed,
                        "flyway checksum mismatch: embedded hawkbit SQL differs from applied migration"
                    );
                }
            }
            None => apply_hawkbit_migration(conn, migration).await?,
        }
    }
    Ok(())
}

async fn apply_hawkbit_migration(
    conn: &mut PgConnection,
    migration: &Migration,
) -> anyhow::Result<()> {
    info!(
        version = migration.version,
        script = migration.script,
        "applying hawkbit migration"
    );
    let started = std::time::Instant::now();
    let mut tx = conn.begin().await?;

    sqlx::raw_sql(migration.sql)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("applying {}", migration.script))?;

    sqlx::query(
        r#"INSERT INTO "schema_version"
           ("installed_rank", "version", "description", "type", "script",
            "checksum", "installed_by", "installed_on", "execution_time", "success")
           VALUES (
             (SELECT COALESCE(MAX("installed_rank"), 0) + 1 FROM "schema_version"),
             $1, $2, $3, $4, $5, current_user, now(), $6, TRUE)"#,
    )
    .bind(migration.version)
    .bind(migration.description)
    .bind(migration.kind)
    .bind(migration.script)
    .bind(flyway::checksum(migration.sql))
    .bind(started.elapsed().as_millis() as i32)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn run_hawier_track(conn: &mut PgConnection) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS hawier_schema_version (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(&mut *conn)
    .await?;

    for (version, name, sql) in HAWIER_MIGRATIONS {
        let applied = sqlx::query("SELECT 1 AS one FROM hawier_schema_version WHERE version = $1")
            .bind(version)
            .fetch_optional(&mut *conn)
            .await?
            .is_some();
        if applied {
            continue;
        }
        info!(name, "applying hawier migration");
        let mut tx = conn.begin().await?;
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("applying hawier migration {name}"))?;
        sqlx::query("INSERT INTO hawier_schema_version (version, name) VALUES ($1, $2)")
            .bind(version)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}
