//! Sessions — persisted in hawier's Postgres via `/ui/v1/sessions`;
//! the browser only holds the opaque session id in a cookie.

use axum::{
    http::{
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::Response,
};
use base64::{engine::general_purpose, Engine as _};
use hawier_api::ui_state::{UiSession, UiSessionCreate};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, state::AppState};

pub const SESSION_COOKIE: &str = "hawier_session";

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub username: String,
    pub tenant: String,
    // NARROW: the mgmt password rides along to replay HTTP Basic upstream
    // (stored server-side in hawier_session; see SCHEMA_NOTES.md #18).
    #[serde(skip_serializing)]
    pub password: String,
}

impl From<UiSession> for Session {
    fn from(session: UiSession) -> Self {
        Self {
            username: session.username,
            tenant: session.tenant,
            password: session.password,
        }
    }
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default = "default_tenant")]
    pub tenant: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub authenticated: bool,
    pub username: Option<String>,
    pub tenant: Option<String>,
}

pub async fn current(state: &AppState, headers: &HeaderMap) -> Option<Session> {
    let session_id = session_id_from_headers(headers)?;
    state
        .ui_api
        .get_json_opt::<UiSession>(&format!("/ui/v1/sessions/{session_id}"))
        .await
        .ok()
        .flatten()
        .map(Session::from)
}

pub async fn require(state: &AppState, headers: &HeaderMap) -> Result<Session, AppError> {
    current(state, headers)
        .await
        .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "login required"))
}

pub async fn authenticate(
    state: &AppState,
    request: LoginRequest,
) -> Result<(Session, String), AppError> {
    if request.username.trim().is_empty() || request.password.is_empty() {
        return Err(AppError::bad_request("username and password are required"));
    }

    let session = Session {
        username: request.username.trim().to_string(),
        password: request.password,
        tenant: empty_as_default(request.tenant),
    };
    crate::hawkbit::verify_credentials(state, &session).await?;

    let created: UiSession = state
        .ui_api
        .send_json(
            reqwest::Method::POST,
            "/ui/v1/sessions",
            &UiSessionCreate {
                username: session.username.clone(),
                tenant: session.tenant.clone(),
                password: session.password.clone(),
            },
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to create session");
            AppError::new(StatusCode::BAD_GATEWAY, "session store unavailable")
        })?;

    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=28800",
        created.token
    );
    Ok((session, cookie))
}

pub async fn clear(state: &AppState, headers: &HeaderMap, mut response: Response) -> Response {
    if let Some(session_id) = session_id_from_headers(headers) {
        state
            .ui_api
            .delete(&format!("/ui/v1/sessions/{session_id}"))
            .await
            .ok();
    }
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static("hawier_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    response
}

pub fn auth_headers(session: &Session) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let token =
        general_purpose::STANDARD.encode(format!("{}:{}", session.username, session.password));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Basic {token}")).expect("basic auth is a valid header"),
    );
    // Lets the server file console-caused incidents apart from real API
    // clients (see hawier_incident.kind).
    headers.insert("X-Hawier-Origin", HeaderValue::from_static("console"));
    headers.insert(
        "X-Hawkbit-Tenant",
        HeaderValue::from_str(&session.tenant)
            .unwrap_or_else(|_| HeaderValue::from_static("DEFAULT")),
    );
    headers
}

pub fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|cookie| {
        let mut parts = cookie.trim().splitn(2, '=');
        match (parts.next(), parts.next()) {
            (Some(name), Some(value)) if name == SESSION_COOKIE => Some(value.to_string()),
            _ => None,
        }
    })
}

pub fn default_tenant() -> String {
    "DEFAULT".to_string()
}

fn empty_as_default(value: String) -> String {
    if value.trim().is_empty() {
        default_tenant()
    } else {
        value.trim().to_string()
    }
}
