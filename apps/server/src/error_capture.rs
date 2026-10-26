//! Error-response middleware: every 4xx/5xx gets the request's incident
//! number as `"incident"` in its JSON body, plain-text rejections (axum
//! extractor failures, 405) are wrapped into hawkBit-style `ExceptionInfo`,
//! and errors are journaled into `hawier_incident` keyed by that same
//! number (UI page /incidents).
//!
//! Journal policy: 5xx — always; 4xx — mgmt (`/rest`) and `/ui` only. DDI
//! 4xx (`/{tenant}/controller/...`) are NOT journaled — a misconfigured
//! fleet would flood the table; they still carry the incident number.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine as _;
use serde_json::json;

use crate::state::AppState;

/// Max journaled rows (trim on insert, like the UI action log).
const ERROR_LOG_RETENTION: i64 = 10_000;
/// Only bodies up to this size are inspected/rewritten.
const MAX_BODY: usize = 64 * 1024;

pub async fn error_capture(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    // The incident-id layer minted this for every request.
    let incident_id: i64 = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| state.incidents.next());
    // Best-effort attribution (unverified Basic username; DDI tenant from path).
    let username = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Basic "))
        .and_then(|b64| {
            base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .ok()
        })
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|creds| creds.split_once(':').map(|(u, _)| u.to_string()));
    let is_ddi = !path.starts_with("/rest/") && !path.starts_with("/ui/") && path != "/health";
    // Where did this request come from? The console marks its own upstream
    // calls, so a bad sort issued by our UI is not mistaken for a real API
    // client failing (dashboards show devices and clients, not our noise).
    let from_console = req
        .headers()
        .get("x-hawier-origin")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("console"));
    let kind = if is_ddi {
        "ddi"
    } else if path.starts_with("/ui/") {
        "ui"
    } else if from_console {
        "console"
    } else {
        "mgmt"
    };
    let tenant = if is_ddi {
        path.split('/').nth(1).map(str::to_string)
    } else {
        None
    };

    let response = next.run(req).await;
    let status = response.status();
    if status.as_u16() < 400 {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        // Body larger than the cap or unreadable: pass through untouched.
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    let mut info: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
        // Plain-text rejection (bad query/body, 405, 415...) → ExceptionInfo.
        _ => json!({
            "exceptionClass": "org.springframework.web.server.ResponseStatusException",
            "errorCode": rejection_code(status),
            "message": String::from_utf8_lossy(&bytes).trim().to_string(),
        }),
    };
    if let Some(object) = info.as_object_mut() {
        // A string: 19-digit numbers do not survive JS's f64 JSON parsing.
        object.insert("incident".into(), json!(incident_id.to_string()));
    }

    let journal = status.as_u16() >= 500 || !is_ddi;
    if journal {
        let error_code = info
            .get("errorCode")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let message: String = info
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .chars()
            .take(2000)
            .collect();
        let pool = state.pool.clone();
        // Journal off the hot path; an unlogged error must not fail the response.
        tokio::spawn(async move {
            let insert = sqlx::query(
                "INSERT INTO hawier_incident
                 (id, kind, tenant, username, method, path, status, error_code, message)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(incident_id)
            .bind(kind)
            .bind(&tenant)
            .bind(&username)
            .bind(&method)
            .bind(&path)
            .bind(status.as_u16() as i32)
            .bind(&error_code)
            .bind(&message)
            .execute(&pool)
            .await;
            if let Err(error) = insert {
                tracing::warn!(%error, "failed to journal incident");
                return;
            }
            sqlx::query(
                "DELETE FROM hawier_incident WHERE id IN (
                     SELECT id FROM hawier_incident
                     ORDER BY occurred_at DESC OFFSET $1)",
            )
            .bind(ERROR_LOG_RETENTION)
            .execute(&pool)
            .await
            .ok();
        });
    }

    let payload = serde_json::to_vec(&info).unwrap_or_else(|_| bytes.to_vec());
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Response::from_parts(parts, Body::from(payload))
}

fn rejection_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::METHOD_NOT_ALLOWED => "hawkbit.server.error.methodNotAllowed",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "hawkbit.server.error.unsupportedMediaType",
        StatusCode::NOT_FOUND => "hawkbit.server.error.notFound",
        s if s.is_server_error() => "hawkbit.server.error.internal",
        _ => "hawkbit.server.error.rest.invalidRequest",
    }
}
