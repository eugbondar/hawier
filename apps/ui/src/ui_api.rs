//! HTTP client for hawier's internal `/ui/v1` state API.
//!
//! All UI state (saved views, action logs, sessions) lives in hawier's
//! Postgres behind this API — the UI has no direct database access.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::Config;

#[derive(Clone)]
pub struct UiApi {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl UiApi {
    pub fn new(client: reqwest::Client, config: &Config) -> Self {
        Self {
            client,
            base: config.hawier_api_url.clone(),
            token: config.internal_token.clone(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?
            .error_for_status()
            .with_context(|| format!("GET {path}"))?;
        Ok(resp.json().await?)
    }

    /// GET that maps 404 to None.
    pub async fn get_json_opt<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        let resp = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = resp
            .error_for_status()
            .with_context(|| format!("GET {path}"))?;
        Ok(Some(resp.json().await?))
    }

    pub async fn send_json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .request(method.clone(), path)
            .json(body)
            .send()
            .await
            .with_context(|| format!("{method} {path}"))?
            .error_for_status()
            .with_context(|| format!("{method} {path}"))?;
        Ok(resp.json().await?)
    }

    /// POST whose response body carries nothing useful (204 and friends).
    pub async fn post_json<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.request(reqwest::Method::POST, path)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {path}"))?
            .error_for_status()
            .with_context(|| format!("POST {path}"))?;
        Ok(())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.request(reqwest::Method::DELETE, path)
            .send()
            .await
            .with_context(|| format!("DELETE {path}"))?
            .error_for_status()
            .with_context(|| format!("DELETE {path}"))?;
        Ok(())
    }
}

pub fn urlencode(value: &str) -> String {
    serde_urlencoded::to_string([("v", value)])
        .map(|s| s.trim_start_matches("v=").to_string())
        .unwrap_or_default()
}

/// What extensions put in the console tree, read once at startup.
///
/// The console has no extension host of its own — the node does — so this is
/// state like saved views or sessions, fetched over the same internal API.
pub async fn fetch_surfaces(
    state: &crate::state::AppState,
) -> Result<hawier_api::ext::ConsoleDeclaration> {
    state
        .ui_api
        .get_json("/ui/v1/surfaces")
        .await
        .context("reading console surfaces")
}

/// The same read, but willing to wait for a node that is still starting.
///
/// The tree is adopted once, so whatever this returns is what the console
/// shows until it is restarted. A single attempt at t=0 therefore let the
/// startup order decide which features exist — and the ordinary deployment
/// starts the console and the node together, so the console lost that race
/// routinely and silently fell back to its built-in tree.
pub async fn fetch_surfaces_waiting(
    state: &crate::state::AppState,
    wait: std::time::Duration,
    retry: std::time::Duration,
) -> Result<hawier_api::ext::ConsoleDeclaration> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match fetch_surfaces(state).await {
            Ok(declaration) => return Ok(declaration),
            Err(error) if std::time::Instant::now() + retry >= deadline => return Err(error),
            Err(_) => tokio::time::sleep(retry).await,
        }
    }
}

// ---------------------------------------------------------------------------

/// The console adopts its tree once, at startup — so startup is where an
/// extension-declared surface can be lost, and nothing later notices.
///
/// No database here: this is the console talking to a node that is not up
/// yet, which is what compose does on every `up`. They are unit tests and not
/// an integration target because a target of their own is another ~90 MB
/// executable, and because `make test` has to keep running them.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{routing::get, Json, Router};
    use serde_json::json;

    use crate::{config::Config, state::AppState};

    fn state_pointing_at(addr: std::net::SocketAddr) -> AppState {
        AppState::new(
            Config {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                hawkbit_mgmt_url: format!("http://{addr}"),
                hawier_api_url: format!("http://{addr}"),
                internal_token: "it-surfaces-token".into(),
                static_dir: "static".into(),
                dev_mode: false,
            },
            reqwest::Client::new(),
        )
    }

    /// A free port nobody is listening on: the bind proves it is ours to take,
    /// the drop makes connections to it refuse rather than hang.
    async fn reserved_addr() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    }

    async fn serve_surfaces(addr: std::net::SocketAddr) {
        let app = Router::new().route(
            "/ui/v1/surfaces",
            get(|| async {
                Json(json!({
                    "surfaces": [{
                        "id": "late-arrival",
                        "label": "Late Arrival",
                        "section": "inventory",
                        "icon": "target",
                        "endpoint": "targets",
                        "columns": ["controllerId"],
                        "views": [],
                    }],
                    "views": [],
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind(addr).await.expect("rebind");
        axum::serve(listener, app).await.ok();
    }

    /// The console started first — the ordinary case, since compose starts both
    /// at once and the node has modules to load. It must still end up with the
    /// declared surface, not with the built-in tree.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_node_that_starts_late_still_gets_its_surfaces_adopted() {
        let addr = reserved_addr().await;
        let state = state_pointing_at(addr);

        // Nothing is listening yet: the first attempts must fail.
        assert!(
            super::fetch_surfaces(&state).await.is_err(),
            "the node is not up — a single attempt is supposed to fail here"
        );

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            serve_surfaces(addr).await;
        });

        let declaration = super::fetch_surfaces_waiting(
            &state,
            Duration::from_secs(10),
            Duration::from_millis(100),
        )
        .await
        .expect("the node came up inside the wait, so the surfaces must arrive");

        assert_eq!(
            declaration
                .surfaces
                .iter()
                .map(|surface| surface.id.as_str())
                .collect::<Vec<_>>(),
            ["late-arrival"],
        );
    }

    /// A console pointed at a node that never answers still has to come up, so
    /// the wait is bounded and reports rather than blocks forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_node_that_never_answers_gives_up_and_lets_the_console_start() {
        let addr = reserved_addr().await;
        let state = state_pointing_at(addr);

        let started = std::time::Instant::now();
        let outcome = super::fetch_surfaces_waiting(
            &state,
            Duration::from_millis(500),
            Duration::from_millis(100),
        )
        .await;

        assert!(outcome.is_err(), "nothing ever answered on {addr}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the wait is bounded: {:?}",
            started.elapsed()
        );
    }
}
