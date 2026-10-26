//! DDI protocol client (rewrite of hawkbit-sdk `DdiController`'s HTTP layer).

use anyhow::{Context, Result};
use futures::StreamExt;
use hawier_api::ddi::{
    Confirmation, DdiActionFeedback, DdiActivateAutoConfirmation, DdiArtifact, DdiCancel,
    DdiConfigData, DdiConfirmationBase, DdiConfirmationBaseAction, DdiConfirmationFeedback,
    DdiControllerBase, DdiDeploymentBase, DdiProgress, DdiResult, DdiStatus, ExecutionStatus,
    FinalResult,
};
use md5::Md5;
use sha1::{Digest, Sha1};

/// Authentication mode for DDI requests.
#[derive(Debug, Clone)]
pub enum DdiAuth {
    TargetToken(String),
    GatewayToken(String),
    None,
}

#[derive(Debug, Clone)]
pub struct DdiClient {
    http: reqwest::Client,
    base_url: String,
    tenant: String,
    controller_id: String,
    auth: DdiAuth,
}

/// Result of downloading + verifying one artifact.
#[derive(Debug)]
pub struct DownloadResult {
    pub filename: String,
    pub size: u64,
    pub sha1_ok: bool,
    pub md5_ok: bool,
}

impl DownloadResult {
    pub fn verified(&self) -> bool {
        self.sha1_ok && self.md5_ok
    }
}

/// Attached to an error when the server answered `503` with `Retry-After`.
///
/// The distinction matters to a fleet: `500` means "broken, try again", and
/// a thousand devices obey it at once. `503` with a delay is the only way a
/// node can push back, so a device that ignores the header keeps the
/// overload alive that the header exists to end.
#[derive(Debug, Clone, Copy)]
pub struct RetryAfter(pub std::time::Duration);

impl std::fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "server is busy, asked to retry in {}s", self.0.as_secs())
    }
}

impl std::error::Error for RetryAfter {}

/// Turn a non-2xx response into an error, keeping `Retry-After` with it.
///
/// Only the delta-seconds form is read. The HTTP-date form is legal but
/// needs a clock the device may not have set, and hawier always sends
/// seconds.
fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    let retry_after = if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
    } else {
        None
    };
    match resp.error_for_status() {
        Ok(resp) => Ok(resp),
        Err(error) => Err(match retry_after {
            Some(delay) => anyhow::Error::new(error).context(RetryAfter(delay)),
            None => anyhow::Error::new(error),
        }),
    }
}

