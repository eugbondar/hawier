//! DDI artifact endpoints: listing, binary download (streaming, ETag,
//! If-Match/If-None-Match, single Range), and the .MD5SUM companion.
//!
//! This is the 10Gbit hot path: no serde on the byte path, no whole-file
//! buffering — the storage hands us a bounded byte stream.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hawier_api::codes::ActionStatus;
use hawier_api::ddi;
use sqlx::Row;

use super::controller_href;
use crate::actions;
use crate::auth::authenticate_ddi;
use crate::db::model::CONTROLLER_PRINCIPAL;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Verify the software module belongs to a DS this target was ever assigned
/// (hawkBit only serves artifacts of the target's own deployments).
async fn authorize_module(state: &AppState, target_id: i64, sm_id: i64) -> AppResult<()> {
    let allowed: bool = sqlx::query(
        "SELECT EXISTS (
            SELECT 1 FROM sp_action a
            JOIN sp_ds_sm l ON l.ds_id = a.distribution_set
            WHERE a.target = $1 AND l.sm_id = $2) AS e",
    )
    .bind(target_id)
    .bind(sm_id)
    .fetch_one(&state.ddi_pool)
    .await?
    .get("e");
    if allowed {
        Ok(())
    } else {
        Err(AppError::NotFound(
            "software module not assigned to this target".to_string(),
        ))
    }
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    Path((tenant, controller_id, sm_id)): Path<(String, String, i64)>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<ddi::DdiArtifact>>> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?;
    authorize_module(&state, target.id, sm_id).await?;

    let rows = sqlx::query(
        "SELECT provided_file_name, sha1_hash, md5_hash, sha256_hash, file_size
         FROM sp_artifact WHERE software_module = $1 AND UPPER(tenant) = UPPER($2)
         ORDER BY id",
    )
    .bind(sm_id)
    .bind(&tenant)
    .fetch_all(&state.ddi_pool)
    .await?;

    let base = controller_href(&headers, &tenant, &controller_id);
    let artifacts = rows
        .iter()
        .map(|row| {
            let filename: String = row.get("provided_file_name");
            let href = format!("{base}/softwaremodules/{sm_id}/artifacts/{filename}");
            let mut links = ddi::Links::new();
            links.insert("download-http".into(), ddi::Link::new(href.clone()));
            links.insert(
                "md5sum-http".into(),
                ddi::Link::new(format!("{href}.MD5SUM")),
            );
            ddi::DdiArtifact {
                filename,
                hashes: ddi::DdiArtifactHash {
                    sha1: row.get("sha1_hash"),
                    md5: row.get::<Option<String>, _>("md5_hash").unwrap_or_default(),
                    sha256: row
                        .get::<Option<String>, _>("sha256_hash")
                        .map(|s| s.trim().to_string()),
                },
                size: row.get::<Option<i64>, _>("file_size").unwrap_or(0),
                links,
            }
        })
        .collect();
    Ok(Json(artifacts))
}

struct ArtifactMeta {
    sha1: String,
    md5: String,
    size: i64,
    filename: String,
}

async fn load_artifact_meta(
    state: &AppState,
    tenant: &str,
    sm_id: i64,
    filename: &str,
) -> AppResult<Option<ArtifactMeta>> {
    let row = sqlx::query(
        "SELECT provided_file_name, sha1_hash, md5_hash, file_size
         FROM sp_artifact
         WHERE software_module = $1 AND provided_file_name = $2 AND UPPER(tenant) = UPPER($3)",
    )
    .bind(sm_id)
    .bind(filename)
    .bind(tenant)
    .fetch_optional(&state.ddi_pool)
    .await?;
    Ok(row.map(|row| ArtifactMeta {
        sha1: row.get("sha1_hash"),
        md5: row.get::<Option<String>, _>("md5_hash").unwrap_or_default(),
        size: row.get::<Option<i64>, _>("file_size").unwrap_or(0),
        filename: row.get("provided_file_name"),
    }))
}

/// Parse a single `bytes=a-b` range. Multi-range → None (serve full 200,
/// mirroring hawkBit closely enough). Returns 416 error when unsatisfiable.
///
/// Shared with the mgmt download endpoint, which needs the identical
/// semantics; the DDI path itself is left exactly as it was.
pub(crate) fn parse_range(headers: &HeaderMap, total: u64) -> AppResult<Option<(u64, u64)>> {
    let Some(raw) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let Some(spec) = raw.strip_prefix("bytes=") else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None); // multi-range: serve the full body
    }
    let (start_s, end_s) = spec.split_once('-').unwrap_or((spec, ""));
    let range = match (start_s.is_empty(), end_s.is_empty()) {
        (false, false) => {
            let start: u64 = start_s.parse().map_err(|_| bad_range())?;
            let end: u64 = end_s.parse().map_err(|_| bad_range())?;
            (start, end.min(total.saturating_sub(1)))
        }
        (false, true) => {
            let start: u64 = start_s.parse().map_err(|_| bad_range())?;
            (start, total.saturating_sub(1))
        }
        (true, false) => {
            let suffix: u64 = end_s.parse().map_err(|_| bad_range())?;
            (total.saturating_sub(suffix), total.saturating_sub(1))
        }
        (true, true) => return Ok(None),
    };
    if range.0 > range.1 || range.0 >= total {
        return Err(AppError::BadRequest("range not satisfiable".to_string()));
    }
    Ok(Some(range))
}

