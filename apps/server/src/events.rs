//! Domain events: emission into the outbox, and asking the sink extension
//! what to send. Delivery itself lives in `outbox.rs`.
//!
//! Emission is a **transactional outbox**: the row is written in the same
//! transaction as the change it describes. That is the only arrangement in
//! which "the rollout started" and "somebody was told the rollout started"
//! cannot disagree — a rolled-back change takes its event with it, and a
//! committed change cannot lose its notification to a failed HTTP call.
//!
//! Emitting is therefore cheap and never talks to the network. It is also
//! safe on a sub node: the row simply waits until a control node drains it.
//!
//! Deciding what to send is not hawier's business. The host reads the
//! subscriptions — it owns the database, an extension never does — and hands
//! them to the sink extension with the event. The extension answers with
//! descriptions of the requests to make: which URL, which headers, which
//! signature. The host then performs them. Filtering rules and signing
//! schemes live entirely in that other process.

use hawier_api::ext::{points, DomainEvent, PlannedDelivery, SinkPlan, Subscription};
use serde_json::Value;
use sqlx::{PgConnection, Row};

use crate::error::AppResult;
use crate::ext;
use crate::incident::IncidentGen;
use crate::state::{now_millis, AppState};

/// Write an event in the caller's transaction.
///
/// Takes a connection rather than the whole state because most call sites
/// are mid-transaction in the rollout engine or a mgmt handler; whatever
/// they are doing, the event must share their fate.
pub async fn emit(
    tx: &mut PgConnection,
    ids: &IncidentGen,
    tenant: &str,
    kind: &str,
    subject: Value,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO hawier_event_outbox (id, tenant, kind, payload)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(ids.next())
    .bind(tenant)
    .bind(kind)
    .bind(subject)
    .execute(tx)
    .await?;
    Ok(())
}

/// Ask the sink extension what to send for this event.
///
/// `None` — no sink configured — means there is nothing to deliver, which is
/// not the same as a failure: the event is done. An extension that is
/// configured but unreachable *is* a failure, and surfaces as one so the
/// event is retried rather than silently dropped.
pub async fn plan(
    state: &AppState,
    event: &DomainEvent,
) -> AppResult<Option<Vec<PlannedDelivery>>> {
    let Some(extension) = state.ext.event_sink.as_deref() else {
        return Ok(None);
    };
    let subscriptions = load_subscriptions(&state.pool, &event.tenant).await?;
    if subscriptions.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let planned = ext::call(
        extension,
        points::EVENT_SINK_PLAN,
        &SinkPlan {
            event: event.clone(),
            subscriptions,
        },
    )
    .await?;
    Ok(Some(planned))
}

async fn load_subscriptions(pool: &sqlx::PgPool, tenant: &str) -> AppResult<Vec<Subscription>> {
    let rows = sqlx::query(
        "SELECT id, url, secret, event_mask FROM hawier_webhook
         WHERE UPPER(tenant) = UPPER($1) AND active = TRUE ORDER BY id",
    )
    .bind(tenant)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Subscription {
            id: row.get::<i64, _>("id").to_string(),
            url: row.get("url"),
            secret: row.get("secret"),
            event_mask: row.get("event_mask"),
        })
        .collect())
}

/// Build the event an outbox row describes.
pub fn event_from_row(row: &sqlx::postgres::PgRow) -> DomainEvent {
    DomainEvent {
        id: row.get::<i64, _>("id").to_string(),
        kind: row.get("kind"),
        tenant: row.get("tenant"),
        occurred_at: now_millis(),
        subject: row.get("payload"),
    }
}
