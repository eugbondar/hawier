use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    /// Mgmt API base (hawier-server, or a stock hawkBit during transition).
    pub hawkbit_mgmt_url: String,
    /// hawier's internal /ui/v1 state API (defaults to the mgmt URL — one
    /// hawier server provides both).
    pub hawier_api_url: String,
    /// Bearer token for /ui/v1 (must match the server's HAWIER_INTERNAL_TOKEN).
    pub internal_token: String,
    pub static_dir: PathBuf,
    pub dev_mode: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("HAWIER_BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8088".to_string())
            .parse()
            .context("HAWIER_BIND_ADDR must be a socket address")?;
        let hawkbit_mgmt_url = trim_url(
            &env::var("HAWKBIT_MGMT_URL").unwrap_or_else(|_| "http://localhost:8080".to_string()),
        );
        let hawier_api_url = env::var("HAWIER_API_URL")
            .map(|url| trim_url(&url))
            .unwrap_or_else(|_| hawkbit_mgmt_url.clone());
        let internal_token =
            env::var("HAWIER_INTERNAL_TOKEN").unwrap_or_else(|_| "dev-internal-token".to_string());
        let static_dir = env::var("HAWIER_STATIC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("static"));
        let dev_mode = env::var("HAWIER_DEV_MODE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Ok(Self {
            bind_addr,
            hawkbit_mgmt_url,
            hawier_api_url,
            internal_token,
            static_dir,
            dev_mode,
        })
    }
}

fn trim_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}
