//! hawier event-sink extension: HTTP webhooks.
//!
//! Decides *what to send* — which subscriptions an event concerns, what the
//! body is, which headers carry the signature — and answers with a
//! description of each request. hawier performs them.
//!
//! Runs as a wasm module with no imports beyond the ABI: it opens no socket,
//! reads no file, and the subscriber secrets it signs with never leave it.
//! Filtering and the signature scheme are local decisions here — changing
//! either means shipping a new version of this module, not of hawier.

use hawier_api::ext::{
    hmac_sha256, points, Delivery, DomainEvent, ExecutionSite, ExtManifest, ExtRequest,
    ExtResponse, PlannedDelivery, SinkPlan, Subscription,
};

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-webhook".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::EVENT_SINK_PLAN.to_string()],
        // Events are drained by the control node only.
        sites: vec![ExecutionSite::Control],
        // None: it describes requests, it does not make them.
        capabilities: Vec::new(),
        deadline_ms: 1_000,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::EVENT_SINK_PLAN {
        return ExtResponse::Err {
            message: format!("hawier-ext-webhook does not implement {}", request.point),
        };
    }
    let plan = match serde_json::from_value::<SinkPlan>(request.payload) {
        Ok(plan) => plan,
        Err(error) => {
            return ExtResponse::Err {
                message: format!("unusable event_sink.plan payload: {error}"),
            }
        }
    };
    let body = match serde_json::to_string(&plan.event) {
        Ok(body) => body,
        Err(error) => {
            return ExtResponse::Err {
                message: format!("cannot serialise the event: {error}"),
            }
        }
    };
    let planned = plan_deliveries(&plan.event, &body, &plan.subscriptions);
    match serde_json::to_value(planned) {
        Ok(payload) => ExtResponse::Ok { payload },
        Err(error) => ExtResponse::Err {
            message: format!("cannot encode the plan: {error}"),
        },
    }
}

/// Does a subscription's mask cover this kind? `*` (or empty) is everything;
/// otherwise a comma-separated list of exact kinds.
fn mask_covers(mask: &str, kind: &str) -> bool {
    let mask = mask.trim();
    if mask.is_empty() || mask == "*" {
        return true;
    }
    mask.split(',').any(|entry| entry.trim() == kind)
}

fn plan_deliveries(
    event: &DomainEvent,
    body: &str,
    subscriptions: &[Subscription],
) -> Vec<PlannedDelivery> {
    subscriptions
        .iter()
        .filter(|subscription| mask_covers(&subscription.event_mask, &event.kind))
        .map(|subscription| {
            let mut headers = vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("x-hawier-event".to_string(), event.kind.clone()),
                ("x-hawier-delivery".to_string(), event.id.clone()),
            ];
            if !subscription.secret.is_empty() {
                // The receiver recomputes this over the raw body to know the
                // call came from us and was not replayed with edits.
                headers.push((
                    "x-hawier-signature".to_string(),
                    format!(
                        "sha256={}",
                        hex::encode(hmac_sha256(subscription.secret.as_bytes(), body.as_bytes()))
                    ),
                ));
            }
            PlannedDelivery {
                subscription_id: subscription.id.clone(),
                delivery: Delivery {
                    url: subscription.url.clone(),
                    method: "POST".to_string(),
                    headers,
                    body: body.to_string(),
                },
            }
        })
        .collect()
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> DomainEvent {
        DomainEvent {
            id: "123".to_string(),
            kind: "rollout.started".to_string(),
            tenant: "DEFAULT".to_string(),
            occurred_at: 0,
            subject: serde_json::json!({}),
        }
    }

    fn subscription(mask: &str, secret: &str) -> Subscription {
        Subscription {
            id: "7".to_string(),
            url: "https://example/hook".to_string(),
            secret: secret.to_string(),
            event_mask: mask.to_string(),
        }
    }

    #[test]
    fn a_star_mask_covers_everything() {
        assert!(mask_covers("*", "rollout.started"));
        assert!(mask_covers("", "rollout.started"));
    }

    #[test]
    fn a_listed_mask_covers_only_its_kinds() {
        let mask = "rollout.started, artifact.uploaded";
        assert!(mask_covers(mask, "rollout.started"));
        assert!(mask_covers(mask, "artifact.uploaded"));
        assert!(!mask_covers(mask, "rollout.finished"));
    }

    #[test]
    fn a_mask_does_not_match_on_prefixes() {
        // "rollout." must not quietly subscribe to every rollout event:
        // masks are exact kinds, and a prefix would be a silent widening.
        assert!(!mask_covers("rollout.", "rollout.started"));
        assert!(!mask_covers("rollout.start", "rollout.started"));
    }

    #[test]
    fn an_unmatched_subscription_gets_no_delivery() {
        let planned = plan_deliveries(&event(), "{}", &[subscription("artifact.deleted", "")]);
        assert!(planned.is_empty());
    }

    #[test]
    fn a_secret_produces_a_signature_header() {
        let planned = plan_deliveries(&event(), "{}", &[subscription("*", "s3cret")]);
        assert_eq!(planned.len(), 1);
        let signature = planned[0]
            .delivery
            .headers
            .iter()
            .find(|(name, _)| name == "x-hawier-signature")
            .map(|(_, value)| value.clone())
            .expect("signed");
        assert_eq!(
            signature,
            format!("sha256={}", hex::encode(hmac_sha256(b"s3cret", b"{}")))
        );
    }

    #[test]
    fn no_secret_means_no_signature_header() {
        let planned = plan_deliveries(&event(), "{}", &[subscription("*", "")]);
        assert_eq!(planned.len(), 1);
        assert!(planned[0]
            .delivery
            .headers
            .iter()
            .all(|(name, _)| name != "x-hawier-signature"));
    }
}
