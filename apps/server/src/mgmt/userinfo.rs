//! `GET /rest/v1/userinfo` — Basic-auth validation probe.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::MgmtIdentity;
use crate::error::AppResult;
use crate::state::AppState;

#[derive(Serialize)]
pub struct MgmtUserInfo {
    pub tenant: String,
    pub username: String,
    pub permissions: Vec<String>,
}

pub async fn get_userinfo(
    State(state): State<AppState>,
    identity: MgmtIdentity,
) -> AppResult<Json<MgmtUserInfo>> {
    // What this identity actually holds, so the console can hide what it
    // cannot use instead of offering a button that answers 403.
    let permissions = crate::authz::permissions_of(&state, &identity).await?;
    Ok(Json(MgmtUserInfo {
        tenant: identity.tenant,
        username: identity.username,
        permissions,
    }))
}
