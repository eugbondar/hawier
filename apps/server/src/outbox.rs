//! Control-node outbox drainer: takes committed domain events and performs
//! the deliveries their sinks describe.
//!
//! Split from emission on purpose (`events.rs` writes, this delivers). The
//! writer must never wait on the network, and the deliverer must never hold
//! a lock while it does.
//!
//! Claiming is one atomic `UPDATE ... FOR UPDATE SKIP LOCKED`, so several
//! control nodes can drain the same table without coordinating and without
//! either of them holding a row lock across an HTTP call.

use std::time::Duration;

use hawier_api::ext::Delivery;
use sqlx::Row;

use crate::error::AppResult;
use crate::events;
use crate::state::AppState;

/// How often the drainer looks for due events.
const TICK: Duration = Duration::from_secs(2);
/// Rows taken per tick. Small: a burst should spread over ticks rather than
/// open hundreds of sockets at once.
const BATCH: i64 = 32;
/// After this many failed attempts an event is dead and stops being retried.
const MAX_ATTEMPTS: i32 = 8;
/// A claimed row whose worker died is returned to the queue after this long.
const CLAIM_TIMEOUT_SECS: i64 = 300;
/// Per-delivery HTTP timeout: a subscriber that hangs must not stall the
/// queue behind it.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let client = match reqwest::Client::builder().timeout(DELIVERY_TIMEOUT).build() {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(%error, "outbox drainer could not build an HTTP client");
                return;
            }
        };
        loop {
            ticker.tick().await;
            if let Err(error) = tick(&state, &client).await {
                tracing::warn!(%error, "outbox tick failed");
            }
        }
    });
}

pub async fn tick(state: &AppState, client: &reqwest::Client) -> AppResult<()> {
    release_stale_claims(state).await?;

    // Claim and mark in one statement: nothing is held while we talk to the
    // network, and a second drainer skips what this one took.
    let claimed = sqlx::query(
        "UPDATE hawier_event_outbox SET state = 'sending', attempts = attempts + 1
         WHERE id IN (
             SELECT id FROM hawier_event_outbox
             WHERE state = 'pending' AND next_attempt_at <= now()
             ORDER BY id LIMIT $1
             FOR UPDATE SKIP LOCKED
         )
         RETURNING id, tenant, kind, payload, attempts",
    )
    .bind(BATCH)
    .fetch_all(&state.pool)
    .await?;

    for row in claimed {
        let id: i64 = row.get("id");
        let attempts: i32 = row.get("attempts");
        let event = events::event_from_row(&row);

        // A sink that is configured but will not answer is a failure: the
        // event goes back in the queue rather than being quietly discarded.
        let planned = match events::plan(state, &event).await {
            Ok(Some(planned)) => planned,
            // No sink configured — nothing to deliver, and nothing wrong.
            Ok(None) => {
                finish(state, id).await?;
                continue;
            }
            Err(error) => {
                retry_later(state, id, attempts, &error.to_string()).await?;
                continue;
            }
        };

        let mut failure: Option<String> = None;
        for plan in &planned {
            let outcome = perform(client, &plan.delivery).await;
            let subscription = plan.subscription_id.parse::<i64>().unwrap_or_default();
            record_attempt(state, &event, subscription, &outcome).await?;
            if let Err(message) = outcome {
                failure = Some(message);
            }
        }

        match failure {
            None => finish(state, id).await?,
            Some(message) => retry_later(state, id, attempts, &message).await?,
        }
    }
    Ok(())
}

/// Outcome of one HTTP call: the status on success, a message on failure.
type Outcome = Result<u16, String>;

async fn perform(client: &reqwest::Client, delivery: &Delivery) -> Outcome {
    let method = reqwest::Method::from_bytes(delivery.method.as_bytes())
        .map_err(|_| format!("unsupported method '{}'", delivery.method))?;
    let mut request = client.request(method, &delivery.url);
    for (name, value) in &delivery.headers {
        request = request.header(name, value);
    }
    let response = request
        .body(delivery.body.clone())
        .send()
        .await
        .map_err(|e| format!("{e}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("subscriber answered {status}"))
    }
}

async fn record_attempt(
    state: &AppState,
    event: &hawier_api::ext::DomainEvent,
    webhook_id: i64,
    outcome: &Outcome,
) -> AppResult<()> {
    let (status, error) = match outcome {
        Ok(status) => (i32::from(*status), String::new()),
        Err(message) => (0, message.clone()),
    };
    sqlx::query(
        "INSERT INTO hawier_webhook_delivery
         (id, tenant, webhook, event, kind, status, error)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(state.incidents.next())
    .bind(&event.tenant)
    .bind(webhook_id)
    .bind(event.id.parse::<i64>().unwrap_or_default())
    .bind(&event.kind)
    .bind(status)
    .bind(error)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn finish(state: &AppState, id: i64) -> AppResult<()> {
    sqlx::query(
        "UPDATE hawier_event_outbox SET state = 'delivered', last_error = '' WHERE id = $1",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Exponential backoff, then give up. A dead event stays in the table: a
/// subscription that silently stopped working must be visible afterwards,
/// not swept away.
async fn retry_later(state: &AppState, id: i64, attempts: i32, message: &str) -> AppResult<()> {
    if attempts >= MAX_ATTEMPTS {
        sqlx::query("UPDATE hawier_event_outbox SET state = 'dead', last_error = $2 WHERE id = $1")
            .bind(id)
            .bind(message)
            .execute(&state.pool)
            .await?;
        tracing::warn!(
            event_id = id,
            attempts,
            error = message,
            "event delivery gave up"
        );
        return Ok(());
    }
    let delay_secs = 30_i64 * 2_i64.pow(attempts.max(1) as u32 - 1);
    sqlx::query(
        "UPDATE hawier_event_outbox
         SET state = 'pending', last_error = $2,
             next_attempt_at = now() + make_interval(secs => $3)
         WHERE id = $1",
    )
    .bind(id)
    .bind(message)
    .bind(delay_secs.min(3600) as f64)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// A drainer that died mid-flight leaves rows marked `sending`. Nothing else
/// would ever pick those up, so time them out back into the queue.
async fn release_stale_claims(state: &AppState) -> AppResult<()> {
    sqlx::query(
        "UPDATE hawier_event_outbox SET state = 'pending'
         WHERE state = 'sending' AND next_attempt_at < now() - make_interval(secs => $1)",
    )
    .bind(CLAIM_TIMEOUT_SECS as f64)
    .execute(&state.pool)
    .await?;
    Ok(())
}
