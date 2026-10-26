use axum::{
    body::Bytes,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        HeaderMap, Method, StatusCode, Uri,
    },
    response::Response,
};
use serde_json::{json, Value};

use crate::{
    error::AppError,
    session::{self, Session},
    state::AppState,
};

pub struct ProxyRequest {
    pub headers: HeaderMap,
    pub method: Method,
    pub uri: Uri,
    pub path: String,
    pub query: std::collections::HashMap<String, String>,
    pub body: Bytes,
}

pub async fn verify_credentials(state: &AppState, session: &Session) -> Result<(), AppError> {
    let response = state
        .client
        .get(format!(
            "{}/rest/v1/rollouts?limit=1",
            state.config.hawkbit_mgmt_url
        ))
        .headers(session::auth_headers(session))
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;

    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return Err(AppError::new(
            response.status(),
            "invalid hawkBit credentials",
        ));
    }

    Ok(())
}

pub async fn status(state: &AppState, session: &Session) -> Result<Value, AppError> {
    let response = state
        .client
        .get(format!("{}/rest/v1", state.config.hawkbit_mgmt_url))
        .headers(session::auth_headers(session))
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;

    Ok(json!({
        "ok": response.status().is_success() || response.status() == StatusCode::NOT_FOUND,
        "status": response.status().as_u16(),
        "mgmtUrl": state.config.hawkbit_mgmt_url,
        "tenant": session.tenant,
    }))
}

/// Total number of rows for `endpoint` (optionally filtered by RSQL `q`).
///
/// `endpoint` must be a bare resource path: the query string is built here,
/// properly encoded — appending `?limit=1` to a path that already carried a
/// `?q=…` produced one malformed query and silently zeroed every dashboard
/// counter.
pub async fn count_resource(
    state: &AppState,
    session: &Session,
    endpoint: &str,
    q: Option<&str>,
) -> Value {
    let mut params = vec![
        ("limit".to_string(), "1".to_string()),
        ("representation".to_string(), "compact".to_string()),
    ];
    if let Some(q) = q.filter(|value| !value.trim().is_empty()) {
        params.push(("q".to_string(), q.to_string()));
    }
    let query = serde_urlencoded::to_string(params).unwrap_or_default();
    let url = format!(
        "{}/rest/v1/{endpoint}?{query}",
        state.config.hawkbit_mgmt_url
    );
    match state
        .client
        .get(url)
        .headers(session::auth_headers(session))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            match response.json::<Value>().await {
                Ok(value) => json!({
                    "ok": status.is_success(),
                    "status": status.as_u16(),
                    "count": value.pointer("/page/totalElements").or_else(|| value.pointer("/total")).cloned(),
                }),
                Err(_) => json!({ "ok": false, "status": status.as_u16(), "count": null }),
            }
        }
        Err(error) => json!({ "ok": false, "error": error.to_string(), "count": null }),
    }
}

pub async fn fetch_page(
    state: &AppState,
    session: &Session,
    endpoint: &str,
    q: Option<&str>,
    sort: Option<&str>,
    page: usize,
    limit: usize,
) -> Result<Value, AppError> {
    let offset = (page.saturating_sub(1)) * limit;
    let mut params = vec![
        ("limit".to_string(), limit.to_string()),
        ("offset".to_string(), offset.to_string()),
        ("representation".to_string(), "compact".to_string()),
    ];
    if let Some(q) = q.filter(|value| !value.trim().is_empty()) {
        params.push(("q".to_string(), q.to_string()));
    }
    if let Some(sort) = sort.filter(|value| !value.trim().is_empty()) {
        params.push(("sort".to_string(), sort.to_string()));
    }
    let query = serde_urlencoded::to_string(params).unwrap_or_default();
    get_json(state, session, &format!("{endpoint}?{query}")).await
}

pub async fn get_json(state: &AppState, session: &Session, path: &str) -> Result<Value, AppError> {
    let response = state
        .client
        .get(format!(
            "{}/rest/v1/{}",
            state.config.hawkbit_mgmt_url, path
        ))
        .headers(session::auth_headers(session))
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;
    response_json_or_error(response).await
}

pub async fn send_json(
    state: &AppState,
    session: &Session,
    method: Method,
    path: &str,
    payload: Value,
) -> Result<Value, AppError> {
    let response = state
        .client
        .request(
            method,
            format!("{}/rest/v1/{}", state.config.hawkbit_mgmt_url, path),
        )
        .headers(session::auth_headers(session))
        .json(&payload)
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;
    response_json_or_error(response).await
}

pub async fn send_empty(
    state: &AppState,
    session: &Session,
    method: Method,
    path: &str,
) -> Result<Value, AppError> {
    let response = state
        .client
        .request(
            method,
            format!("{}/rest/v1/{}", state.config.hawkbit_mgmt_url, path),
        )
        .headers(session::auth_headers(session))
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;
    response_json_or_error(response).await
}

