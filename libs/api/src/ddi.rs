//! DDI (Direct Device Integration) wire types, hawkBit 1.0-compatible.
//!
//! Serialized by hawier-server, deserialized by hawier-client. Field names
//! and optionality follow hawkBit's `org.eclipse.hawkbit.ddi.json.model.*`
//! (Jackson `NON_NULL` include → `skip_serializing_if = "Option::is_none"`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// HAL link: `{"href": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub href: String,
}

impl Link {
    pub fn new(href: impl Into<String>) -> Self {
        Self { href: href.into() }
    }
}

/// `_links` object; BTreeMap keeps serialization order deterministic.
pub type Links = BTreeMap<String, Link>;

// ---------------------------------------------------------------------------
// Controller base resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiControllerBase {
    pub config: DdiConfig,
    #[serde(rename = "_links", default, skip_serializing_if = "Links::is_empty")]
    pub links: Links,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiConfig {
    pub polling: DdiPolling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiPolling {
    /// `HH:mm:ss`.
    pub sleep: String,
}

// ---------------------------------------------------------------------------
// Deployment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiDeploymentBase {
    pub id: String,
    pub deployment: DdiDeployment,
    #[serde(rename = "actionHistory", skip_serializing_if = "Option::is_none")]
    pub action_history: Option<DdiActionHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiDeployment {
    pub download: HandlingType,
    pub update: HandlingType,
    #[serde(default)]
    pub chunks: Vec<DdiChunk>,
    #[serde(rename = "maintenanceWindow", skip_serializing_if = "Option::is_none")]
    pub maintenance_window: Option<MaintenanceWindowStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandlingType {
    Skip,
    Attempt,
    Forced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaintenanceWindowStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiChunk {
    /// Software module type key; legacy mapping `application`→`bApp`,
    /// `runtime`→`jvm` (hawkBit `DataConversionHelper.mapChunkLegacyKeys`).
    pub part: String,
    pub version: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<DdiArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Vec<DdiMetadata>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiArtifact {
    pub filename: String,
    pub hashes: DdiArtifactHash,
    pub size: i64,
    #[serde(rename = "_links", default, skip_serializing_if = "Links::is_empty")]
    pub links: Links,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiArtifactHash {
    pub sha1: String,
    pub md5: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiMetadata {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiActionHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
}

// ---------------------------------------------------------------------------
// Feedback
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiActionFeedback {
    pub status: DdiStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiStatus {
    pub execution: ExecutionStatus,
    pub result: DdiResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Closed,
    Proceeding,
    Canceled,
    Scheduled,
    Rejected,
    Resumed,
    Downloaded,
    Download,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiResult {
    pub finished: FinalResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<DdiProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinalResult {
    Success,
    Failure,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiProgress {
    pub cnt: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub of: Option<i32>,
}

// ---------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiCancel {
    pub id: String,
    #[serde(rename = "cancelAction")]
    pub cancel_action: DdiCancelActionToStop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiCancelActionToStop {
    #[serde(rename = "stopId")]
    pub stop_id: String,
}

// ---------------------------------------------------------------------------
// Config data (controller attributes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiConfigData {
    pub data: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<UpdateMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    Merge,
    Replace,
    Remove,
}

// ---------------------------------------------------------------------------
// Installed base (offline assigned version)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiAssignedVersion {
    pub name: String,
    pub version: String,
}

// ---------------------------------------------------------------------------
// Confirmation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiConfirmationBase {
    #[serde(rename = "autoConfirm")]
    pub auto_confirm: DdiAutoConfirmationState,
    #[serde(rename = "_links", default, skip_serializing_if = "Links::is_empty")]
    pub links: Links,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiAutoConfirmationState {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(rename = "activatedAt", skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiConfirmationBaseAction {
    pub id: String,
    pub confirmation: DdiDeployment,
    #[serde(rename = "actionHistory", skip_serializing_if = "Option::is_none")]
    pub action_history: Option<DdiActionHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiConfirmationFeedback {
    pub confirmation: Confirmation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confirmation {
    Confirmed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdiActivateAutoConfirmation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_base_serializes_like_hawkbit() {
        let mut links = Links::new();
        links.insert(
            "deploymentBase".into(),
            Link::new("http://h/DEFAULT/controller/v1/dev1/deploymentBase/7?c=-1"),
        );
        let base = DdiControllerBase {
            config: DdiConfig {
                polling: DdiPolling {
                    sleep: "00:05:00".into(),
                },
            },
            links,
        };
        let json = serde_json::to_value(&base).unwrap();
        assert_eq!(json["config"]["polling"]["sleep"], "00:05:00");
        assert!(json["_links"]["deploymentBase"]["href"]
            .as_str()
            .unwrap()
            .contains("deploymentBase/7"));
    }

    #[test]
    fn feedback_parses_hawkbit_shape() {
        let raw = r#"{
            "status": {
                "execution": "closed",
                "result": { "finished": "success", "progress": { "cnt": 2, "of": 5 } },
                "details": ["done"]
            }
        }"#;
        let fb: DdiActionFeedback = serde_json::from_str(raw).unwrap();
        assert_eq!(fb.status.execution, ExecutionStatus::Closed);
        assert_eq!(fb.status.result.finished, FinalResult::Success);
        assert_eq!(fb.status.result.progress.as_ref().unwrap().cnt, 2);
    }

    #[test]
    fn optional_fields_are_omitted() {
        let dep = DdiDeployment {
            download: HandlingType::Forced,
            update: HandlingType::Forced,
            chunks: vec![],
            maintenance_window: None,
        };
        let json = serde_json::to_string(&dep).unwrap();
        assert!(!json.contains("maintenanceWindow"));
        assert!(json.contains(r#""download":"forced""#));
    }
}
