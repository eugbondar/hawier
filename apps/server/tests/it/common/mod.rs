//! Integration test harness: each test gets its own database (created via
//! the admin connection from `HAWIER_IT_DATABASE_URL`) and an in-process
//! server on an ephemeral port.

// Compiled once per test binary; not every binary uses every helper.
#![allow(dead_code)]

use sqlx::{Connection, PgConnection};

use hawier_server::config::{Config, MgmtUser, Role};
use hawier_server::state::AppState;

pub const IT_ENV: &str = "HAWIER_IT_DATABASE_URL";

pub struct TestServer {
    pub base_url: String,
    pub state: AppState,
    pub db_url: String,
    /// False for a second node sharing another's database.
    owns_database: bool,
    server_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if self.owns_database {
            drop_database(&self.db_url);
        }
    }
}

impl TestServer {
    /// Kill this node's HTTP listener (simulates the node dying). The
    /// database and any other nodes keep running.
    pub async fn kill(&mut self) {
        if let Some(task) = self.server_task.take() {
            task.abort();
            task.await.ok();
        }
    }
}

/// Path to a built extension module.
///
/// Extensions are wasm modules now, so the harness loads real ones — that is
/// the point: the transport is exercised by every integration test rather
/// than mocked. `make it` builds them first.
pub fn ext_module(name: &str) -> String {
    let mut path = std::env::current_exe().expect("test exe path");
    path.pop(); // the executable -> target/<profile>/deps
    path.pop(); // -> target/<profile>
    let profile = path.file_name().expect("profile dir").to_owned();
    path.pop(); // -> target
    path.push("wasm32-wasip1");
    path.push(profile);
    path.push(format!("{name}.wasm"));
    assert!(
        path.exists(),
        "{} not built - run `make ext` (or `make it`, which does)",
        path.display()
    );
    path.to_string_lossy().into_owned()
}

pub fn admin_url() -> String {
    std::env::var(IT_ENV)
        .unwrap_or_else(|_| panic!("{IT_ENV} not set — integration tests run via `make it`"))
}

fn unique_db_name() -> String {
    use rand::Rng;
    let suffix: u64 = rand::thread_rng().gen();
    format!("hawier_it_{suffix:016x}")
}

pub async fn create_database() -> String {
    let admin = admin_url();
    let db_name = unique_db_name();
    let mut conn = PgConnection::connect(&admin)
        .await
        .expect("connect admin db");
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&mut conn)
        .await
        .expect("create test database");
    let (base, _) = admin.rsplit_once('/').expect("db url with path");
    format!("{base}/{db_name}")
}

pub fn test_config(db_url: &str) -> Config {
    let artifact_dir = std::env::temp_dir()
        .join(format!("hawier-it-artifacts-{}", std::process::id()))
        .to_string_lossy()
        .into_owned();
    // The storage extension resolves its root at startup, so it must exist
    // before the first artifact call spawns it.
    std::fs::create_dir_all(&artifact_dir).expect("artifact dir");
    // wasm modules have no argv, so the root travels through the environment
    // allowlist. Every test in this binary shares one artifact dir, so this
    // is the same value each time.
    std::env::set_var("HAWIER_EXT_FS_ROOT", &artifact_dir);
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url: db_url.to_string(),
        role: Role::Control,
        artifact_dir: artifact_dir.clone(),
        ext_storage: ext_module("hawier_ext_fs"),
        ext_event_sink: ext_module("hawier_ext_webhook"),
        ext_retention: ext_module("hawier_ext_retention"),
        ext_rollout: ext_module("hawier_ext_rollout_classic"),
        ext_console: ext_module("hawier_ext_console"),
        ext_authz: ext_module("hawier_ext_authz"),
        ext_params_file: String::new(),
        ext_deadline_ms: 5_000,
        mgmt_users: vec![
            MgmtUser {
                username: "admin".into(),
                password: "admin".into(),
                tenant: "DEFAULT".into(),
            },
            // A second identity, for tests that switch authorisation on.
            // The authz cache is process-wide and keyed by (tenant, user,
            // permission), so a test that gives admin a restricted role would
            // otherwise decide what every concurrent test sees for admin.
            MgmtUser {
                username: "limited".into(),
                password: "limited".into(),
                tenant: "DEFAULT".into(),
            },
        ],
        internal_token: "it-internal-token".into(),
        ddi_targettoken_default: true,
        ddi_gatewaytoken_default: false,
        ddi_anonymous: false,
        // The harness starts postgres before us, so there is nothing to wait for.
        db_wait_secs: 5,
        mgmt_pool_size: 5,
        ddi_pool_size: 5,
        max_connections: 128,
        max_artifact_size: 64 * 1024 * 1024,
        tenant_cache_ttl_ms: 0,
        poll_touch_interval_ms: 0,
        housekeeping_interval_ms: 200,
        node_id: 9,
    }
}

/// Boot a fresh server (own database, migrations applied) and serve it.
pub async fn spawn() -> TestServer {
    spawn_with(|_| {}).await
}

