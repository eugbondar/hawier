//! Action log: written through hawier's internal `/ui/v1/logs`, read back
//! through the ordinary mgmt resource `/rest/v1/actionlog` — so reads are
//! scoped by the caller's own credentials and filtered by the server (the
//! previous client-side filtering pulled 1000 rows for every lookup).

use anyhow::Result;
use serde_json::Value;

use crate::{hawkbit, session::Session, state::AppState};

pub use hawier_api::ui_state::ActionLog;
use hawier_api::ui_state::{ActionLogFinish, ActionLogStart};

pub struct ActionLogDraft<'a> {
    pub resource: &'a str,
    pub resource_id: &'a str,
    pub action: &'a str,
    pub request_summary: String,
}

pub async fn record_started(
    state: &AppState,
    session: &Session,
    draft: ActionLogDraft<'_>,
) -> Result<ActionLog> {
    let body = ActionLogStart {
        tenant: session.tenant.clone(),
        username: session.username.clone(),
        resource: draft.resource.to_string(),
        resource_id: draft.resource_id.to_string(),
        action: draft.action.to_string(),
        request_summary: draft.request_summary,
    };
    state
        .ui_api
        .send_json(reqwest::Method::POST, "/ui/v1/logs", &body)
        .await
}

pub async fn finish(
    state: &AppState,
    id: &str,
    status: &str,
    response_summary: String,
    error: String,
) -> Result<()> {
    let body = ActionLogFinish {
        status: status.to_string(),
        response_summary,
        error,
    };
    let _: ActionLog = state
        .ui_api
        .send_json(reqwest::Method::PUT, &format!("/ui/v1/logs/{id}"), &body)
        .await?;
    Ok(())
}

/// One page of the log, filtered server-side by RSQL.
async fn fetch(state: &AppState, session: &Session, q: Option<&str>, limit: usize) -> Vec<Value> {
    let Some(resource) = crate::resources::by_id("action-log") else {
        return Vec::new();
    };
    match hawkbit::fetch_page(
        state,
        session,
        &resource.endpoint,
        q,
        Some("createdAt:DESC"),
        1,
        limit,
    )
    .await
    {
        Ok(page) => hawkbit::read_items(&page),
        Err(error) => {
            tracing::warn!(error = %error.message, "failed to load the action log");
            Vec::new()
        }
    }
}

/// Free-text lookup across the searchable columns of the log.
///
/// The console has no "recent actions" panels any more: the journal is one
/// resource (Action Log, under Admin) with the ordinary filter/sort/paging,
/// so nothing else needs to pull rows out of it.
pub async fn search(state: &AppState, session: &Session, q: &str, limit: usize) -> Vec<Value> {
    let needle = hawkbit::rsql_literal(q);
    if needle.is_empty() {
        return Vec::new();
    }
    let q = ["action", "resource", "error", "resourceid"]
        .map(|field| format!("{field}==\"*{needle}*\""))
        .join(",");
    fetch(state, session, Some(&q), limit).await
}
