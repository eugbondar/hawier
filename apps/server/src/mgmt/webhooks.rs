//! `/rest/v1/webhooks` — who gets told when something happens, and
//! `/rest/v1/webhookdeliveries` — what actually went out.
//!
//! hawier extension (recorded in API_NOTES): hawkBit has no such resource.
//! Ordinary resources rather than a bespoke page, so subscriptions get
//! sorting, paging, search and saved views for free (UI/UX rule 11).
//!
//! The `secret` is write-only. It is the key a subscriber verifies the HMAC
//! with, so it goes in and never comes back out — the list shows only
//! whether one is set.

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hawier_api::ext::kinds;
use serde_json::{json, Value};
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::{order_by, paged, rsql_filter, ListParams};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/webhooks", get(list).post(create))
        .route(
            "/rest/v1/webhooks/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route("/rest/v1/webhookdeliveries", get(list_deliveries))
        .route("/rest/v1/webhooks/{id}/deliveries", post(no_write))
}

/// The `{id}/deliveries` path exists only so a mistaken POST gets a clear
/// answer instead of a 405 from the router.
async fn no_write() -> AppResult<Json<Value>> {
    Err(AppError::BadRequest(
        "deliveries are written by the node, not by clients".to_string(),
    ))
}

static FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "w.id::text",
                kind: Kind::Text,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "w.name",
                kind: Kind::Text,
            },
        ),
        (
            "url",
            Field::Column {
                sql: "w.url",
                kind: Kind::Text,
            },
        ),
        (
            "eventmask",
            Field::Column {
                sql: "w.event_mask",
                kind: Kind::Text,
            },
        ),
        (
            "active",
            Field::Column {
                sql: "w.active",
                kind: Kind::Bool,
            },
        ),
        (
            // Rendered as a column, so it has to be sortable and filterable
            // like any other (UI/UX rule 25).
            "signed",
            Field::Column {
                sql: "(w.secret <> '')",
                kind: Kind::Bool,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM w.created_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

static DELIVERY_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "d.id::text",
                kind: Kind::Text,
            },
        ),
        (
            "webhook",
            Field::Column {
                sql: "d.webhook::text",
                kind: Kind::Text,
            },
        ),
        (
            "webhookname",
            Field::Column {
                sql: "w.name",
                kind: Kind::Text,
            },
        ),
        (
            "kind",
            Field::Column {
                sql: "d.kind",
                kind: Kind::Text,
            },
        ),
        (
            "event",
            Field::Column {
                sql: "d.event::text",
                kind: Kind::Text,
            },
        ),
        (
            "status",
            Field::Column {
                sql: "d.status",
                kind: Kind::Int,
            },
        ),
        (
            "error",
            Field::Column {
                sql: "d.error",
                kind: Kind::Text,
            },
        ),
        (
            "deliveredat",
            Field::Column {
                sql: "(EXTRACT(EPOCH FROM d.delivered_at) * 1000)::BIGINT",
                kind: Kind::Int,
            },
        ),
    ])
});

const SELECT: &str = "SELECT w.id, w.name, w.url, w.event_mask, w.active,
       w.secret <> '' AS signed,
       (EXTRACT(EPOCH FROM w.created_at) * 1000)::BIGINT AS created_ms,
       (EXTRACT(EPOCH FROM w.updated_at) * 1000)::BIGINT AS updated_ms
FROM hawier_webhook w";

const DELIVERY_SELECT: &str = "SELECT d.id, d.webhook, w.name AS webhook_name, d.event, d.kind,
       d.status, d.error, d.duration_ms,
       (EXTRACT(EPOCH FROM d.delivered_at) * 1000)::BIGINT AS delivered_ms
FROM hawier_webhook_delivery d JOIN hawier_webhook w ON w.id = d.webhook";

fn webhook_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "name": row.get::<String, _>("name"),
        "url": row.get::<String, _>("url"),
        "eventMask": row.get::<String, _>("event_mask"),
        "active": row.get::<bool, _>("active"),
        // Never the secret itself — only whether deliveries are signed.
        "signed": row.get::<bool, _>("signed"),
        "createdAt": row.get::<Option<i64>, _>("created_ms"),
        "updatedAt": row.get::<Option<i64>, _>("updated_ms"),
    })
}

fn delivery_json(row: &PgRow) -> Value {
    json!({
        "id": row.get::<i64, _>("id").to_string(),
        "webhook": row.get::<i64, _>("webhook").to_string(),
        "webhookName": row.get::<String, _>("webhook_name"),
        "event": row.get::<i64, _>("event").to_string(),
        "kind": row.get::<String, _>("kind"),
        "status": row.get::<i32, _>("status"),
        "error": row.get::<String, _>("error"),
        "durationMs": row.get::<i32, _>("duration_ms"),
        "deliveredAt": row.get::<Option<i64>, _>("delivered_ms"),
    })
}

