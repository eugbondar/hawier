//! Simulated device: the poll loop (rewrite of hawkbit-sdk-device's
//! `DdiController` + `UpdateHandler` behaviour).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use anyhow::Result;
use hawier_api::ddi::{
    Confirmation, DdiConfigData, DdiControllerBase, ExecutionStatus, FinalResult, UpdateMode,
};
use tracing::{info, warn};

use crate::client::{feedback, DdiClient};

/// Fallback poll interval when the server's `polling.sleep` is unparsable.
const DEFAULT_POLL: Duration = Duration::from_secs(5);
/// Fast follow-up poll after handling work (mirrors sdk IMMEDIATE_MS).
const IMMEDIATE: Duration = Duration::from_millis(50);
/// First pause after a failed cycle; doubles per consecutive failure.
const BACKOFF_BASE: Duration = Duration::from_millis(200);
/// Longest wait a server may ask for. A device that stops polling is a
/// device that cannot be recovered remotely.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct DeviceOptions {
    /// Override the server-provided polling sleep.
    pub poll_override: Option<Duration>,
    /// Attributes sent once via configData at startup.
    pub attributes: BTreeMap<String, String>,
    /// Directory to store downloads; None = verify-and-discard.
    pub download_dir: Option<std::path::PathBuf>,
    /// Run a single poll cycle, handle at most one unit of work, then exit.
    pub once: bool,
    /// Fault injection: report failure after download.
    pub fail_download: bool,
    /// Deny confirmations instead of confirming.
    pub deny_confirmation: bool,
    /// Defer soft/attempt updates: download, report `downloaded`, apply
    /// only when the server escalates handling to forced.
    pub defer_soft: bool,
    /// Confirm pending confirmations automatically.
    pub auto_confirm: bool,
}

impl Default for DeviceOptions {
    fn default() -> Self {
        Self {
            poll_override: None,
            attributes: BTreeMap::new(),
            download_dir: None,
            once: false,
            fail_download: false,
            deny_confirmation: false,
            defer_soft: false,
            auto_confirm: true,
        }
    }
}

/// The delay a `503 Retry-After` asked for.
///
/// `anyhow::Error::downcast_ref`, not `chain()`: a value attached with
/// `.context()` is not one of the chain's links — the links are wrapper
/// types — so walking the chain finds nothing and the device hammers on.
fn retry_after(error: &anyhow::Error) -> Option<Duration> {
    error
        .downcast_ref::<crate::client::RetryAfter>()
        .map(|hint| hint.0)
}

/// Exponential retry pause, capped at the interval the device would poll on
/// anyway — a failing device must never be busier than a healthy one. The
/// jitter is derived from the controller id: a thousand devices that failed
/// together must not come back together.
fn backoff_for(failures: u32, cap: Duration, controller_id: &str) -> Duration {
    use std::hash::{Hash, Hasher};
    let doublings = failures.clamp(1, 16) - 1;
    let capped = BACKOFF_BASE.saturating_mul(1u32 << doublings).min(cap);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    controller_id.hash(&mut hasher);
    // +/-25%, deterministic per device.
    let spread = (hasher.finish() % 500) as f64 / 1000.0 - 0.25;
    capped.mul_f64(1.0 + spread)
}

/// Outcome of one poll cycle, mostly for tests.
#[derive(Debug, PartialEq, Eq)]
pub enum CycleOutcome {
    Idle,
    UpdatedSuccessfully,
    UpdateFailed,
    Canceled,
    Confirmed,
    Denied,
    /// Downloaded, waiting for the maintenance window to open.
    WaitingForWindow,
    /// Downloaded a soft/attempt update, waiting for forced escalation
    /// (`defer_soft`).
    Deferred,
}

pub struct Device {
    client: DdiClient,
    pub options: DeviceOptions,
    attributes_sent: bool,
    /// Action ids already downloaded while their maintenance window was
    /// closed — don't re-download or re-report on every poll.
    predownloaded: HashSet<String>,
    /// Consecutive failed cycles, for the retry backoff.
    failures: u32,
}

