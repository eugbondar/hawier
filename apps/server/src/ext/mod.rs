//! The extension host: everything hawier delegates outside its own process.
//!
//! There is no "builtin" tier. A decision that belongs to an extension is
//! taken by an extension, in its own process, or it is not taken at all —
//! the host answers with the point's declared failure instead of quietly
//! doing the work itself. That is the difference between a mechanism and a
//! set of hooks nobody uses.
//!
//! What crosses the boundary is a decision or a descriptor: a URL, a path,
//! a request to send. Never bytes, never a database handle. The host keeps
//! the things an extension must not hold — transactions, locks, sockets,
//! the artifact stream — and does the IO the descriptor describes.
//!
//! Extensions are wasm modules, which is what makes that a fact rather than
//! a convention: a module has no ambient authority, so "it may not open a
//! socket" is enforced by the runtime and not by the author's good manners.

pub mod params;
pub mod wasm;

use std::sync::Arc;

use hawier_api::ext::{points, ExtRequest};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::Config;
use crate::error::{AppError, AppResult};
use wasm::Extension;

/// One configurable slot: a module the node loads, and the point or points
/// it answers.
///
/// Six slots for seven points — `rollout.plan` and `rollout.evaluate` are two
/// questions for one strategy module, so they share a slot. An inventory that
/// listed points would show a module twice; one that listed slots and named
/// their points shows what an operator actually configures.
pub struct Slot {
    /// Stable key, used by the API and the console.
    pub key: &'static str,
    pub label: &'static str,
    /// The variable that points this slot at a module. The console cannot
    /// change it — see `params::RESERVED` — so it tells the operator what to
    /// change instead.
    pub env_var: &'static str,
    pub points: &'static [&'static str],
    /// What happens when nothing is configured here.
    ///
    /// Taken from the call site's own behaviour rather than invented for the
    /// page: an absent point has a declared consequence, and the console
    /// should repeat it, not paraphrase it.
    pub absence: &'static str,
}

pub const SLOTS: [Slot; 6] = [
    Slot {
        key: "storage",
        label: "Artifact storage",
        env_var: "HAWIER_EXT_STORAGE",
        points: &[points::STORAGE_LOCATE],
        absence: "artifact download and upload answer 503 with Retry-After",
    },
    Slot {
        key: "event_sink",
        label: "Event sink",
        env_var: "HAWIER_EXT_EVENT_SINK",
        points: &[points::EVENT_SINK_PLAN],
        absence: "domain events accumulate in the outbox and are never delivered",
    },
    Slot {
        key: "retention",
        label: "Retention",
        env_var: "HAWIER_EXT_RETENTION",
        points: &[points::RETENTION_SELECT],
        absence: "retention rules are recorded and never fire",
    },
    Slot {
        key: "rollout",
        label: "Rollout strategy",
        env_var: "HAWIER_EXT_ROLLOUT",
        points: &[points::ROLLOUT_PLAN, points::ROLLOUT_EVALUATE],
        absence: "rollouts cannot be divided into groups at all",
    },
    Slot {
        key: "console",
        label: "Console surfaces",
        env_var: "HAWIER_EXT_CONSOLE",
        points: &[points::CONSOLE_SURFACES],
        absence: "the console shows only its built-in tree",
    },
    Slot {
        key: "authz",
        label: "Authorisation",
        env_var: "HAWIER_EXT_AUTHZ",
        points: &[points::AUTHZ_DECIDE],
        absence: "every authenticated request is allowed",
    },
];

impl ExtRegistry {
    /// The module in a slot, by key.
    pub fn slot(&self, key: &str) -> Option<&Arc<Extension>> {
        match key {
            "storage" => self.storage.as_ref(),
            "event_sink" => self.event_sink.as_ref(),
            "retention" => self.retention.as_ref(),
            "rollout" => self.rollout.as_ref(),
            "console" => self.console.as_ref(),
            "authz" => self.authz.as_ref(),
            _ => None,
        }
    }
}

/// The extensions this node was configured with.
///
/// `None` means the point is unconfigured, which is not a fallback: callers
/// must answer as the point's contract says (storage answers 503, so a
/// device backs off and retries rather than being told a lie).
#[derive(Clone, Default)]
pub struct ExtRegistry {
    pub storage: Option<Arc<Extension>>,
    pub event_sink: Option<Arc<Extension>>,
    pub retention: Option<Arc<Extension>>,
    pub rollout: Option<Arc<Extension>>,
    pub console: Option<Arc<Extension>>,
    pub authz: Option<Arc<Extension>>,
    /// What the modules were loaded with. Kept so the node can say what it
    /// is running on, and so drift is measured against a value rather than
    /// re-derived from an environment that may since have changed.
    pub params: Arc<params::Resolved>,
}