pub async fn proxy(
    state: &AppState,
    session: &Session,
    proxy: ProxyRequest,
) -> Result<Response, AppError> {
    let mut url = format!("{}/rest/v1/{}", state.config.hawkbit_mgmt_url, proxy.path);
    if let Some(query_string) = proxy.uri.query() {
        url.push('?');
        url.push_str(query_string);
    } else if !proxy.query.is_empty() {
        let query_string = serde_urlencoded::to_string(proxy.query)
            .map_err(|_| AppError::bad_request("invalid query string"))?;
        url.push('?');
        url.push_str(&query_string);
    }

    let mut request = state
        .client
        .request(proxy.method.clone(), url)
        .headers(session::auth_headers(session));
    if let Some(accept) = proxy.headers.get(ACCEPT) {
        request = request.header(ACCEPT, accept);
    }
    if let Some(content_type) = proxy.headers.get(CONTENT_TYPE) {
        request = request.header(CONTENT_TYPE, content_type);
    }
    // A browser resuming an interrupted download sends this; dropping it
    // silently turned every resume into a fresh full transfer.
    if let Some(range) = proxy.headers.get(http::header::RANGE) {
        request = request.header(http::header::RANGE, range);
    }
    if !proxy.body.is_empty() && proxy.method != Method::GET && proxy.method != Method::HEAD {
        request = request.body(proxy.body);
    }

    let upstream = request
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;
    proxy_response(upstream).await
}

async fn response_json_or_error(response: reqwest::Response) -> Result<Value, AppError> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(AppError::upstream_unavailable)?;
    let value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
    };
    if status.is_success() {
        Ok(value)
    } else {
        Err(AppError {
            status,
            message: read_upstream_error(&value, status),
            detail: Some(value),
        })
    }
}

/// Hand an upstream answer to the browser without ever holding the body in
/// memory. Artifacts run to gigabytes, and buffering one would blow the
/// console's memory and break the streaming invariant.
///
/// The status is forwarded, not assumed: answering 200 to an upstream 206
/// would hand the browser a partial file labelled complete — a silently
/// corrupted firmware image.
async fn proxy_response(upstream: reqwest::Response) -> Result<Response, AppError> {
    use http::header::{
        ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED,
    };

    let status = upstream.status();
    let mut headers = axum::http::HeaderMap::new();
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_DISPOSITION,
        CONTENT_RANGE,
        ACCEPT_RANGES,
        ETAG,
        LAST_MODIFIED,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            headers.insert(name, value.clone());
        }
    }
    let body = axum::body::Body::from_stream(upstream.bytes_stream());
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

/// Stream one upstream GET straight through to the browser, carrying the
/// caller's `Range` up and the content headers back down.
pub async fn stream(
    state: &AppState,
    session: &Session,
    path: &str,
    range: Option<&axum::http::HeaderValue>,
) -> Result<Response, AppError> {
    let mut request = state
        .client
        .get(format!(
            "{}/rest/v1/{}",
            state.config.hawkbit_mgmt_url, path
        ))
        .headers(session::auth_headers(session));
    if let Some(range) = range {
        request = request.header(http::header::RANGE, range);
    }
    let upstream = request
        .send()
        .await
        .map_err(AppError::upstream_unavailable)?;
    if !upstream.status().is_success() && upstream.status() != StatusCode::PARTIAL_CONTENT {
        return Err(response_json_or_error(upstream).await.unwrap_err());
    }
    proxy_response(upstream).await
}

fn read_upstream_error(value: &Value, status: StatusCode) -> String {
    let mut message = value
        .get("error")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("hawier returned HTTP {}", status.as_u16()));
    // Correlate with the server incident journal (page /incidents): the id rides
    // along in the action log and any UI error message.
    if let Some(incident) = value.get("incident").and_then(Value::as_str) {
        message.push_str(&format!(" [incident={incident}]"));
    }
    message
}

/// Stream a browser-uploaded artifact into the mgmt multipart endpoint
/// without buffering the whole file in the UI backend: the borrowed axum
/// field is pumped through a channel while reqwest streams from it.
pub async fn upload_artifact(
    state: &AppState,
    session: &Session,
    sm_id: &str,
    filename: &str,
    mut field: axum::extract::multipart::Field<'_>,
) -> Result<Value, AppError> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    let pump = async {
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    // Poison the stream so reqwest ABORTS the request —
                    // otherwise mgmt would store a silently truncated file.
                    let _ = tx.send(Err(std::io::Error::other(error.to_string()))).await;
                    drop(tx);
                    return Err(AppError::bad_request(format!("upload read: {error}")));
                }
            }
        }
        drop(tx);
        Ok::<(), AppError>(())
    };

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let part = reqwest::multipart::Part::stream(reqwest::Body::wrap_stream(stream))
        .file_name(filename.to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let send = async {
        state
            .client
            .post(format!(
                "{}/rest/v1/softwaremodules/{}/artifacts",
                state.config.hawkbit_mgmt_url, sm_id
            ))
            .headers(session::auth_headers(session))
            .multipart(form)
            .send()
            .await
            .map_err(AppError::upstream_unavailable)
    };

    let (pumped, response) = tokio::join!(pump, send);
    pumped?;
    response_json_or_error(response?).await
}

/// Strip RSQL metacharacters from user input before embedding it in a
/// query: the needle is data, never syntax.
pub fn rsql_literal(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '*' | ',' | ';' | '(' | ')' | '=' | '"' | '\'' | '\\'))
        .collect()
}

pub fn read_items(value: &Value) -> Vec<Value> {
    if let Some(items) = value.get("content").and_then(Value::as_array) {
        return items.clone();
    }
    // Some sub-resources (e.g. a target's tags) answer with a bare array.
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    if let Some(object) = value.get("_embedded").and_then(Value::as_object) {
        if let Some(items) = object.values().find_map(Value::as_array) {
            return items.clone();
        }
    }
    Vec::new()
}

pub fn read_total(value: &Value) -> Option<usize> {
    value
        .pointer("/page/totalElements")
        .and_then(Value::as_u64)
        .or_else(|| value.get("total").and_then(Value::as_u64))
        .map(|value| value as usize)
}