/// Same, with a chance to change the config first — used to test what a node
/// does when an extension is missing or broken.
pub async fn spawn_with(adjust: impl FnOnce(&mut Config)) -> TestServer {
    let db_url = create_database().await;
    spawn_config(db_url, adjust).await
}

/// A second node on an existing node's database, like a sub node beside a
/// control node. It does not own the database, so it must not drop it.
pub async fn spawn_on(other: &TestServer, adjust: impl FnOnce(&mut Config)) -> TestServer {
    let mut server = spawn_config(other.db_url.clone(), adjust).await;
    server.owns_database = false;
    server
}

async fn spawn_config(db_url: String, adjust: impl FnOnce(&mut Config)) -> TestServer {
    let mut config = test_config(&db_url);
    adjust(&mut config);
    let state = hawier_server::init(config).await.expect("init server");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().unwrap();
    let app = hawier_server::app(state.clone());
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    TestServer {
        base_url: format!("http://{addr}"),
        state,
        db_url,
        owns_database: true,
        server_task: Some(server_task),
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// Seeding helpers (mgmt API arrives in Phase 3; until then we seed directly)
// ---------------------------------------------------------------------------

pub struct SeededTarget {
    pub id: i64,
    pub controller_id: String,
    pub token: String,
}

pub async fn seed_target(server: &TestServer, tenant: &str, controller_id: &str) -> SeededTarget {
    hawier_server::tenant::ensure_tenant(&server.state.pool, tenant)
        .await
        .unwrap();
    let token = hawier_server::db::model::generate_sec_token();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_target
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, controller_id, sec_token, request_controller_attributes, update_status)
         VALUES (1, 'test', 1, 'test', 1, UPPER($1), $2, $2, $3, FALSE, 0)
         RETURNING id",
    )
    .bind(tenant)
    .bind(controller_id)
    .bind(&token)
    .fetch_one(&server.state.pool)
    .await
    .unwrap();
    SeededTarget {
        id,
        controller_id: controller_id.to_string(),
        token,
    }
}

pub struct SeededDs {
    pub ds_id: i64,
    pub sm_id: i64,
    pub sha1: String,
    pub md5: String,
    pub filename: String,
    pub content: Vec<u8>,
}

/// Create an "os" software module with one stored artifact and wrap it in a
/// distribution set of type "os".
pub async fn seed_ds_with_artifact(
    server: &TestServer,
    tenant: &str,
    name: &str,
    content: &[u8],
) -> SeededDs {
    hawier_server::tenant::ensure_tenant(&server.state.pool, tenant)
        .await
        .unwrap();
    let pool = &server.state.pool;

    let os_type: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_software_module_type WHERE UPPER(tenant) = UPPER($1) AND type_key = 'os'",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .unwrap();
    let ds_type: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set_type WHERE UPPER(tenant) = UPPER($1) AND type_key = 'os'",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .unwrap();

    let sm_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_software_module
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, version, sm_type, deleted, locked)
         VALUES (1, 'test', 1, 'test', 1, UPPER($1), $2, '1.0.0', $3, FALSE, TRUE)
         RETURNING id",
    )
    .bind(tenant)
    .bind(name)
    .bind(os_type)
    .fetch_one(pool)
    .await
    .unwrap();

    let stored = server
        .state
        .storage
        .store(
            tenant,
            futures::stream::iter(vec![Ok(bytes::Bytes::copy_from_slice(content))]),
        )
        .await
        .unwrap();
    let filename = format!("{name}.bin");
    sqlx::query(
        "INSERT INTO sp_artifact
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, md5_hash, file_size, provided_file_name, sha1_hash, software_module, sha256_hash)
         VALUES (1, 'test', 1, 'test', 1, UPPER($1), $2, $3, $4, $5, $6, $7)",
    )
    .bind(tenant)
    .bind(&stored.md5)
    .bind(stored.size)
    .bind(&filename)
    .bind(&stored.sha1)
    .bind(sm_id)
    .bind(&stored.sha256)
    .execute(pool)
    .await
    .unwrap();

    let ds_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_distribution_set
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, version, deleted, required_migration_step, ds_type, valid, locked)
         VALUES (1, 'test', 1, 'test', 1, UPPER($1), $2, '1.0.0', FALSE, FALSE, $3, TRUE, TRUE)
         RETURNING id",
    )
    .bind(tenant)
    .bind(name)
    .bind(ds_type)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO sp_ds_sm (ds_id, sm_id) VALUES ($1, $2)")
        .bind(ds_id)
        .bind(sm_id)
        .execute(pool)
        .await
        .unwrap();

    SeededDs {
        ds_id,
        sm_id,
        sha1: stored.sha1,
        md5: stored.md5,
        filename,
        content: content.to_vec(),
    }
}

pub async fn set_tenant_config(server: &TestServer, tenant: &str, key: &str, value: &str) {
    sqlx::query(
        "INSERT INTO sp_tenant_configuration
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, conf_key, conf_value)
         VALUES (1, 'test', 1, 'test', 1, UPPER($1), $2, $3)
         ON CONFLICT (conf_key, tenant) DO UPDATE SET conf_value = EXCLUDED.conf_value",
    )
    .bind(tenant)
    .bind(key)
    .bind(value)
    .execute(&server.state.pool)
    .await
    .unwrap();
}

