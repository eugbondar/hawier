//! Small pure helpers shared by the route handlers and the view models:
//! URL building, JSON path reading, timestamp formatting, static assets.
//!
//! No page markup lives here — see `apps/ui/templates` and [`crate::view`].

use axum::{
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::IntoResponse,
    response::Response,
};
use serde_json::Value;

use crate::{resources::Resource, state::AppState};

pub async fn static_asset(state: &AppState, path: &str) -> Response {
    let safe_path = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "..")
        .fold(
            state.config.static_dir.join("assets"),
            |mut acc, segment| {
                acc.push(segment);
                acc
            },
        );
    match tokio::fs::read(&safe_path).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&safe_path).first_or_octet_stream();
            let mut response = bytes.into_response();
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            response
        }
        Err(_) if state.config.dev_mode => (
            StatusCode::NOT_FOUND,
            format!("static asset not built: {}", safe_path.display()),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "static asset not found").into_response(),
    }
}

/// Field names whose value is an epoch-millisecond timestamp. Anything
/// matching also matches by suffix (`*At`, `*Date`, `*Time`) — see the
/// UI/UX rules in CLAUDE.md: no timestamp is ever printed as a raw number.
const TIME_FIELDS: [&str; 4] = ["forcetime", "starttime", "timestamp", "startat"];

pub fn is_time_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    TIME_FIELDS.contains(&lower.as_str())
        || lower.ends_with("at")
        || lower.ends_with("date")
        || lower.ends_with("time")
}

fn parts_of(epoch_millis: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(epoch_millis) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

/// `HH:MM:SS dd.mm.yy` — the console's one date format. Server-side text is
/// UTC so the page reads correctly without JS; `main.js` rewrites it into
/// the reader's timezone and leaves the exact stamp in the tooltip.
pub fn human_stamp(epoch_millis: i64) -> String {
    let at = parts_of(epoch_millis);
    format!(
        "{:02}:{:02}:{:02} {:02}.{:02}.{:02}",
        at.hour(),
        at.minute(),
        at.second(),
        at.day(),
        u8::from(at.month()),
        at.year() % 100,
    )
}

/// The value exactly as the API sent it — strings unquoted, everything
/// else as compact JSON. This is what a FIQL filter expects.
pub fn raw_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

pub fn read_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if let Some(object) = value.as_object() {
        if let Some(value) = object.get(path) {
            return Some(value);
        }
    }
    path.split('.').try_fold(value, |acc, key| acc.get(key))
}

/// Human label for a search hit: what the row is called, not its raw id.
pub fn row_label(resource: &Resource, value: &Value) -> String {
    let id = row_id(resource, value);
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty());
    match name {
        Some(name) if name == id => id,
        Some(name) => match value.get("version").and_then(Value::as_str) {
            Some(version) => format!("{name} {version} ({id})"),
            None => format!("{name} ({id})"),
        },
        None => id,
    }
}

pub fn row_id(resource: &Resource, value: &Value) -> String {
    read_path(value, &resource.id_field)
        .or_else(|| read_path(value, "controllerId"))
        .or_else(|| read_path(value, "id"))
        .or_else(|| read_path(value, "name"))
        .map(render_plain_value)
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn created_row_id(resource: &Resource, value: &Value) -> String {
    if let Some(first) = value.as_array().and_then(|items| items.first()) {
        return row_id(resource, first);
    }
    row_id(resource, value)
}

pub fn render_plain_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Object(object) => object
            .get("name")
            .or_else(|| object.get("id"))
            .or_else(|| object.get("controllerId"))
            .map(render_plain_value)
            .unwrap_or_else(|| "object".to_string()),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn next_sort(column: &str, current: Option<&str>) -> String {
    match current.and_then(|sort| sort.split_once(':')) {
        Some((field, "ASC")) if field == column => format!("{column}:DESC"),
        Some((field, "DESC")) if field == column => format!("{column}:ASC"),
        _ => format!("{column}:ASC"),
    }
}

pub fn list_href(
    resource: &Resource,
    query: &crate::routes::ListQuery,
    override_pair: Option<(&str, &str)>,
    columns: Option<&str>,
    page: Option<usize>,
) -> String {
    let mut params = Vec::new();
    if let Some(view) = query.view.as_deref() {
        params.push(("view", view.to_string()));
    }
    if let Some(q) = query.q.as_deref() {
        params.push(("q", q.to_string()));
    }
    if let Some(sort) = query.sort.as_deref() {
        params.push(("sort", sort.to_string()));
    }
    if let Some(columns) = columns.or(query.columns.as_deref()) {
        params.push(("columns", columns.to_string()));
    }
    if let Some(limit) = query.limit {
        params.push(("limit", limit.to_string()));
    }
    if let Some(page) = page {
        params.push(("page", page.to_string()));
    }
    if let Some((key, value)) = override_pair {
        params.retain(|(existing, _)| *existing != key);
        params.push((key, value.to_string()));
    }
    let query = params
        .into_iter()
        .map(|(key, value)| format!("{}={}", key, encode_component(&value)))
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        format!("/{}", resource.id)
    } else {
        format!("/{}?{}", resource.id, query)
    }
}

/// Where a row of this resource leads. Normally its own object page; a
/// resource that declares `row_href` points at a filtered list instead.
///
/// The value is quoted because the RSQL parser reads a bare value up to the
/// next `,;()` and trims it — a group called `eu,west` or one with a trailing
/// space would otherwise produce a link whose total disagrees with the count
/// the row just showed.
pub fn row_href(resource: &Resource, row: &Value) -> String {
    let id = row_id(resource, row);
    match resource.row_href {
        Some(spec) => {
            let value = read_path(row, spec.column)
                .map(render_plain_value)
                .unwrap_or_else(|| id.clone());
            filtered_list_href(spec.resource, spec.field, &value)
        }
        None => format!("/{}/{}", resource.id, encode_path(&id)),
    }
}

/// `/{resource}?q={field}=="{value}"`, quoted and percent-encoded. Every
/// link to a filtered list goes through here — building one by hand in a
/// template is how the group link on a target card ended up unquoted.
pub fn filtered_list_href(resource: &str, field: &str, value: &str) -> String {
    format!(
        "/{resource}?q={}",
        encode_component(&format!("{field}==\"{}\"", rsql_quote(value)))
    )
}

/// Escape what a double-quoted RSQL literal cannot hold verbatim.
fn rsql_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn encode_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

pub fn encode_path(value: &str) -> String {
    encode_component(value).replace('+', "%20")
}
