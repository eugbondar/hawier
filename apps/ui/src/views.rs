//! Saved views — thin client over hawier's `/ui/v1/views` API.

use anyhow::Result;

use crate::{session::Session, state::AppState, ui_api::urlencode};

pub use hawier_api::ui_state::SavedView;
use hawier_api::ui_state::{SavedViewCreate, SavedViewReorder, SavedViewUpdate};

#[derive(Default)]
pub struct SavedViewDraft {
    pub resource: String,
    pub name: String,
    pub q: String,
    pub columns: String,
    pub sort: String,
    pub page_size: usize,
}

fn scope_query(session: &Session, resource: Option<&str>) -> String {
    let mut query = format!(
        "tenant={}&username={}",
        urlencode(&session.tenant),
        urlencode(&session.username)
    );
    if let Some(resource) = resource {
        query.push_str(&format!("&resource={}", urlencode(resource)));
    }
    query
}

pub async fn for_resource(state: &AppState, session: &Session, resource: &str) -> Vec<SavedView> {
    state
        .ui_api
        .get_json::<Vec<SavedView>>(&format!(
            "/ui/v1/views?{}",
            scope_query(session, Some(resource))
        ))
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to load saved views");
            Vec::new()
        })
}

pub async fn all_for_user(state: &AppState, session: &Session) -> Vec<SavedView> {
    state
        .ui_api
        .get_json::<Vec<SavedView>>(&format!("/ui/v1/views?{}", scope_query(session, None)))
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to load saved views");
            Vec::new()
        })
}

pub async fn find_for_user(state: &AppState, session: &Session, id: &str) -> Option<SavedView> {
    all_for_user(state, session)
        .await
        .into_iter()
        .find(|view| view.id == id)
}

pub async fn find_for_resource(
    state: &AppState,
    session: &Session,
    resource: &str,
    id: &str,
) -> Option<SavedView> {
    for_resource(state, session, resource)
        .await
        .into_iter()
        .find(|view| view.id == id)
}

pub async fn create(
    state: &AppState,
    session: &Session,
    draft: SavedViewDraft,
) -> Result<SavedView> {
    let body = SavedViewCreate {
        tenant: session.tenant.clone(),
        username: session.username.clone(),
        resource: draft.resource,
        name: draft.name,
        q: draft.q,
        columns: draft.columns,
        sort: draft.sort,
        page_size: draft.page_size.max(10),
    };
    state
        .ui_api
        .send_json(reqwest::Method::POST, "/ui/v1/views", &body)
        .await
}

pub async fn update(
    state: &AppState,
    session: &Session,
    id: &str,
    draft: SavedViewDraft,
) -> Result<Option<SavedView>> {
    // Ownership check first — /ui/v1 trusts its caller.
    if find_for_user(state, session, id).await.is_none() {
        return Ok(None);
    }
    let body = SavedViewUpdate {
        name: Some(draft.name),
        q: Some(draft.q),
        columns: Some(draft.columns),
        sort: Some(draft.sort),
        page_size: Some(draft.page_size.max(10)),
    };
    let updated = state
        .ui_api
        .send_json(reqwest::Method::PUT, &format!("/ui/v1/views/{id}"), &body)
        .await?;
    Ok(Some(updated))
}

pub async fn delete(state: &AppState, session: &Session, id: &str) -> Result<Option<SavedView>> {
    let Some(view) = find_for_user(state, session, id).await else {
        return Ok(None);
    };
    state.ui_api.delete(&format!("/ui/v1/views/{id}")).await?;
    Ok(Some(view))
}

pub async fn duplicate(state: &AppState, session: &Session, id: &str) -> Result<Option<SavedView>> {
    let Some(view) = find_for_user(state, session, id).await else {
        return Ok(None);
    };
    let copy = create(
        state,
        session,
        SavedViewDraft {
            resource: view.resource.clone(),
            name: format!("{} copy", view.name),
            q: view.q.clone(),
            columns: view.columns.clone(),
            sort: view.sort.clone(),
            page_size: view.page_size,
        },
    )
    .await?;
    Ok(Some(copy))
}

pub async fn move_view(
    state: &AppState,
    session: &Session,
    id: &str,
    direction: &str,
) -> Result<Option<SavedView>> {
    let Some(view) = find_for_user(state, session, id).await else {
        return Ok(None);
    };
    let mut views = for_resource(state, session, &view.resource).await;
    views.sort_by_key(|v| v.position);
    let Some(current) = views.iter().position(|v| v.id == id) else {
        return Ok(None);
    };
    let target = match direction {
        "up" if current > 0 => current - 1,
        "down" if current + 1 < views.len() => current + 1,
        _ => current,
    };
    if target != current {
        views.swap(current, target);
        let body = SavedViewReorder {
            tenant: session.tenant.clone(),
            username: session.username.clone(),
            resource: view.resource.clone(),
            ids: views.iter().map(|v| v.id.clone()).collect(),
        };
        let _: serde_json::Value = state
            .ui_api
            .send_json(reqwest::Method::PUT, "/ui/v1/views/reorder", &body)
            .await
            .unwrap_or(serde_json::Value::Null);
    }
    Ok(Some(view))
}
