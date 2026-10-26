//! DDI API (`/{tenant}/controller/v1/...`), hawkBit 1.0-compatible.
//!
//! The DDI path is deliberately free of any dependency on background
//! engines or singletons: sub-nodes serve it with just Postgres and the
//! artifact store, so devices keep updating when the control node is down.

pub mod artifacts;
pub mod base;
pub mod cancel;
pub mod config_data;
pub mod confirmation;
pub mod deployment;

use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::Router;
use hawier_api::codes::{ActionStatus, ActionType};
use hawier_api::ddi;
use sqlx::Row;

use crate::actions::ActionRow;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{tenant}/controller/v1/{controller_id}",
            get(base::controller_base),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/deploymentBase/{action_id}",
            get(deployment::deployment_base),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/deploymentBase/{action_id}/feedback",
            post(deployment::deployment_feedback),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/installedBase/{action_id}",
            get(deployment::installed_base),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/installedBase",
            put(deployment::put_installed_base),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/cancelAction/{action_id}",
            get(cancel::cancel_action),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/cancelAction/{action_id}/feedback",
            post(cancel::cancel_feedback),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/configData",
            put(config_data::put_config_data),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/softwaremodules/{sm_id}/artifacts",
            get(artifacts::list_artifacts),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/softwaremodules/{sm_id}/artifacts/{file_name}",
            get(artifacts::download_artifact),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/confirmationBase",
            get(confirmation::confirmation_base),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/confirmationBase/activateAutoConfirm",
            post(confirmation::activate_auto_confirm),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/confirmationBase/deactivateAutoConfirm",
            post(confirmation::deactivate_auto_confirm),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/confirmationBase/{action_id}",
            get(confirmation::confirmation_base_action),
        )
        .route(
            "/{tenant}/controller/v1/{controller_id}/confirmationBase/{action_id}/feedback",
            post(confirmation::confirmation_feedback),
        )
}

/// External base URL as seen by the device (`http(s)://host`), for `_links`.
pub fn request_base(headers: &HeaderMap) -> String {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{proto}://{host}")
}

pub fn controller_href(headers: &HeaderMap, tenant: &str, controller_id: &str) -> String {
    format!(
        "{}/{}/controller/v1/{}",
        request_base(headers),
        tenant,
        controller_id
    )
}

/// Map SM type key to the legacy DDI chunk `part` value
/// (hawkBit `DataConversionHelper.mapChunkLegacyKeys`).
pub fn legacy_chunk_part(type_key: &str) -> String {
    match type_key {
        "application" => "bApp".to_string(),
        "runtime" => "jvm".to_string(),
        other => other.to_string(),
    }
}

/// download/update handling pair for an action.
pub fn handling_types(action: &ActionRow, now: i64) -> (ddi::HandlingType, ddi::HandlingType) {
    match ActionType::try_from(action.action_type) {
        Ok(ActionType::Forced) => (ddi::HandlingType::Forced, ddi::HandlingType::Forced),
        Ok(ActionType::DownloadOnly) => (ddi::HandlingType::Forced, ddi::HandlingType::Skip),
        Ok(ActionType::TimeForced) => {
            if action.forced_time.is_some_and(|t| t <= now) {
                (ddi::HandlingType::Forced, ddi::HandlingType::Forced)
            } else {
                (ddi::HandlingType::Attempt, ddi::HandlingType::Attempt)
            }
        }
        _ => (ddi::HandlingType::Attempt, ddi::HandlingType::Attempt),
    }
}