impl Device {
    pub fn new(client: DdiClient, options: DeviceOptions) -> Self {
        Self {
            client,
            options,
            attributes_sent: false,
            predownloaded: HashSet::new(),
            failures: 0,
        }
    }

    pub fn client(&self) -> &DdiClient {
        &self.client
    }

    /// Parse hawkBit's `HH:mm:ss` polling sleep.
    pub fn parse_sleep(sleep: &str) -> Option<Duration> {
        let mut parts = sleep.split(':');
        let hours: u64 = parts.next()?.parse().ok()?;
        let minutes: u64 = parts.next()?.parse().ok()?;
        let seconds: u64 = parts.next()?.parse().ok()?;
        Some(Duration::from_secs(hours * 3600 + minutes * 60 + seconds))
    }

    /// Run the poll loop forever (or a single cycle with `once`).
    pub async fn run(&mut self) -> Result<()> {
        loop {
            let outcome = self.cycle().await;
            match &outcome {
                Ok(outcome) => info!(
                    controller = self.client().controller_id(),
                    ?outcome,
                    "cycle"
                ),
                Err(error) => {
                    // `%error` prints the outermost context only ("GET
                    // <url>"), which is the one part already obvious from
                    // the request. `{:#}` walks the whole chain, so the
                    // reason — timeout, 503, connection reset — is legible.
                    warn!(
                        controller = self.client().controller_id(),
                        error = format!("{error:#}"),
                        "cycle failed"
                    )
                }
            }
            if self.options.once {
                outcome?;
                return Ok(());
            }
            let sleep = match &outcome {
                // The server shortens polling.sleep ahead of a maintenance
                // window — honour it instead of hammering with IMMEDIATE.
                Ok(
                    CycleOutcome::Idle | CycleOutcome::WaitingForWindow | CycleOutcome::Deferred,
                ) => {
                    self.failures = 0;
                    self.next_sleep().await
                }
                Ok(_) => {
                    self.failures = 0;
                    IMMEDIATE
                }
                // A failed cycle used to retry with no pause at all. Under
                // load that is congestion collapse: the devices whose
                // download timed out immediately take bandwidth back from
                // the devices still making progress, so more time out. A
                // 1000-device run produced 92k redundant reports this way.
                Err(error) => {
                    self.failures = self.failures.saturating_add(1);
                    // A node that says "busy, come back in N" has told us
                    // more than any local heuristic can guess. Honour it —
                    // that header is the server's only lever against a
                    // fleet, and a device that ignores it keeps the overload
                    // alive. Capped so a bad header cannot park a device.
                    match retry_after(error) {
                        Some(delay) => delay.min(MAX_RETRY_AFTER),
                        None => self.backoff().await,
                    }
                }
            };
            tokio::time::sleep(sleep).await;
        }
    }

    /// Exponential retry pause, capped at the interval the device would
    /// poll on anyway — a failing device must never be busier than a healthy
    /// one. Jitter is derived from the controller id: a thousand devices
    /// that failed together must not come back together.
    async fn backoff(&self) -> Duration {
        backoff_for(
            self.failures,
            self.next_sleep().await,
            self.client().controller_id(),
        )
    }

    async fn next_sleep(&self) -> Duration {
        if let Some(d) = self.options.poll_override {
            return d;
        }
        match self.client().poll().await {
            Ok(base) => Self::parse_sleep(&base.config.polling.sleep).unwrap_or(DEFAULT_POLL),
            Err(_) => DEFAULT_POLL,
        }
    }

