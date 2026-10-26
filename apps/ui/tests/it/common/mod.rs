//! Shared harness for the console test suites: hawier-server and
//! hawier-ui both in-process, driven over HTTP like a browser (forms +
//! session cookie). Needs HAWIER_IT_DATABASE_URL (`make it`).

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use sqlx::Connection;

pub struct Stack {
    /// Dropped with the stack (see `drop_database`).
    pub db_url: String,
    pub ui_url: String,
    /// hawier-server behind this console (mgmt, DDI and /ui/v1).
    pub server_url: String,
    pub server_state: hawier_server::state::AppState,
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

pub async fn spawn_stack() -> Stack {
    spawn_stack_with(|_| {}).await
}

/// The same stack with the node's configuration adjusted — for the handful of
/// audits that need a node configured differently from the default one.
pub async fn spawn_stack_with(adjust: impl FnOnce(&mut hawier_server::config::Config)) -> Stack {
    let admin = std::env::var("HAWIER_IT_DATABASE_URL")
        .expect("HAWIER_IT_DATABASE_URL not set — integration tests run via `make it`");
    let db_name = format!(
        "hawier_it_ui_{}",
        &hawier_server::db::model::generate_sec_token()[..16]
    );
    let mut conn = sqlx::PgConnection::connect(&admin)
        .await
        .expect("connect admin db");
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&mut conn)
        .await
        .expect("create test database");
    let (base, _) = admin.rsplit_once('/').expect("db url with path");
    let db_url = format!("{base}/{db_name}");

    let artifact_dir = std::env::temp_dir()
        .join(format!("hawier-it-ui-artifacts-{}", std::process::id()))
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(&artifact_dir).expect("artifact dir");
    // wasm modules have no argv, so the root travels through the environment
    // allowlist. Every test in this binary shares one artifact dir, so this
    // is the same value each time.
    std::env::set_var("HAWIER_EXT_FS_ROOT", &artifact_dir);

    let server_config = hawier_server::config::Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url: db_url.clone(),
        role: hawier_server::config::Role::Control,
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
            hawier_server::config::MgmtUser {
                username: "admin".into(),
                password: "admin".into(),
                tenant: "DEFAULT".into(),
            },
            // A second identity, for tests that switch authorisation on.
            // The authz cache is process-wide and keyed by (tenant, user,
            // permission), so a test that gives admin a restricted role would
            // otherwise decide what every concurrent test sees for admin.
            hawier_server::config::MgmtUser {
                username: "limited".into(),
                password: "limited".into(),
                tenant: "DEFAULT".into(),
            },
        ],
        internal_token: "it-ui-internal-token".into(),
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
        housekeeping_interval_ms: 60_000,
        node_id: 9,
    };
    let mut server_config = server_config;
    adjust(&mut server_config);
    let server_state = hawier_server::init(server_config)
        .await
        .expect("init server");
    hawier_server::tenant::ensure_tenant(&server_state.pool, "DEFAULT")
        .await
        .expect("bootstrap tenant");
    let server_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let server_addr = server_listener.local_addr().unwrap();
    let server_app = hawier_server::app(server_state.clone());
    tokio::spawn(async move {
        axum::serve(server_listener, server_app).await.ok();
    });

    let ui_config = hawier_ui::config::Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        hawkbit_mgmt_url: format!("http://{server_addr}"),
        hawier_api_url: format!("http://{server_addr}"),
        internal_token: "it-ui-internal-token".into(),
        // Repo root: tests run with CWD = apps/ui.
        static_dir: std::path::PathBuf::from("../../static"),
        dev_mode: false,
    };
    let ui_state = hawier_ui::state::AppState::new(ui_config, reqwest::Client::new());
    // The audit crawls whatever is in the tree, so it has to see what
    // extensions declared — otherwise a declared surface is the one page
    // nobody checks.
    match hawier_ui::ui_api::fetch_surfaces(&ui_state).await {
        Ok(declaration) => hawier_ui::resources::adopt(declaration),
        Err(error) => panic!("console surfaces must be readable in tests: {error:#}"),
    }
    let ui_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ui");
    let ui_addr = ui_listener.local_addr().unwrap();
    let ui_app = hawier_ui::routes::build_router(ui_state);
    tokio::spawn(async move {
        axum::serve(ui_listener, ui_app).await.ok();
    });

    Stack {
        db_url,
        ui_url: format!("http://{ui_addr}"),
        server_url: format!("http://{server_addr}"),
        server_state,
    }
}

/// Browser-like client: no auto-redirects (we assert on 303s), manual cookie.
pub fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

pub async fn login(stack: &Stack, http: &reqwest::Client) -> String {
    login_as(stack, http, "admin").await
}

/// Log in as one of the harness's configured users. Both have the password
/// equal to their name.
pub async fn login_as(stack: &Stack, http: &reqwest::Client, username: &str) -> String {
    let response = http
        .post(format!("{}/login", stack.ui_url))
        .form(&[
            ("username", username),
            ("password", username),
            ("tenant", "DEFAULT"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303, "login must redirect");
    let cookie = response
        .headers()
        .get("set-cookie")
        .expect("login sets the session cookie")
        .to_str()
        .unwrap();
    cookie.split(';').next().unwrap().to_string()
}

pub async fn get_html(stack: &Stack, http: &reqwest::Client, cookie: &str, path: &str) -> String {
    let response = http
        .get(format!("{}{path}", stack.ui_url))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "GET {path} -> {}",
        response.status()
    );
    response.text().await.unwrap()
}

pub async fn post_form(
    stack: &Stack,
    http: &reqwest::Client,
    cookie: &str,
    path: &str,
    form: &[(&str, &str)],
) -> reqwest::Response {
    http.post(format!("{}{path}", stack.ui_url))
        .header("Cookie", cookie)
        .form(form)
        .send()
        .await
        .unwrap()
}

impl Drop for Stack {
    fn drop(&mut self) {
        drop_database(&self.db_url);
    }
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
