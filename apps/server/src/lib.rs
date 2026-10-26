//! hawier-server: Rust rewrite of Eclipse hawkBit (Mgmt + DDI APIs).
//!
//! COMPATIBILITY NOTE (temporary, dev stage): the database schema is kept
//! byte-compatible with hawkBit 1.0.0 (Flyway baseline B1_20_0 + V1_20_1 +
//! V1_20_2) so a stock hawkBit can still run against the same PostgreSQL
//! database. This is a deliberate, temporary decision for the development
//! stage. Known schema narrow places are catalogued in SCHEMA_NOTES.md and
//! marked `// NARROW:` at their touchpoints in code; they will be fixed in
//! pre-production hawier schema revisions.

pub mod actions;
pub mod artifact;
pub mod auth;
pub mod authz;
pub mod config;
pub mod db;
pub mod ddi;
pub mod deploy_logic;
pub mod error;
pub mod error_capture;
pub mod events;
pub mod ext;
pub mod housekeeping;
pub mod incident;
pub mod maintenance;
pub mod mgmt;
pub mod outbox;
pub mod retention;
pub mod rollout;
pub mod rollout_strategy;
pub mod rsql;
pub mod state;
pub mod tenant;
pub mod ui_state;

use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::config::Role;
use crate::state::AppState;

/// Build the full router for the given state, respecting the node role.
///
/// Sub nodes serve the complete DDI surface plus read-only Mgmt: devices
/// keep updating when the control node is down; mutating mgmt calls belong
/// on the control node.
pub fn app(state: AppState) -> Router {
    // Mgmt carries multipart artifact uploads — lift axum's default 2 MB
    // body cap to the configured artifact limit (authenticated surface only).
    let body_limit = axum::extract::DefaultBodyLimit::max(state.config.max_artifact_size);
    let mgmt_router = match state.config.role {
        Role::Control => mgmt::router().layer(body_limit),
        Role::Sub => mgmt::router()
            .layer(middleware::from_fn(reject_writes_on_sub))
            .layer(body_limit),
    }
    // One layer rather than a guard in every handler: the check that gets
    // forgotten is the hole nobody sees (authz::permission_for).
    .layer(middleware::from_fn_with_state(
        state.clone(),
        authz::enforce,
    ));

    let max_in_flight = state.config.max_connections;
    Router::new()
        .route("/health", get(health))
        .merge(mgmt_router)
        .merge(ddi::router())
        .merge(ui_state::router())
        .layer(TraceLayer::new_for_http())
        // `incident` in every error body + journal (error_capture.rs); the
        // incident-id layer sits outside so the middleware sees the header.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            error_capture::error_capture,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            incident::assign_incident_id,
        ))
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(
            max_in_flight,
        ))
        .with_state(state)
}

async fn reject_writes_on_sub(request: Request, next: Next) -> Response {
    let read_only = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if read_only {
        next.run(request).await
    } else {
        let body = axum::Json(serde_json::json!({
            "exceptionClass": "org.springframework.security.access.AccessDeniedException",
            "errorCode": "hawkbit.server.error.readOnlyNode",
            "message": "this is a sub node: mutating Mgmt API calls go to the control node",
        }));
        (StatusCode::METHOD_NOT_ALLOWED, body).into_response()
    }
}

use axum::response::IntoResponse;

async fn health() -> &'static str {
    "ok"
}

/// Connect pools, run migrations, and return ready state.
/// Which cargo profile this binary was built with.
///
/// `debug_assertions` is the only signal that cannot drift from reality: it
/// is set by the profile itself rather than passed in, so a node cannot be
/// told it is something it is not. A debug node behaves identically and runs
/// several times slower, which matters to exactly two audiences — whoever is
/// wondering why the stand is sluggish, and anything recording a benchmark.
pub fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

pub async fn init(config: config::Config) -> anyhow::Result<AppState> {
    db::migrate::run(
        &config.database_url,
        std::time::Duration::from_secs(config.db_wait_secs),
    )
    .await?;
    let (pool, ddi_pool) = db::connect(&config).await?;
    let base = std::path::Path::new(&config.artifact_dir);
    // Local scratch for the upload spool: an artifact is named by its own
    // sha1, so it is written and hashed here before any extension can be
    // told where it belongs. Not a store — the store is an extension.
    artifact::ensure_base_dir(base)?;

    let extensions = ext::ExtRegistry::from_config(&config)?;
    if extensions.storage.is_none() {
        // Not fatal: a node with no storage extension still serves polls,
        // deployments and feedback, and answers 503 for artifacts. Loud,
        // because it is almost never what anyone meant.
        tracing::warn!("no HAWIER_EXT_STORAGE configured - artifact requests will answer 503");
    }
    let storage = artifact::Storage::new(
        extensions.clone(),
        base,
        reqwest::Client::builder()
            .build()
            .map_err(|e| anyhow::Error::from(e).context("building the artifact HTTP client"))?,
    );
    Ok(AppState::new(config, pool, ddi_pool, storage, extensions))
}

/// Start role-dependent background engines (rollout + auto-assign +
/// housekeeping + outbox delivery on the control node; sub nodes run none).
///
/// Sub nodes still *emit* events — they serve DDI — they just never deliver
/// them. With control down, the outbox queues instead of losing anything.
pub fn spawn_background(state: &AppState) {
    if state.config.role == Role::Control {
        rollout::spawn(state.clone());
        housekeeping::spawn(state.clone());
        outbox::spawn(state.clone());
    }
}
