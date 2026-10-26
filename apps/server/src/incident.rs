//! Incident numbers: one decimal snowflake per request. Minted for EVERY
//! request and returned in `X-Request-Id`; when the request fails, the same
//! number becomes the incident id — it keys the `hawier_incident` journal
//! row and appears as `"incident"` in the error body. Layout (i64, top bit
//! zero): 41 bits milliseconds since [`HAWIER_EPOCH_MS`] | 10 bits node id
//! | 12 bits per-millisecond sequence — the number itself tells when and
//! where it was born.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// 2026-01-01T00:00:00Z — hawier's snowflake epoch.
pub const HAWIER_EPOCH_MS: i64 = 1_767_225_600_000;

const NODE_BITS: u32 = 10;
const SEQ_BITS: u32 = 12;
pub const MAX_NODE_ID: u16 = (1 << NODE_BITS) - 1;
const SEQ_MASK: u64 = (1 << SEQ_BITS) - 1;

/// Decoded parts of an incident number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncidentParts {
    /// Unix epoch millis (not hawier-epoch).
    pub timestamp_ms: i64,
    pub node: u16,
    pub sequence: u16,
}

pub fn decode(id: i64) -> IncidentParts {
    let raw = id as u64;
    IncidentParts {
        timestamp_ms: ((raw >> (NODE_BITS + SEQ_BITS)) as i64) + HAWIER_EPOCH_MS,
        node: ((raw >> SEQ_BITS) as u16) & MAX_NODE_ID,
        sequence: (raw & SEQ_MASK) as u16,
    }
}

/// Lock-free snowflake generator: one per node process.
pub struct IncidentGen {
    node: u64,
    /// Packed `last_ms << SEQ_BITS | seq` — one CAS claims a slot.
    state: AtomicU64,
}

impl IncidentGen {
    pub fn new(node_id: u16) -> Self {
        Self {
            node: u64::from(node_id & MAX_NODE_ID) << SEQ_BITS,
            state: AtomicU64::new(0),
        }
    }

    pub fn next(&self) -> i64 {
        loop {
            let now_ms = (crate::state::now_millis() - HAWIER_EPOCH_MS).max(0) as u64;
            let current = self.state.load(Ordering::Relaxed);
            let (last_ms, seq) = (current >> SEQ_BITS, current & SEQ_MASK);
            let candidate = if now_ms > last_ms {
                now_ms << SEQ_BITS
            } else if seq < SEQ_MASK {
                // Same (or clock-skewed earlier) millisecond: bump the
                // sequence, keeping ids monotonic on last_ms.
                (last_ms << SEQ_BITS) | (seq + 1)
            } else {
                // 4096 ids in one ms exhausted — wait for the next tick.
                std::hint::spin_loop();
                continue;
            };
            if self
                .state
                .compare_exchange(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let (ms, seq) = (candidate >> SEQ_BITS, candidate & SEQ_MASK);
                return ((ms << (NODE_BITS + SEQ_BITS)) | self.node | seq) as i64;
            }
        }
    }
}

/// Mint an incident number for every request: overwrite `x-request-id` on
/// the request (client-supplied values are never trusted — the number must
/// be unique, it is a DB primary key on errors) and echo it on the response.
pub async fn assign_incident_id(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let id = state.incidents.next();
    let value = HeaderValue::from_str(&id.to_string()).expect("decimal is a valid header value");
    req.headers_mut().insert("x-request-id", value.clone());
    let mut response = next.run(req).await;
    response.headers_mut().insert("x-request-id", value);
    response
}

/// Default node id when `HAWIER_NODE_ID` is not set: stable hostname hash.
pub fn default_node_id() -> u16 {
    use std::hash::{Hash, Hasher};
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| {
        std::fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "hawier".to_string())
            .trim()
            .to_string()
    });
    let mut hasher = std::hash::DefaultHasher::new();
    hostname.hash(&mut hasher);
    (hasher.finish() as u16) & MAX_NODE_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_decode() {
        let generator = IncidentGen::new(42);
        let id = generator.next();
        assert!(id > 0);
        let parts = decode(id);
        assert_eq!(parts.node, 42);
        let now = crate::state::now_millis();
        assert!(
            (parts.timestamp_ms - now).abs() < 5_000,
            "timestamp {} vs now {now}",
            parts.timestamp_ms
        );
    }

    #[test]
    fn ids_are_unique_and_monotonic_under_burst() {
        let generator = IncidentGen::new(1);
        let mut previous = 0;
        for _ in 0..10_000 {
            let id = generator.next();
            assert!(id > previous, "{id} after {previous}");
            previous = id;
        }
    }

    #[test]
    fn unique_across_threads() {
        use std::collections::HashSet;
        use std::sync::Arc;
        let generator = Arc::new(IncidentGen::new(7));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let generator = generator.clone();
                std::thread::spawn(move || (0..5_000).map(|_| generator.next()).collect::<Vec<_>>())
            })
            .collect();
        let mut seen = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                assert!(seen.insert(id), "duplicate id {id}");
            }
        }
    }
}