/// `*` or a comma-separated list of kinds hawier actually emits. A mask
/// naming something that is never emitted is a subscription that will look
/// healthy and stay silent, so it is refused here rather than debugged later.
fn validate_mask(mask: &str) -> AppResult<String> {
    let mask = mask.trim();
    if mask.is_empty() || mask == "*" {
        return Ok("*".to_string());
    }
    let mut cleaned = Vec::new();
    for entry in mask.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if !kinds::ALL.contains(&entry) {
            return Err(AppError::BadRequest(format!(
                "unknown event kind '{entry}'; known kinds are {}",
                kinds::ALL.join(", ")
            )));
        }
        cleaned.push(entry.to_string());
    }
    if cleaned.is_empty() {
        return Ok("*".to_string());
    }
    Ok(cleaned.join(","))
}

fn required_str(body: &Value, key: &str) -> AppResult<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppError::BadRequest(format!("'{key}' is required")))
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &FIELDS, "w.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_webhook w
             WHERE UPPER(w.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SELECT} WHERE UPPER(w.tenant) = UPPER($1){filter_sql} {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(rows.iter().map(webhook_json).collect(), total))
}

async fn list_deliveries(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &DELIVERY_FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &DELIVERY_FIELDS, "d.id DESC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM hawier_webhook_delivery d
             JOIN hawier_webhook w ON w.id = d.webhook
             WHERE UPPER(d.tenant) = UPPER($1){filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{DELIVERY_SELECT} WHERE UPPER(d.tenant) = UPPER($1){filter_sql} {order} \
             LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(rows.iter().map(delivery_json).collect(), total))
}

async fn load(state: &AppState, tenant: &str, id: i64) -> AppResult<PgRow> {
    sqlx::query(&format!(
        "{SELECT} WHERE w.id = $2 AND UPPER(w.tenant) = UPPER($1)"
    ))
    .bind(tenant)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("webhook {id} not found")))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    Ok(Json(webhook_json(
        &load(&state, &identity.tenant, id).await?,
    )))
}

async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Json(body): Json<Value>,
) -> AppResult<(axum::http::StatusCode, Json<Value>)> {
    let name = required_str(&body, "name")?;
    let url = required_str(&body, "url")?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "'url' must be http or https".to_string(),
        ));
    }
    let mask = validate_mask(
        body.get("eventMask")
            .and_then(|v| v.as_str())
            .unwrap_or("*"),
    )?;
    let secret = body
        .get("secret")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let active = body.get("active").and_then(|v| v.as_bool()).unwrap_or(true);

    let id = state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_webhook (id, tenant, name, url, secret, event_mask, active)
         VALUES ($1, UPPER($2), $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(&identity.tenant)
    .bind(&name)
    .bind(&url)
    .bind(&secret)
    .bind(&mask)
    .bind(active)
    .execute(&state.pool)
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(webhook_json(&load(&state, &identity.tenant, id).await?)),
    ))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load(&state, &identity.tenant, id).await?;

    // Only what was sent is changed; an absent `secret` keeps the stored one
    // (it is never rendered back, so a round-trip cannot resend it).
    let mask = match body.get("eventMask").and_then(|v| v.as_str()) {
        Some(mask) => Some(validate_mask(mask)?),
        None => None,
    };
    sqlx::query(
        "UPDATE hawier_webhook SET
            name = COALESCE($3, name),
            url = COALESCE($4, url),
            event_mask = COALESCE($5, event_mask),
            active = COALESCE($6, active),
            secret = COALESCE($7, secret),
            updated_at = now()
         WHERE id = $2 AND UPPER(tenant) = UPPER($1)",
    )
    .bind(&identity.tenant)
    .bind(id)
    .bind(body.get("name").and_then(|v| v.as_str()))
    .bind(body.get("url").and_then(|v| v.as_str()))
    .bind(mask)
    .bind(body.get("active").and_then(|v| v.as_bool()))
    .bind(body.get("secret").and_then(|v| v.as_str()))
    .execute(&state.pool)
    .await?;

    Ok(Json(webhook_json(
        &load(&state, &identity.tenant, id).await?,
    )))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<axum::http::StatusCode> {
    let deleted =
        sqlx::query("DELETE FROM hawier_webhook WHERE id = $2 AND UPPER(tenant) = UPPER($1)")
            .bind(&identity.tenant)
            .bind(id)
            .execute(&state.pool)
            .await?
            .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound(format!("webhook {id} not found")));
    }
    Ok(axum::http::StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_mask_is_accepted_and_normalised() {
        assert_eq!(validate_mask("*").unwrap(), "*");
        assert_eq!(validate_mask("  ").unwrap(), "*");
    }

    #[test]
    fn known_kinds_are_kept_and_trimmed() {
        let mask = validate_mask(" rollout.started , artifact.uploaded ").unwrap();
        assert_eq!(mask, "rollout.started,artifact.uploaded");
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        // A mask nobody emits is a subscription that looks healthy and stays
        // silent — better a 400 now than a debugging session later.
        let error = validate_mask("rollout.exploded").unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)));
    }
}