/// Build the `deployment` payload (chunks with artifacts + target-visible
/// metadata) for an action. Shared by deploymentBase, installedBase and
/// confirmationBase.
pub async fn build_deployment(
    state: &AppState,
    headers: &HeaderMap,
    tenant: &str,
    controller_id: &str,
    action: &ActionRow,
) -> AppResult<ddi::DdiDeployment> {
    let now = crate::state::now_millis();
    let (download, mut update) = handling_types(action, now);

    // hawkBit: a defined-but-closed maintenance window forces update=skip —
    // the device may download but must not apply until the window opens.
    let maintenance_window = crate::maintenance::action_window_state(
        &action.maintenance_cron_schedule,
        &action.maintenance_duration,
        &action.maintenance_time_zone,
        now,
    )
    .map(|window| {
        if window.available {
            ddi::MaintenanceWindowStatus::Available
        } else {
            if update != ddi::HandlingType::Skip {
                update = ddi::HandlingType::Skip;
            }
            ddi::MaintenanceWindowStatus::Unavailable
        }
    });

    let modules = sqlx::query(
        "SELECT sm.id, sm.name, sm.version, sm.encrypted, t.type_key
         FROM sp_ds_sm link
         JOIN sp_software_module sm ON sm.id = link.sm_id
         JOIN sp_software_module_type t ON t.id = sm.sm_type
         WHERE link.ds_id = $1
         ORDER BY sm.id",
    )
    .bind(action.distribution_set)
    .fetch_all(&state.ddi_pool)
    .await?;

    let mut chunks = Vec::with_capacity(modules.len());
    for module in &modules {
        let sm_id: i64 = module.get("id");

        let artifact_rows = sqlx::query(
            "SELECT provided_file_name, sha1_hash, md5_hash, sha256_hash, file_size
             FROM sp_artifact WHERE software_module = $1 ORDER BY id",
        )
        .bind(sm_id)
        .fetch_all(&state.ddi_pool)
        .await?;

        let mut artifacts = Vec::with_capacity(artifact_rows.len());
        for artifact in &artifact_rows {
            let filename: String = artifact.get("provided_file_name");
            let mut links = ddi::Links::new();
            let file_href = format!(
                "{}/softwaremodules/{}/artifacts/{}",
                controller_href(headers, tenant, controller_id),
                sm_id,
                filename
            );
            links.insert("download-http".into(), ddi::Link::new(file_href.clone()));
            links.insert(
                "md5sum-http".into(),
                ddi::Link::new(format!("{file_href}.MD5SUM")),
            );
            artifacts.push(ddi::DdiArtifact {
                filename,
                hashes: ddi::DdiArtifactHash {
                    sha1: artifact.get("sha1_hash"),
                    md5: artifact
                        .get::<Option<String>, _>("md5_hash")
                        .unwrap_or_default(),
                    sha256: artifact
                        .get::<Option<String>, _>("sha256_hash")
                        .map(|s| s.trim().to_string()),
                },
                size: artifact.get::<Option<i64>, _>("file_size").unwrap_or(0),
                links,
            });
        }

        let metadata_rows = sqlx::query(
            "SELECT meta_key, meta_value FROM sp_sm_metadata
             WHERE sm = $1 AND target_visible = TRUE ORDER BY meta_key",
        )
        .bind(sm_id)
        .fetch_all(&state.ddi_pool)
        .await?;
        let metadata: Vec<ddi::DdiMetadata> = metadata_rows
            .iter()
            .map(|r| ddi::DdiMetadata {
                key: r.get("meta_key"),
                value: r.get::<Option<String>, _>("meta_value").unwrap_or_default(),
            })
            .collect();

        chunks.push(ddi::DdiChunk {
            part: legacy_chunk_part(module.get::<String, _>("type_key").as_str()),
            version: module.get("version"),
            name: module.get("name"),
            encrypted: module.get::<Option<bool>, _>("encrypted").filter(|e| *e),
            artifacts,
            metadata: if metadata.is_empty() {
                None
            } else {
                Some(metadata)
            },
        });
    }

    Ok(ddi::DdiDeployment {
        download,
        update,
        chunks,
        maintenance_window,
    })
}

/// The controller's current action via the DDI pool (weight DESC, id ASC).
pub async fn next_active_or_none(state: &AppState, target_id: i64) -> AppResult<Option<ActionRow>> {
    crate::actions::next_active_action(&state.ddi_pool, target_id).await
}

/// Build the optional `actionHistory` block: `actionHistory` query param —
/// `0`/absent = none, positive = last N messages, negative = all.
pub async fn build_action_history(
    state: &AppState,
    action: &ActionRow,
    param: Option<&str>,
) -> AppResult<Option<ddi::DdiActionHistory>> {
    let count: i64 = match param.map(str::trim) {
        None | Some("") | Some("0") => return Ok(None),
        Some(v) => v
            .parse()
            .map_err(|_| AppError::BadRequest(format!("invalid actionHistory value '{v}'")))?,
    };

    let limit = if count < 0 { i64::MAX } else { count };
    let rows = sqlx::query(
        "SELECT m.detail_message
         FROM sp_action_status s
         JOIN sp_action_status_messages m ON m.action_status = s.id
         WHERE s.action = $1
         ORDER BY s.id DESC
         LIMIT $2",
    )
    .bind(action.id)
    .bind(limit)
    .fetch_all(&state.ddi_pool)
    .await?;

    let status_name = ActionStatus::try_from(action.status)
        .map(|s| s.name().to_string())
        .unwrap_or_else(|_| action.status.to_string());
    Ok(Some(ddi::DdiActionHistory {
        status: Some(status_name),
        messages: rows.iter().map(|r| r.get("detail_message")).collect(),
    }))
}
