//! Wire types for hawier's internal `/ui/v1` state API.
//!
//! hawier-server owns the `hawier_*` Postgres tables; hawier-ui is a pure
//! HTTP client of this API (no direct DB access, per project decision).
//! Field shapes intentionally match the structs hawier-ui already uses so
//! the UI switch-over stays a small diff.

use serde::{Deserialize, Serialize};

/// A saved table view, scoped by `(tenant, username, resource)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedView {
    pub id: String,
    pub tenant: String,
    pub username: String,
    pub resource: String,
    pub name: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub columns: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    #[serde(default)]
    pub position: usize,
    /// Epoch milliseconds — every timestamp on hawier's wire is millis.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_page_size() -> usize {
    25
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedViewCreate {
    pub tenant: String,
    pub username: String,
    pub resource: String,
    pub name: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub columns: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedViewUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<usize>,
}

/// Reorder request: complete list of view ids in the new order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedViewReorder {
    pub tenant: String,
    pub username: String,
    pub resource: String,
    pub ids: Vec<String>,
}

/// One UI action-log entry (two-phase: started → finished).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLog {
    pub id: String,
    pub tenant: String,
    pub username: String,
    pub resource: String,
    pub resource_id: String,
    pub action: String,
    pub status: String,
    pub request_summary: String,
    pub response_summary: String,
    pub error: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLogStart {
    pub tenant: String,
    pub username: String,
    pub resource: String,
    pub resource_id: String,
    pub action: String,
    pub request_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLogFinish {
    /// `success` or `error`.
    pub status: String,
    #[serde(default)]
    pub response_summary: String,
    #[serde(default)]
    pub error: String,
}

/// UI session record.
///
/// NARROW: carries the user's mgmt password so the UI can replay HTTP Basic
/// upstream (hawkBit-era design). Pre-production hawier replaces this with
/// server-issued tokens; see SCHEMA_NOTES.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSession {
    /// Ordinary number, like every other hawier id.
    pub id: String,
    /// The bearer credential the browser cookie carries. Unguessable on
    /// purpose: it unlocks `password` below.
    pub token: String,
    pub username: String,
    pub tenant: String,
    pub password: String,
    /// Epoch milliseconds.
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSessionCreate {
    pub username: String,
    pub tenant: String,
    pub password: String,
}