fn bad_range() -> AppError {
    AppError::BadRequest("malformed Range header".to_string())
}

fn strip_quotes(v: &str) -> &str {
    v.trim().trim_matches('"')
}

pub async fn download_artifact(
    State(state): State<AppState>,
    Path((tenant, controller_id, sm_id, file_name)): Path<(String, String, i64, String)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let actor = authenticate_ddi(&state, &tenant, &controller_id, &headers).await?;
    let target = actor.require_target()?.clone();
    authorize_module(&state, target.id, sm_id).await?;

    // `.MD5SUM` companion resource.
    if let Some(real_name) = file_name.strip_suffix(".MD5SUM") {
        let meta = load_artifact_meta(&state, &tenant, sm_id, real_name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("artifact {real_name} not found")))?;
        let body = format!("{}  {}\n", meta.md5, meta.filename);
        return Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
                (
                    header::CONTENT_DISPOSITION,
                    HeaderValue::from_str(&format!("attachment;filename={}.MD5SUM", meta.filename))
                        .map_err(|e| AppError::Internal(e.into()))?,
                ),
            ],
            body,
        )
            .into_response());
    }

    let meta = load_artifact_meta(&state, &tenant, sm_id, &file_name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("artifact {file_name} not found")))?;

    // Conditional headers: ETag is the sha1.
    if let Some(if_match) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match != "*" && strip_quotes(if_match) != meta.sha1 {
            return Ok(StatusCode::PRECONDITION_FAILED.into_response());
        }
    }
    let etag_value = HeaderValue::from_str(&format!("\"{}\"", meta.sha1))
        .map_err(|e| AppError::Internal(e.into()))?;
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if strip_quotes(inm) == meta.sha1 {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            resp.headers_mut().insert(header::ETAG, etag_value);
            return Ok(resp);
        }
    }

    let total = meta.size.max(0) as u64;
    let range = parse_range(&headers, total)?;

    // One question to the storage extension, answered before the status
    // write so a download costs exactly one call to it. Everything that
    // decides anything has already happened above — actor authentication,
    // module authorisation, the sha1 ETag and its conditional headers —
    // because the request arrives here whoever ends up serving the bytes.
    let download = state
        .storage
        .for_download(&tenant, &meta.sha1, range)
        .await?;

    // Full downloads of an active deployment are logged as DOWNLOAD status
    // (range requests are resume traffic — hawkBit skips logging those).
    // Written for a redirect too: a device's progress must not depend on who
    // hands it the payload.
    if range.is_none() {
        if let Some(action) = super::next_active_or_none(&state, target.id).await? {
            if action.is_active() {
                let mut tx = state.ddi_pool.begin().await?;
                actions::insert_status(
                    &mut tx,
                    &tenant,
                    action.id,
                    ActionStatus::Download,
                    None,
                    None,
                    CONTROLLER_PRINCIPAL,
                    &[format!("Target downloads {}", meta.filename)],
                )
                .await
                .ok();
                tx.commit().await?;
            }
        }
    }

    let reader = match download {
        crate::artifact::Download::Redirect(url) => {
            // No Range handling here on purpose: the device re-issues its own
            // Range against the object store, which honours it natively. The
            // ETag stays hawier's sha1 so a resuming device still validates
            // against what the database promised (the store's own ETag is not a
            // sha1 for multipart objects — API_NOTES/EXTENSIONS: integrity is
            // the client's sha1/md5 check, not the transport's ETag).
            let mut response = StatusCode::FOUND.into_response();
            let headers_mut = response.headers_mut();
            headers_mut.insert(
                header::LOCATION,
                HeaderValue::from_str(&url).map_err(|e| AppError::Internal(e.into()))?,
            );
            headers_mut.insert(header::ETAG, etag_value);
            headers_mut.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            return Ok(response);
        }
        crate::artifact::Download::Absent => {
            return Err(AppError::NotFound(
                "artifact binary missing from storage".to_string(),
            ))
        }
        crate::artifact::Download::Reader(reader) => reader,
    };

    let mut response = Response::new(Body::from_stream(reader.stream));
    if range.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    let headers_mut = response.headers_mut();
    headers_mut.insert(header::ETAG, etag_value);
    headers_mut.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers_mut.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers_mut.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment;filename={}", meta.filename))
            .map_err(|e| AppError::Internal(e.into()))?,
    );

    match range {
        Some((start, end)) => {
            headers_mut.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                    .map_err(|e| AppError::Internal(e.into()))?,
            );
            headers_mut.insert(header::CONTENT_LENGTH, HeaderValue::from(end - start + 1));
        }
        None => {
            headers_mut.insert(header::CONTENT_LENGTH, HeaderValue::from(total));
        }
    }
    Ok(response)
}