/// Assign a DS to a target through the shared deploy logic (what mgmt will
/// use from Phase 3 on).
pub async fn assign_ds(
    server: &TestServer,
    tenant: &str,
    target_id: i64,
    ds_id: i64,
    action_type: hawier_api::codes::ActionType,
) -> i64 {
    let outcome = hawier_server::deploy_logic::assign(
        &server.state.pool,
        &server.state.incidents,
        &tenant.to_uppercase(),
        &hawier_server::deploy_logic::AssignRequest {
            target_id,
            ds_id,
            action_type,
            forced_time: None,
            weight: hawier_server::deploy_logic::DEFAULT_WEIGHT,
            initiated_by: "test".to_string(),
            confirmation_required: None,
            maintenance_window: None,
        },
    )
    .await
    .unwrap();
    outcome.action_id
}

/// Full-control assignment: custom weight and optional maintenance window
/// (schedule, duration, timezone).
pub async fn assign_ds_full(
    server: &TestServer,
    tenant: &str,
    target_id: i64,
    ds_id: i64,
    action_type: hawier_api::codes::ActionType,
    weight: i32,
    maintenance_window: Option<(&str, &str, &str)>,
) -> i64 {
    let outcome = hawier_server::deploy_logic::assign(
        &server.state.pool,
        &server.state.incidents,
        &tenant.to_uppercase(),
        &hawier_server::deploy_logic::AssignRequest {
            target_id,
            ds_id,
            action_type,
            forced_time: None,
            weight,
            initiated_by: "test".to_string(),
            confirmation_required: None,
            maintenance_window: maintenance_window.map(|(schedule, duration, time_zone)| {
                hawier_server::deploy_logic::MaintenanceWindow {
                    schedule: schedule.to_string(),
                    duration: duration.to_string(),
                    time_zone: time_zone.to_string(),
                }
            }),
        },
    )
    .await
    .unwrap();
    outcome.action_id
}

pub async fn action_statuses(server: &TestServer, action_id: i64) -> Vec<i32> {
    sqlx::query_scalar::<_, i32>(
        "SELECT status FROM sp_action_status WHERE action = $1 ORDER BY id",
    )
    .bind(action_id)
    .fetch_all(&server.state.pool)
    .await
    .unwrap()
}

/// Basic auth header value for the default admin user.
pub fn basic(user: &str, pass: &str) -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
    )
}

/// Each test gets its own database; without this they pile up (925 of them
/// once slowed `make it` to a crawl) and every later run pays for it.
fn drop_database(db_url: &str) {
    let Some((base, name)) = db_url.rsplit_once('/') else {
        return;
    };
    let (admin, name) = (format!("{base}/postgres"), name.to_string());
    // Drop from a plain thread: Drop cannot await, and the pools that still
    // hold this database are closed by FORCE.
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return,
        };
        runtime.block_on(async move {
            if let Ok(mut conn) = sqlx::PgConnection::connect(&admin).await {
                let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                    .execute(&mut conn)
                    .await;
            }
        });
    })
    .join()
    .ok();
}

/// One hardware revision as the hardened profile models it: a distribution
/// set type of its own, and a target type that lists exactly that one.
pub struct Revision {
    pub ds_type: i64,
    pub target_type: i64,
}

/// Create the pair for `rev` through the management API, the way an
/// operator would.
pub async fn seed_revision(server: &TestServer, rev: &str) -> Revision {
    let http = http();
    let auth = basic("admin", "admin");
    let ds_types: serde_json::Value = http
        .post(format!("{}/rest/v1/distributionsettypes", server.base_url))
        .header("Authorization", &auth)
        .json(&serde_json::json!([{ "name": format!("os-{rev}"), "key": format!("os-{rev}") }]))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ds_type = ds_types[0]["id"].as_i64().expect("ds type created");
    let target_types: serde_json::Value = http
        .post(format!("{}/rest/v1/targettypes", server.base_url))
        .header("Authorization", &auth)
        .json(&serde_json::json!([{
            "name": format!("board-{rev}"),
            "key": format!("board-{rev}"),
            "compatibledistributionsettypes": [{ "id": ds_type }],
        }]))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let target_type = target_types[0]["id"].as_i64().expect("target type created");
    Revision {
        ds_type,
        target_type,
    }
}

pub async fn set_target_type(server: &TestServer, target_id: i64, target_type: i64) {
    sqlx::query("UPDATE sp_target SET target_type = $2 WHERE id = $1")
        .bind(target_id)
        .bind(target_type)
        .execute(&server.state.pool)
        .await
        .unwrap();
}

pub async fn set_ds_type(server: &TestServer, ds_id: i64, ds_type: i64) {
    sqlx::query("UPDATE sp_distribution_set SET ds_type = $2 WHERE id = $1")
        .bind(ds_id)
        .bind(ds_type)
        .execute(&server.state.pool)
        .await
        .unwrap();
}

pub async fn action_count(server: &TestServer, target_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sp_action WHERE target = $1")
        .bind(target_id)
        .fetch_one(&server.state.pool)
        .await
        .unwrap()
}
