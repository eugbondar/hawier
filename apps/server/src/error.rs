//! Error taxonomy, wire-compatible with hawkBit's `ExceptionInfo` JSON:
//! `{"exceptionClass": "...", "errorCode": "...", "message": "..."}`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// hawkBit-style error body.
#[derive(Debug, Serialize)]
pub struct ExceptionInfo {
    #[serde(rename = "exceptionClass")]
    pub exception_class: String,
    #[serde(rename = "errorCode")]
    pub error_code: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Gone(String),
    #[error("{0}")]
    Quota(String),
    #[error("{0}")]
    InvalidRsql(String),
    /// A set offered to a target whose type does not list the set's type —
    /// or, with `target.type.required`, to a target with no type at all.
    #[error("{0}")]
    IncompatibleTargetType(String),
    /// A dependency this node needs is not answering — today, an extension.
    /// Busy, not broken: it answers 503 with Retry-After for the same reason
    /// resource exhaustion does (see `exhausted_resource`).
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// A node that ran out of a resource is not broken — it is busy, and the
/// difference is the whole protocol. `500` tells a device "the server is
/// broken, try again", and a fleet obeys that immediately: one benchmark run
/// logged 154k `500`s from a node that had exhausted its file descriptors,
/// each one answered with another request. `503` plus `Retry-After` says
/// "come back later", which is the only thing that can break the loop from
/// the server side.
///
/// Detected here rather than at every call site, so any path that opens a
/// file or takes a connection is covered without remembering to.
fn exhausted_resource(error: &AppError) -> Option<&'static str> {
    fn from_io(error: &std::io::Error) -> Option<&'static str> {
        match error.raw_os_error() {
            // EMFILE (per-process) and ENFILE (system-wide).
            Some(24) | Some(23) => Some("file descriptors"),
            _ => None,
        }
    }
    match error {
        AppError::Unavailable(_) => Some("an extension"),
        AppError::Db(sqlx::Error::PoolTimedOut) => Some("database connections"),
        AppError::Db(sqlx::Error::Io(io)) => from_io(io),
        AppError::Internal(error) => error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>().and_then(from_io)),
        _ => None,
    }
}

/// How long to tell a device to wait. Spread over a few seconds so a fleet
/// that was refused together does not return together — the clock is the
/// only source of variation available this deep, and requests within one
/// millisecond are rare enough that it does the job.
fn retry_after_seconds() -> u64 {
    3 + crate::state::now_millis().unsigned_abs() % 5
}