    /// One poll cycle: send attributes if requested, then handle the most
    /// urgent link (cancel > confirmation > deployment).
    pub async fn cycle(&mut self) -> Result<CycleOutcome> {
        let base = self.client().poll().await?;

        if !self.attributes_sent
            && (base.links.contains_key("configData") || !self.options.attributes.is_empty())
        {
            // Doctrine (API_NOTES.md): use the provided link; the base URL
            // is only for the poll itself.
            let config_href = base.links.get("configData").map(|l| l.href.clone());
            self.send_attributes(config_href.as_deref()).await?;
            self.attributes_sent = true;
        }

        if let Some(link) = base.links.get("cancelAction").cloned() {
            return self.handle_cancel(&link.href).await;
        }
        if let Some(link) = base.links.get("confirmationBase").cloned() {
            if let Some(outcome) = self.handle_confirmation(&link.href).await? {
                return Ok(outcome);
            }
        }
        if let Some(link) = base.links.get("deploymentBase").cloned() {
            return self.handle_deployment(&link.href).await;
        }
        let _ = self.sleep_from(&base);
        Ok(CycleOutcome::Idle)
    }

    fn sleep_from(&self, base: &DdiControllerBase) -> Duration {
        Self::parse_sleep(&base.config.polling.sleep).unwrap_or(DEFAULT_POLL)
    }

    async fn send_attributes(&self, config_href: Option<&str>) -> Result<()> {
        let mut data = self.options.attributes.clone();
        if data.is_empty() {
            data.insert("hawier.client".into(), env!("CARGO_PKG_VERSION").into());
        }
        let payload = DdiConfigData {
            data,
            mode: Some(UpdateMode::Merge),
        };
        match config_href {
            Some(href) => self.client().put_config_data_by_href(href, &payload).await,
            None => self.client().put_config_data(&payload).await,
        }
    }

    async fn handle_cancel(&self, href: &str) -> Result<CycleOutcome> {
        // Fetch the stop order, then answer at {href}/feedback — never at a
        // URL rebuilt from the configured base (API_NOTES.md doctrine).
        let _cancel = self.client().cancel_action_by_href(href).await?;
        self.client()
            .post_feedback_by_href(
                href,
                &feedback(
                    ExecutionStatus::Canceled,
                    FinalResult::Success,
                    vec!["Simulated device canceled the action.".into()],
                ),
            )
            .await?;
        Ok(CycleOutcome::Canceled)
    }

    async fn handle_confirmation(&self, root_href: &str) -> Result<Option<CycleOutcome>> {
        let confirmation = self.client().confirmation_base_by_href(root_href).await?;
        let Some(link) = confirmation.links.get("confirmationBase").cloned() else {
            return Ok(None);
        };
        let _action = self.client().confirmation_base_action(&link.href).await?;
        if self.options.deny_confirmation {
            self.client()
                .post_confirmation_feedback_by_href(
                    &link.href,
                    Confirmation::Denied,
                    vec!["Simulated device denied the action.".into()],
                )
                .await?;
            return Ok(Some(CycleOutcome::Denied));
        }
        if !self.options.auto_confirm {
            return Ok(Some(CycleOutcome::Idle));
        }
        self.client()
            .post_confirmation_feedback_by_href(
                &link.href,
                Confirmation::Confirmed,
                vec!["Simulated device confirmed the action.".into()],
            )
            .await?;
        Ok(Some(CycleOutcome::Confirmed))
    }