/// Give up on a connection that never opens.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Give up on a connection that opened and then went silent. Per-read, so a
/// large artifact on a slow link is not affected.
pub const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A device's HTTP client.
///
/// `reqwest::Client::new()` has no timeouts of any kind, and a device that
/// hangs on a stalled connection is a device that is gone: it never polls
/// again, and the operator sees an action that stays "running" forever. In a
/// 1000-device run 17 of them froze on their first download and never came
/// back. The read timeout is per-read, not per-request, so a large artifact
/// on a slow link still finishes — only a silent connection is abandoned.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        // The builder only fails on a broken TLS backend, which would make
        // every request fail anyway.
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl DdiClient {
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        controller_id: impl Into<String>,
        auth: DdiAuth,
    ) -> Self {
        Self {
            http: http_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            tenant: tenant.into(),
            controller_id: controller_id.into(),
            auth,
        }
    }

    pub fn controller_id(&self) -> &str {
        &self.controller_id
    }

    pub fn controller_url(&self) -> String {
        format!(
            "{}/{}/controller/v1/{}",
            self.base_url, self.tenant, self.controller_id
        )
    }

    fn authorize(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            DdiAuth::TargetToken(token) => {
                req.header("Authorization", format!("TargetToken {token}"))
            }
            DdiAuth::GatewayToken(token) => {
                req.header("Authorization", format!("GatewayToken {token}"))
            }
            DdiAuth::None => req,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .authorize(self.http.get(url))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let resp = check_status(resp).with_context(|| format!("GET {url}"))?;
        Ok(resp.json().await?)
    }

    async fn send_json<B: serde::Serialize>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: &B,
    ) -> Result<()> {
        let resp = self
            .authorize(self.http.request(method.clone(), url))
            .json(body)
            .send()
            .await
            .with_context(|| format!("{method} {url}"))?;
        check_status(resp).with_context(|| format!("{method} {url}"))?;
        Ok(())
    }

    /// Poll the controller base resource. This is the ONLY URL a device is
    /// allowed to construct itself — everything else follows `_links`
    /// (doctrine, see API_NOTES.md).
    pub async fn poll(&self) -> Result<DdiControllerBase> {
        self.get_json(&self.controller_url()).await
    }

    /// Follow an absolute link returned in `_links`.
    pub async fn deployment_base_by_href(&self, href: &str) -> Result<DdiDeploymentBase> {
        self.get_json(href).await
    }

    /// The action's feedback endpoint, derived from the followed href
    /// (query dropped): `{href}/feedback`. Deriving from the href — not the
    /// configured base — keeps devices working when links point at other
    /// nodes (API_NOTES.md).
    pub fn feedback_href(action_href: &str) -> String {
        let base = action_href.split('?').next().unwrap_or(action_href);
        format!("{}/feedback", base.trim_end_matches('/'))
    }

    /// POST action feedback to the endpoint derived from the followed href.
    pub async fn post_feedback_by_href(
        &self,
        action_href: &str,
        feedback: &DdiActionFeedback,
    ) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &Self::feedback_href(action_href),
            feedback,
        )
        .await
    }

    /// POST confirmation feedback derived from the followed confirmation href.
    pub async fn post_confirmation_feedback_by_href(
        &self,
        action_href: &str,
        confirmation: Confirmation,
        details: Vec<String>,
    ) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &Self::feedback_href(action_href),
            &DdiConfirmationFeedback {
                confirmation,
                code: None,
                details,
            },
        )
        .await
    }

    /// GET the confirmationBase root via its poll `_links` entry.
    pub async fn confirmation_base_by_href(&self, href: &str) -> Result<DdiConfirmationBase> {
        self.get_json(href).await
    }

    /// PUT configData via its poll `_links` entry.
    pub async fn put_config_data_by_href(&self, href: &str, data: &DdiConfigData) -> Result<()> {
        self.send_json(reqwest::Method::PUT, href, data).await
    }

    /// GET installedBase via its poll `_links` entry.
    pub async fn installed_base_by_href(&self, href: &str) -> Result<DdiDeploymentBase> {
        self.get_json(href).await
    }

    // -----------------------------------------------------------------
    // Base-constructed helpers below exist for TESTS ONLY (driving single
    // endpoints without a prior poll). Devices must follow `_links`.
    // -----------------------------------------------------------------

    pub async fn deployment_base(&self, action_id: &str) -> Result<DdiDeploymentBase> {
        self.get_json(&format!(
            "{}/deploymentBase/{action_id}",
            self.controller_url()
        ))
        .await
    }

    pub async fn installed_base(&self, action_id: &str) -> Result<DdiDeploymentBase> {
        self.get_json(&format!(
            "{}/installedBase/{action_id}",
            self.controller_url()
        ))
        .await
    }

    pub async fn cancel_action_by_href(&self, href: &str) -> Result<DdiCancel> {
        self.get_json(href).await
    }

    pub async fn deployment_feedback(
        &self,
        action_id: &str,
        feedback: &DdiActionFeedback,
    ) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &format!(
                "{}/deploymentBase/{action_id}/feedback",
                self.controller_url()
            ),
            feedback,
        )
        .await
    }

    pub async fn cancel_feedback(
        &self,
        action_id: &str,
        feedback: &DdiActionFeedback,
    ) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &format!(
                "{}/cancelAction/{action_id}/feedback",
                self.controller_url()
            ),
            feedback,
        )
        .await
    }

    pub async fn put_config_data(&self, data: &DdiConfigData) -> Result<()> {
        self.send_json(
            reqwest::Method::PUT,
            &format!("{}/configData", self.controller_url()),
            data,
        )
        .await
    }

    pub async fn confirmation_base(&self) -> Result<DdiConfirmationBase> {
        self.get_json(&format!("{}/confirmationBase", self.controller_url()))
            .await
    }

    pub async fn confirmation_base_action(&self, href: &str) -> Result<DdiConfirmationBaseAction> {
        self.get_json(href).await
    }

    pub async fn confirmation_feedback(
        &self,
        action_id: &str,
        confirmation: Confirmation,
        details: Vec<String>,
    ) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &format!(
                "{}/confirmationBase/{action_id}/feedback",
                self.controller_url()
            ),
            &DdiConfirmationFeedback {
                confirmation,
                code: None,
                details,
            },
        )
        .await
    }

    pub async fn activate_auto_confirm(&self, initiator: Option<String>) -> Result<()> {
        self.send_json(
            reqwest::Method::POST,
            &format!(
                "{}/confirmationBase/activateAutoConfirm",
                self.controller_url()
            ),
            &DdiActivateAutoConfirmation {
                initiator,
                remark: None,
            },
        )
        .await
    }

    /// Stream-download an artifact via its `download-http` link, verifying
    /// sha1/md5 on the fly. Bytes go to `sink` (e.g. a file) or are discarded.
    pub async fn download_artifact(
        &self,
        artifact: &DdiArtifact,
        mut sink: Option<tokio::fs::File>,
    ) -> Result<DownloadResult> {
        use tokio::io::AsyncWriteExt;

        let href = artifact
            .links
            .get("download-http")
            .or_else(|| artifact.links.get("download"))
            .map(|l| l.href.clone())
            .with_context(|| format!("artifact {} has no download link", artifact.filename))?;

        let resp = self.authorize(self.http.get(&href)).send().await?;
        let resp = check_status(resp).with_context(|| format!("GET {href}"))?;

        let mut sha1 = Sha1::new();
        let mut md5 = Md5::new();
        let mut size: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            sha1.update(&chunk);
            md5.update(&chunk);
            size += chunk.len() as u64;
            if let Some(file) = sink.as_mut() {
                file.write_all(&chunk).await?;
            }
        }
        if let Some(file) = sink.as_mut() {
            file.flush().await?;
        }

        let sha1_hex = hex::encode(sha1.finalize());
        let md5_hex = hex::encode(md5.finalize());
        Ok(DownloadResult {
            filename: artifact.filename.clone(),
            size,
            sha1_ok: sha1_hex.eq_ignore_ascii_case(&artifact.hashes.sha1),
            md5_ok: artifact.hashes.md5.is_empty()
                || md5_hex.eq_ignore_ascii_case(&artifact.hashes.md5),
        })
    }
}

