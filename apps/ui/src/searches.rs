//! Recent global searches, stored server-side (`/ui/v1/searches`) so the
//! history follows the user between browsers.

use serde_json::{json, Value};

use crate::{session::Session, state::AppState, ui_api::urlencode};

/// One remembered query.
pub struct RecentSearch {
    pub id: String,
    pub q: String,
}

fn scope(session: &Session) -> String {
    format!(
        "tenant={}&username={}",
        urlencode(&session.tenant),
        urlencode(&session.username)
    )
}

pub async fn recent(state: &AppState, session: &Session, limit: usize) -> Vec<RecentSearch> {
    let rows: Vec<Value> = state
        .ui_api
        .get_json(&format!("/ui/v1/searches?{}&limit={limit}", scope(session)))
        .await
        .unwrap_or_default();
    rows.iter()
        .filter_map(|row| {
            Some(RecentSearch {
                id: row.get("id")?.as_str()?.to_string(),
                q: row.get("q")?.as_str()?.to_string(),
            })
        })
        .collect()
}

pub async fn record(state: &AppState, session: &Session, q: &str) {
    if q.trim().is_empty() {
        return;
    }
    let _ = state
        .ui_api
        .post_json(
            "/ui/v1/searches",
            &json!({
                "tenant": session.tenant,
                "username": session.username,
                "q": q.trim(),
            }),
        )
        .await;
}

pub async fn delete(state: &AppState, id: &str) {
    let _ = state
        .ui_api
        .delete(&format!("/ui/v1/searches/{}", urlencode(id)))
        .await;
}

pub async fn clear(state: &AppState, session: &Session) {
    let _ = state
        .ui_api
        .delete(&format!("/ui/v1/searches?{}", scope(session)))
        .await;
}
