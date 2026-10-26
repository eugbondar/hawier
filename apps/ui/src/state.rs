use reqwest::Client;

use crate::{config::Config, ui_api::UiApi};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub client: Client,
    /// hawier's internal /ui/v1 state API — the only home of UI state
    /// (views, logs, sessions); nothing is kept in process memory anymore.
    pub ui_api: UiApi,
}

impl AppState {
    pub fn new(config: Config, client: Client) -> Self {
        let ui_api = UiApi::new(client.clone(), &config);
        Self {
            config,
            client,
            ui_api,
        }
    }
}