impl AppError {
    /// True when this is "busy", not "broken" — see [`exhausted_resource`].
    pub fn is_overload(&self) -> bool {
        exhausted_resource(self).is_some()
    }

    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        // (status, exceptionClass, errorCode) — codes mirror hawkBit's
        // SpServerError values that clients may switch on.
        match self {
            AppError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                "org.eclipse.hawkbit.repository.exception.EntityNotFoundException",
                "hawkbit.server.error.repo.entitiyNotFound",
            ),
            AppError::BadRequest(_) => (
                StatusCode::BAD_REQUEST,
                "org.eclipse.hawkbit.repository.exception.InvalidParameterException",
                "hawkbit.server.error.repo.invalidParameter",
            ),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "org.springframework.security.authentication.BadCredentialsException",
                "hawkbit.server.error.unauthorized",
            ),
            AppError::Forbidden(_) => (
                StatusCode::FORBIDDEN,
                "org.springframework.security.access.AccessDeniedException",
                "hawkbit.server.error.forbidden",
            ),
            AppError::Conflict(_) => (
                StatusCode::CONFLICT,
                "org.eclipse.hawkbit.repository.exception.EntityAlreadyExistsException",
                "hawkbit.server.error.repo.entitiyAlreayExists",
            ),
            AppError::Gone(_) => (
                StatusCode::GONE,
                "org.eclipse.hawkbit.repository.exception.CancelActionNotAllowedException",
                "hawkbit.server.error.repo.actionNotCancelable",
            ),
            AppError::Quota(_) => (
                StatusCode::FORBIDDEN,
                "org.eclipse.hawkbit.repository.exception.AssignmentQuotaExceededException",
                "hawkbit.server.error.repo.quotaExceeded",
            ),
            AppError::InvalidRsql(_) => (
                StatusCode::BAD_REQUEST,
                "org.eclipse.hawkbit.repository.exception.RSQLParameterSyntaxException",
                "hawkbit.server.error.rest.param.rsqlInvalid",
            ),
            AppError::IncompatibleTargetType(_) => (
                StatusCode::BAD_REQUEST,
                "org.eclipse.hawkbit.repository.exception.IncompatibleTargetTypeException",
                "hawkbit.server.error.target.type.incompatible",
            ),
            // Same shape as the overload 503 below, and for the same
            // reason: busy is not broken.
            AppError::Unavailable(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "org.springframework.web.client.HttpServerErrorException$ServiceUnavailable",
                "hawkbit.server.error.serviceUnavailable",
            ),
            AppError::Db(_) | AppError::Internal(_) if self.is_overload() => (
                StatusCode::SERVICE_UNAVAILABLE,
                // Additive to hawkBit, which has no 503 in its taxonomy
                // (API_NOTES §5). A real Spring type rather than an invented
                // org.eclipse one: a client switching on the class name will
                // not match either way, and a fake package would be a lie.
                "org.springframework.web.client.HttpServerErrorException$ServiceUnavailable",
                "hawkbit.server.error.serviceUnavailable",
            ),
            AppError::Db(_) | AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "java.lang.RuntimeException",
                "hawkbit.server.error.internal",
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, class, code) = self.parts();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "internal error");
        }
        let overloaded = exhausted_resource(&self);
        if let Some(resource) = overloaded {
            tracing::warn!(resource, error = %self, "out of capacity, answering 503");
        }
        let body = ExceptionInfo {
            exception_class: class.to_string(),
            error_code: code.to_string(),
            message: self.to_string(),
        };
        let mut resp = (status, Json(body)).into_response();
        if overloaded.is_some() {
            resp.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(retry_after_seconds()),
            );
        }
        if status == StatusCode::UNAUTHORIZED {
            resp.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Basic realm=\"hawier\""),
            );
        }
        resp
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod overload_tests {
    use super::*;
    use axum::http::header::RETRY_AFTER;

    fn status_and_retry_after(error: AppError) -> (StatusCode, Option<u64>) {
        let response = error.into_response();
        let retry = response
            .headers()
            .get(RETRY_AFTER)
            .map(|value| value.to_str().unwrap().parse().unwrap());
        (response.status(), retry)
    }

    #[test]
    fn running_out_of_connections_is_busy_not_broken() {
        let (status, retry) = status_and_retry_after(AppError::Db(sqlx::Error::PoolTimedOut));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        // The delay has to be there and has to be sane: without it a device
        // has nothing to obey, and 500-with-no-hint is what fed the storm.
        let seconds = retry.expect("503 without Retry-After is a 500 in disguise");
        assert!((3..8).contains(&seconds), "{seconds}s");
    }

    #[test]
    fn descriptor_exhaustion_is_found_anywhere_in_the_chain() {
        // The artifact path wraps the io error in context before it gets
        // here; a check that only looked at the top would miss it.
        let io = std::io::Error::from_raw_os_error(24); // EMFILE
        let buried = anyhow::Error::from(io)
            .context("open artifact")
            .context("GET /artifacts/abc");
        let (status, retry) = status_and_retry_after(AppError::Internal(buried));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(retry.is_some());
    }

    #[test]
    fn an_ordinary_failure_is_still_a_500_without_a_hint() {
        let (status, retry) =
            status_and_retry_after(AppError::Internal(anyhow::anyhow!("bug in a query")));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(retry, None, "a broken server must not invite a retry");
    }

    #[test]
    fn a_missing_file_is_not_exhaustion() {
        let io = std::io::Error::from(std::io::ErrorKind::NotFound);
        let (status, _) = status_and_retry_after(AppError::Internal(anyhow::Error::from(io)));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[cfg(test)]
mod code_tests {
    use super::*;

    #[tokio::test]
    async fn incompatible_target_type_speaks_hawkbit() {
        let response =
            AppError::IncompatibleTargetType("Targets of types [rev2] ...".into()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["errorCode"],
            "hawkbit.server.error.target.type.incompatible"
        );
        assert_eq!(
            body["exceptionClass"],
            "org.eclipse.hawkbit.repository.exception.IncompatibleTargetTypeException"
        );
    }
}
