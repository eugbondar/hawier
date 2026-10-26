use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hawier_server=info,tower_http=info".into()),
        )
        .init();

    let config = hawier_server::config::Config::from_env()?;
    let bind_addr = config.bind_addr;
    let role = config.role;

    let state = hawier_server::init(config).await?;
    hawier_server::spawn_background(&state);
    info!(%bind_addr, ?role, "hawier server listening");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, hawier_server::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
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
