use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use tokio::net::TcpListener;
use tracing::info;

use hawier_ui::{config::Config, routes::build_router, state::AppState};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hawier_ui=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let state = AppState::new(
        config.clone(),
        Client::builder()
            .user_agent(concat!("hawier-ui/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?,
    );
    // The tree is assembled once, at startup: what extensions declare is
    // state like any other, so it comes from the node's /ui/v1 API. A node
    // that cannot answer leaves the console with its built-in tree, which is
    // a working console — not a reason to refuse to start.
    //
    // But "cannot answer" and "has not started yet" are different, and the
    // second is the ordinary case: compose starts the console and the node
    // together, and the node needs a few seconds to load its modules. One
    // attempt at t=0 made a startup race decide which features exist for the
    // life of the process, silently — the tree is adopted once, so a console
    // that lost the race stays wrong until someone restarts it.
    adopt_surfaces(&state).await;

    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;

    info!(addr = %config.bind_addr, "hawier backend listening");
    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server failed")?;

    Ok(())
}

/// How long to wait for the node before settling for the built-in tree.
/// Long enough to cover a cold start of the node beside us, short enough
/// that a console pointed at nothing still comes up on its own.
const SURFACE_WAIT: Duration = Duration::from_secs(60);
const SURFACE_RETRY: Duration = Duration::from_secs(2);

async fn adopt_surfaces(state: &AppState) {
    match hawier_ui::ui_api::fetch_surfaces_waiting(state, SURFACE_WAIT, SURFACE_RETRY).await {
        Ok(declaration) => hawier_ui::resources::adopt(declaration),
        Err(error) => tracing::warn!(
            %error,
            waited_secs = SURFACE_WAIT.as_secs(),
            "could not read console surfaces; using the built-in tree"
        ),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