/// Build a standard feedback payload.
pub fn feedback(
    execution: ExecutionStatus,
    finished: FinalResult,
    details: Vec<String>,
) -> DdiActionFeedback {
    DdiActionFeedback {
        status: DdiStatus {
            execution,
            result: DdiResult {
                finished,
                progress: None,
            },
            code: None,
            details,
        },
        timestamp: None,
    }
}

/// Feedback with progress counter (`cnt` of `of`).
pub fn feedback_with_progress(
    execution: ExecutionStatus,
    finished: FinalResult,
    cnt: i32,
    of: i32,
) -> DdiActionFeedback {
    DdiActionFeedback {
        status: DdiStatus {
            execution,
            result: DdiResult {
                finished,
                progress: Some(DdiProgress { cnt, of: Some(of) }),
            },
            code: None,
            details: Vec::new(),
        },
        timestamp: None,
    }
}

#[cfg(test)]
mod tests {
    use super::DdiClient;

    #[test]
    fn feedback_href_strips_query_and_appends_feedback() {
        assert_eq!(
            DdiClient::feedback_href("http://h/DEFAULT/controller/v1/d/deploymentBase/7?c=123"),
            "http://h/DEFAULT/controller/v1/d/deploymentBase/7/feedback"
        );
        assert_eq!(
            DdiClient::feedback_href("http://h/DEFAULT/controller/v1/d/cancelAction/9"),
            "http://h/DEFAULT/controller/v1/d/cancelAction/9/feedback"
        );
        assert_eq!(
            DdiClient::feedback_href("http://h/x/confirmationBase/5/"),
            "http://h/x/confirmationBase/5/feedback"
        );
    }
}