impl ExtRegistry {
    pub fn from_config(config: &Config) -> AppResult<Self> {
        let file = Some(config.ext_params_file.as_str())
            .filter(|path| !path.trim().is_empty())
            .map(std::path::Path::new);
        let resolved = params::Resolved::load(file);
        // A file that is there and is not a parameters file stops the node,
        // exactly as a missing module does: a node that boots on half a
        // credentials file is worse than one that refuses to boot. An absent
        // or unreadable file does not — a mount can be late, and the node has
        // devices to serve.
        if let params::FileState::Malformed(why) = &resolved.state {
            return Err(AppError::Internal(anyhow::anyhow!(
                "{}: {why}",
                config.ext_params_file
            )));
        }
        let guest = resolved.for_guest();

        Ok(Self {
            storage: spawn_point(&config.ext_storage, points::STORAGE_LOCATE, config, &guest)?,
            event_sink: spawn_point(
                &config.ext_event_sink,
                points::EVENT_SINK_PLAN,
                config,
                &guest,
            )?,
            retention: spawn_point(
                &config.ext_retention,
                points::RETENTION_SELECT,
                config,
                &guest,
            )?,
            rollout: spawn_point(&config.ext_rollout, points::ROLLOUT_PLAN, config, &guest)?,
            console: spawn_point(
                &config.ext_console,
                points::CONSOLE_SURFACES,
                config,
                &guest,
            )?,
            authz: spawn_point(&config.ext_authz, points::AUTHZ_DECIDE, config, &guest)?,
            params: Arc::new(resolved),
        })
    }

    pub fn storage(&self) -> AppResult<&Extension> {
        self.storage.as_deref().ok_or_else(unavailable_storage)
    }
}

fn spawn_point(
    command: &str,
    point: &str,
    config: &Config,
    guest_env: &[(String, String)],
) -> AppResult<Option<Arc<Extension>>> {
    if command.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(Extension::new(
        point,
        command,
        std::time::Duration::from_millis(config.ext_deadline_ms),
        guest_env.to_vec(),
    )?)))
}

/// No storage extension, or it cannot answer.
///
/// Deliberately the overload error: it carries a `Retry-After`, so a fleet
/// that finds artifacts unavailable backs off instead of reading it as
/// "broken, retry now" (see API_NOTES — a node out of capacity answers 503,
/// never 500).
pub fn unavailable_storage() -> AppError {
    AppError::Unavailable("artifact storage extension is unavailable".to_string())
}

/// Typed call helper: encode the argument, call, decode the answer.
pub async fn call<I: Serialize, O: DeserializeOwned>(
    extension: &Extension,
    point: &str,
    argument: &I,
) -> AppResult<O> {
    let payload =
        serde_json::to_value(argument).map_err(|e| AppError::Internal(anyhow::Error::from(e)))?;
    let response = extension
        .call(ExtRequest {
            point: point.to_string(),
            payload,
        })
        .await?;
    serde_json::from_value(response).map_err(|e| {
        AppError::Internal(anyhow::Error::from(e).context(format!("{point} answered unusably")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every point is served by exactly one slot.
    ///
    /// Without this a point added to the ABI and forgotten here would be a
    /// capability the node has and the inventory cannot show — which is the
    /// one thing an inventory must not do.
    #[test]
    fn every_point_belongs_to_exactly_one_slot() {
        for point in points::ALL {
            let owners: Vec<&str> = SLOTS
                .iter()
                .filter(|slot| slot.points.contains(point))
                .map(|slot| slot.key)
                .collect();
            assert_eq!(owners.len(), 1, "{point} is served by {owners:?}");
        }
    }

    #[test]
    fn every_slot_says_what_its_absence_costs() {
        for slot in &SLOTS {
            assert!(!slot.absence.trim().is_empty(), "{}", slot.key);
            assert!(!slot.points.is_empty(), "{}", slot.key);
            assert!(
                crate::ext::params::is_reserved(slot.env_var),
                "{} must not be settable from the parameters file: it chooses \
                 which code the node runs",
                slot.env_var
            );
        }
    }

    #[test]
    fn a_slot_key_resolves_to_its_registry_field() {
        let registry = ExtRegistry::default();
        for slot in &SLOTS {
            // Default registry: nothing configured, but the key must be known
            // — an unknown key would silently report "not configured".
            assert!(registry.slot(slot.key).is_none());
        }
        assert!(registry.slot("nonexistent").is_none());
    }
}