    async fn handle_deployment(&mut self, href: &str) -> Result<CycleOutcome> {
        let deployment = self.client().deployment_base_by_href(href).await?;
        let window_closed = deployment.deployment.maintenance_window
            == Some(hawier_api::ddi::MaintenanceWindowStatus::Unavailable);
        let already_downloaded = self.predownloaded.contains(&deployment.id);

        // Window closed and everything already fetched: nothing to do until
        // the server reopens the update (no repeated feedback spam).
        if window_closed && already_downloaded {
            return Ok(CycleOutcome::WaitingForWindow);
        }

        let mut all_verified = true;
        let mut details = Vec::new();
        if !already_downloaded {
            self.client()
                .post_feedback_by_href(
                    href,
                    &feedback(
                        ExecutionStatus::Proceeding,
                        FinalResult::None,
                        vec!["Simulated device started the update.".into()],
                    ),
                )
                .await?;

            // Download every artifact of every chunk, verifying hashes.
            for chunk in &deployment.deployment.chunks {
                for artifact in &chunk.artifacts {
                    let sink = match &self.options.download_dir {
                        Some(dir) => {
                            tokio::fs::create_dir_all(dir).await?;
                            Some(tokio::fs::File::create(dir.join(&artifact.filename)).await?)
                        }
                        None => None,
                    };
                    let result = self.client().download_artifact(artifact, sink).await?;
                    if result.verified() {
                        details.push(format!(
                            "Downloaded {} ({} bytes), hashes verified.",
                            result.filename, result.size
                        ));
                    } else {
                        all_verified = false;
                        details.push(format!(
                            "Hash mismatch for {} (sha1 ok: {}, md5 ok: {}).",
                            result.filename, result.sha1_ok, result.md5_ok
                        ));
                    }
                }
            }

            self.client()
                .post_feedback_by_href(
                    href,
                    &feedback(
                        ExecutionStatus::Downloaded,
                        FinalResult::None,
                        details.clone(),
                    ),
                )
                .await?;
        }

        // Maintenance window closed: keep the payload, apply on a later poll
        // once the deployment reports the window available again.
        if window_closed {
            if all_verified && !self.options.fail_download {
                self.predownloaded.insert(deployment.id.clone());
                return Ok(CycleOutcome::WaitingForWindow);
            }
            return Ok(CycleOutcome::UpdateFailed);
        }

        // Polite mode: soft/attempt updates stay at `downloaded` until the
        // server escalates handling to forced (TIMEFORCED or an operator).
        if self.options.defer_soft
            && deployment.deployment.update == hawier_api::ddi::HandlingType::Attempt
        {
            if all_verified && !self.options.fail_download {
                self.predownloaded.insert(deployment.id.clone());
                return Ok(CycleOutcome::Deferred);
            }
            return Ok(CycleOutcome::UpdateFailed);
        }

        // DOWNLOAD_ONLY deployments (update: skip) end at `downloaded` —
        // the server closes the action itself; a `closed` here would 410.
        if deployment.deployment.update == hawier_api::ddi::HandlingType::Skip {
            return Ok(if all_verified && !self.options.fail_download {
                CycleOutcome::UpdatedSuccessfully
            } else {
                CycleOutcome::UpdateFailed
            });
        }
        self.predownloaded.remove(&deployment.id);

        let success = all_verified && !self.options.fail_download;
        let (result, message) = if success {
            (
                FinalResult::Success,
                "Simulated update applied successfully.",
            )
        } else {
            (FinalResult::Failure, "Simulated update failed.")
        };
        details.push(message.into());
        self.client()
            .post_feedback_by_href(href, &feedback(ExecutionStatus::Closed, result, details))
            .await?;

        Ok(if success {
            CycleOutcome::UpdatedSuccessfully
        } else {
            CycleOutcome::UpdateFailed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_polling_sleep() {
        assert_eq!(
            Device::parse_sleep("00:05:00"),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            Device::parse_sleep("01:00:30"),
            Some(Duration::from_secs(3630))
        );
        assert_eq!(Device::parse_sleep("garbage"), None);
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::{backoff_for, BACKOFF_BASE};
    use std::time::Duration;

    #[test]
    fn retries_slow_down_and_stop_at_the_poll_interval() {
        let cap = Duration::from_secs(5);
        let first = backoff_for(1, cap, "dev-1");
        let second = backoff_for(2, cap, "dev-1");
        assert!(
            second > first,
            "the second failure waits longer: {first:?} -> {second:?}"
        );
        assert!(first >= BACKOFF_BASE.mul_f64(0.75) && first <= BACKOFF_BASE.mul_f64(1.25));
        // However long the outage, a failing device is never busier than a
        // healthy one: the cap is the interval it would poll on anyway.
        assert!(backoff_for(30, cap, "dev-1") <= cap.mul_f64(1.25));
    }

    #[test]
    fn devices_that_failed_together_come_back_apart() {
        let cap = Duration::from_secs(5);
        let waits: Vec<Duration> = (0..50)
            .map(|i| backoff_for(4, cap, &format!("dev-{i}")))
            .collect();
        let distinct = waits.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(
            distinct > 40,
            "jitter must spread the herd, got {distinct} distinct waits"
        );
    }
}
