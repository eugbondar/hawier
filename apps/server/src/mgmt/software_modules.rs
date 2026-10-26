//! `/rest/v1/softwaremodules` incl. multipart artifact upload and download.

use std::sync::LazyLock;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use super::repr::{artifact_json, sm_json, SM_SELECT};
use super::{
    body_str, one_or_many, order_by, paged, request_base, require_str, rsql_filter, ListParams,
};
use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::rsql::{bind_values, Field, FieldMap, Kind};
use crate::state::{now_millis, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/softwaremodules", get(list).post(create))
        .route(
            "/rest/v1/softwaremodules/{id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route(
            "/rest/v1/softwaremodules/{id}/artifacts",
            get(list_artifacts).post(upload_artifact),
        )
        .route(
            "/rest/v1/softwaremodules/{id}/artifacts/{artifact_id}",
            get(get_artifact).delete(delete_artifact),
        )
        .route(
            "/rest/v1/softwaremodules/{id}/artifacts/{artifact_id}/download",
            get(download_artifact),
        )
        .route(
            "/rest/v1/softwaremodules/{id}/metadata",
            get(list_metadata).post(create_metadata),
        )
        .route(
            "/rest/v1/softwaremodules/{id}/metadata/{key}",
            get(get_metadata).put(put_metadata).delete(delete_metadata),
        )
}

static SM_FIELDS: LazyLock<FieldMap> = LazyLock::new(|| {
    FieldMap::new(vec![
        (
            "id",
            Field::Column {
                sql: "m.id",
                kind: Kind::Int,
            },
        ),
        (
            "name",
            Field::Column {
                sql: "m.name",
                kind: Kind::Text,
            },
        ),
        (
            "description",
            Field::Column {
                sql: "m.description",
                kind: Kind::Text,
            },
        ),
        (
            "version",
            Field::Column {
                sql: "m.version",
                kind: Kind::Text,
            },
        ),
        (
            "vendor",
            Field::Column {
                sql: "m.vendor",
                kind: Kind::Text,
            },
        ),
        (
            "type",
            Field::Column {
                sql: "t.type_key",
                kind: Kind::Text,
            },
        ),
        (
            // Scalar subquery: filterable (artifactCount=gt=0) and sortable.
            "artifactcount",
            Field::Column {
                sql: "(SELECT COUNT(*) FROM sp_artifact a WHERE a.software_module = m.id)",
                kind: Kind::Int,
            },
        ),
        (
            "locked",
            Field::Column {
                sql: "m.locked",
                kind: Kind::Bool,
            },
        ),
        (
            "createdat",
            Field::Column {
                sql: "m.created_at",
                kind: Kind::Int,
            },
        ),
        (
            "lastmodifiedat",
            Field::Column {
                sql: "m.last_modified_at",
                kind: Kind::Int,
            },
        ),
        (
            "metadata.*",
            Field::KeyValueSub {
                table: "sp_sm_metadata",
                fk: "sm",
                owner: "m.id",
                key_col: "meta_key",
                val_col: "meta_value",
            },
        ),
    ])
});

async fn load_sm(state: &AppState, tenant: &str, id: i64) -> AppResult<sqlx::postgres::PgRow> {
    sqlx::query(&format!(
        "{SM_SELECT} WHERE m.id = $1 AND UPPER(m.tenant) = UPPER($2)"
    ))
    .bind(id)
    .bind(tenant)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("software module {id} not found")))
}

async fn list(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let base = request_base(&headers);
    let (filter_sql, binds) = rsql_filter(params.q.as_deref(), &SM_FIELDS, 1)?;
    let order = order_by(params.sort.as_deref(), &SM_FIELDS, "m.id ASC")?;

    let total: i64 = bind_values(
        sqlx::query(&format!(
            "SELECT COUNT(*) FROM sp_software_module m
             JOIN sp_software_module_type t ON t.id = m.sm_type
             WHERE UPPER(m.tenant) = UPPER($1) AND m.deleted IS NOT TRUE{filter_sql}"
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_one(&state.pool)
    .await?
    .get(0);

    let rows = bind_values(
        sqlx::query(&format!(
            "{SM_SELECT} WHERE UPPER(m.tenant) = UPPER($1) AND m.deleted IS NOT TRUE{filter_sql} \
             {order} LIMIT {} OFFSET {}",
            params.limit()?,
            params.offset()?
        ))
        .bind(&identity.tenant),
        &binds,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(paged(
        rows.iter().map(|r| sm_json(r, &base)).collect(),
        total,
    ))
}

async fn create(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let (items, _) = one_or_many(body)?;
    let now = now_millis();
    let mut created = Vec::new();

    for item in &items {
        let name = require_str(item, "name")?;
        let version = require_str(item, "version")?;
        let type_key = require_str(item, "type")?;
        let sm_type: i64 = sqlx::query_scalar(
            "SELECT id FROM sp_software_module_type
             WHERE type_key = $1 AND UPPER(tenant) = UPPER($2)",
        )
        .bind(&type_key)
        .bind(&identity.tenant)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("software module type '{type_key}' not found"))
        })?;

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO sp_software_module
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, description, name, version, deleted, vendor, sm_type, encrypted, locked)
             VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, FALSE, $7, $8, $9, FALSE)
             RETURNING id",
        )
        .bind(now)
        .bind(&identity.username)
        .bind(&identity.tenant)
        .bind(body_str(item, "description"))
        .bind(&name)
        .bind(&version)
        .bind(body_str(item, "vendor"))
        .bind(sm_type)
        .bind(
            item.get("encrypted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )
        .fetch_one(&state.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
                AppError::Conflict(format!("software module {name}:{version} already exists"))
            }
            _ => AppError::Db(e),
        })?;

        let row = load_sm(&state, &identity.tenant, id).await?;
        created.push(sm_json(&row, &base));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let row = load_sm(&state, &identity.tenant, id).await?;
    Ok(Json(sm_json(&row, &request_base(&headers))))
}

async fn update(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    sqlx::query(
        "UPDATE sp_software_module SET
             description = COALESCE($1, description),
             vendor = COALESCE($2, vendor),
             locked = COALESCE($3, locked),
             last_modified_at = $4, last_modified_by = $5,
             optlock_revision = COALESCE(optlock_revision, 0) + 1
         WHERE id = $6",
    )
    .bind(body_str(&body, "description"))
    .bind(body_str(&body, "vendor"))
    .bind(body.get("locked").and_then(|v| v.as_bool()))
    .bind(now_millis())
    .bind(&identity.username)
    .bind(id)
    .execute(&state.pool)
    .await?;
    let row = load_sm(&state, &identity.tenant, id).await?;
    Ok(Json(sm_json(&row, &request_base(&headers))))
}

async fn delete_one(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    load_sm(&state, &identity.tenant, id).await?;
    sqlx::query("UPDATE sp_software_module SET deleted = TRUE WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

async fn list_artifacts(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    let rows = sqlx::query(
        "SELECT id, created_at, created_by, last_modified_at, last_modified_by,
                sha1_hash, md5_hash, sha256_hash, provided_file_name, file_size
         FROM sp_artifact WHERE software_module = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let base = request_base(&headers);
    Ok(Json(Value::Array(
        rows.iter().map(|r| artifact_json(r, &base, id)).collect(),
    )))
}

#[derive(Deserialize)]
pub struct UploadParams {
    pub filename: Option<String>,
    pub md5sum: Option<String>,
    pub sha1sum: Option<String>,
    pub sha256sum: Option<String>,
}

/// `POST /rest/v1/softwaremodules/{id}/artifacts` — multipart `file` part;
/// optional hash query params are verified against the stored content.
async fn upload_artifact(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<UploadParams>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<Value>)> {
    let sm = load_sm(&state, &identity.tenant, id).await?;
    if sm.get::<bool, _>("locked") {
        return Err(AppError::Conflict(
            "software module is locked and cannot be modified".into(),
        ));
    }

    let field = loop {
        let field = multipart
            .next_field()
            .await
            .map_err(|e| AppError::BadRequest(format!("malformed multipart body: {e}")))?
            .ok_or_else(|| AppError::BadRequest("multipart 'file' part is required".into()))?;
        if field.name() == Some("file") {
            break field;
        }
    };
    let filename = params
        .filename
        .clone()
        .or_else(|| field.file_name().map(str::to_string))
        .ok_or_else(|| AppError::BadRequest("artifact filename is required".into()))?;
    validate_artifact_filename(&filename)?;

    // Stream the part straight into storage, hashing as it goes.
    let stream = field.map(|chunk| chunk.map_err(|e| std::io::Error::other(e.to_string())));
    let stored = state
        .storage
        .store(&identity.tenant, Box::pin(stream))
        .await?;

    let check = |provided: &Option<String>, actual: &str, kind: &str| -> AppResult<()> {
        if let Some(expected) = provided {
            if !expected.trim().eq_ignore_ascii_case(actual) {
                return Err(AppError::BadRequest(format!(
                    "{kind} hash mismatch: expected {expected}, got {actual}"
                )));
            }
        }
        Ok(())
    };
    if let Err(error) = check(&params.sha1sum, &stored.sha1, "SHA1")
        .and_then(|_| check(&params.md5sum, &stored.md5, "MD5"))
        .and_then(|_| check(&params.sha256sum, &stored.sha256, "SHA256"))
    {
        // Only drop the blob if nothing else references it.
        let referenced: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sp_artifact WHERE sha1_hash = $1")
                .bind(&stored.sha1)
                .fetch_one(&state.pool)
                .await?;
        if referenced == 0 {
            state
                .storage
                .delete(&identity.tenant, &stored.sha1)
                .await
                .ok();
        }
        return Err(error);
    }

    let now = now_millis();
    // The row and its "uploaded" event commit together: a subscriber must
    // never hear about an artifact the database does not have.
    let mut tx = state.pool.begin().await?;
    let artifact_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_artifact
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, md5_hash, file_size, provided_file_name, sha1_hash, software_module, sha256_hash)
         VALUES ($1, $2, $1, $2, 1, UPPER($3), $4, $5, $6, $7, $8, $9)
         RETURNING id",
    )
    .bind(now)
    .bind(&identity.username)
    .bind(&identity.tenant)
    .bind(&stored.md5)
    .bind(stored.size)
    .bind(&filename)
    .bind(&stored.sha1)
    .bind(id)
    .bind(&stored.sha256)
    .fetch_one(&mut *tx)
    .await?;

    crate::events::emit(
        &mut tx,
        &state.incidents,
        &identity.tenant,
        hawier_api::ext::kinds::ARTIFACT_UPLOADED,
        serde_json::json!({
            "artifactId": artifact_id,
            "softwareModuleId": id,
            "filename": filename,
            "sha1": stored.sha1,
            "size": stored.size,
            "uploadedBy": identity.username,
        }),
    )
    .await?;
    tx.commit().await?;

    let row = sqlx::query(
        "SELECT id, created_at, created_by, last_modified_at, last_modified_by,
                sha1_hash, md5_hash, sha256_hash, provided_file_name, file_size
         FROM sp_artifact WHERE id = $1",
    )
    .bind(artifact_id)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(artifact_json(&row, &request_base(&headers), id)),
    ))
}

async fn get_artifact(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, artifact_id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    let row = sqlx::query(
        "SELECT id, created_at, created_by, last_modified_at, last_modified_by,
                sha1_hash, md5_hash, sha256_hash, provided_file_name, file_size
         FROM sp_artifact WHERE id = $1 AND software_module = $2",
    )
    .bind(artifact_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("artifact {artifact_id} not found")))?;
    Ok(Json(artifact_json(&row, &request_base(&headers), id)))
}

async fn delete_artifact(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, artifact_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    load_sm(&state, &identity.tenant, id).await?;
    let mut tx = state.pool.begin().await?;
    let sha1: Option<String> = sqlx::query_scalar(
        "DELETE FROM sp_artifact WHERE id = $1 AND software_module = $2 RETURNING sha1_hash",
    )
    .bind(artifact_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(sha1) = sha1 else {
        return Err(AppError::NotFound(format!(
            "artifact {artifact_id} not found"
        )));
    };
    // Content-addressed storage is shared: only delete the blob when the
    // last row referencing it is gone.
    let referenced: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sp_artifact WHERE sha1_hash = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(&sha1)
    .bind(&identity.tenant)
    .fetch_one(&mut *tx)
    .await?;

    crate::events::emit(
        &mut tx,
        &state.incidents,
        &identity.tenant,
        hawier_api::ext::kinds::ARTIFACT_DELETED,
        serde_json::json!({
            "artifactId": artifact_id,
            "softwareModuleId": id,
            "sha1": sha1,
            "blobRemoved": referenced == 0,
            "deletedBy": identity.username,
        }),
    )
    .await?;
    // The row is gone for good only once this commits; the blob follows,
    // never the other way round.
    tx.commit().await?;

    if referenced == 0 {
        state.storage.delete(&identity.tenant, &sha1).await.ok();
    }
    Ok(StatusCode::OK)
}

async fn download_artifact(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, artifact_id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    load_sm(&state, &identity.tenant, id).await?;
    let row = sqlx::query(
        "SELECT sha1_hash, provided_file_name, file_size FROM sp_artifact
         WHERE id = $1 AND software_module = $2",
    )
    .bind(artifact_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("artifact {artifact_id} not found")))?;

    let sha1: String = row.get("sha1_hash");
    let filename: Option<String> = row.get("provided_file_name");
    let size = row.get::<Option<i64>, _>("file_size").unwrap_or(0).max(0) as u64;

    // Multi-gigabyte images over flaky links need resumable downloads, and
    // the browser asks for them by itself. The storage layer already takes a
    // range; the DDI endpoint has parsed one for a while, and this reuses that
    // parser rather than growing a second dialect of the same header.
    let range = crate::ddi::artifacts::parse_range(&headers, size)?;
    let reader = state
        .storage
        .open(&identity.tenant, &sha1, range)
        .await?
        .ok_or_else(|| AppError::NotFound("artifact binary missing from storage".into()))?;

    let mut response = Response::new(axum::body::Body::from_stream(reader.stream));
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    out.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{sha1}\"")).map_err(|e| AppError::Internal(e.into()))?,
    );
    match range {
        Some((start, end)) => {
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            let out = response.headers_mut();
            out.insert(header::CONTENT_LENGTH, HeaderValue::from(end - start + 1));
            out.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
                    .map_err(|e| AppError::Internal(e.into()))?,
            );
        }
        None => {
            response
                .headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from(size));
        }
    }
    let out = response.headers_mut();
    if let Some(filename) = filename {
        out.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&content_disposition(&filename))
                .map_err(|e| AppError::Internal(e.into()))?,
        );
    }
    Ok(response.into_response())
}

/// The width of `sp_artifact.provided_file_name`.
const MAX_ARTIFACT_FILENAME: usize = 256;

/// Upload refuses a name it could not serve back everywhere: DDI addresses
/// the artifact by its name as one path segment and builds
/// `Content-Disposition` without RFC 6266, so a non-ASCII name answered 500
/// there and a slash made it unreachable. hawkBit accepts any name; this is
/// a deliberate divergence (API_NOTES.md). Checked before a byte is stored.
fn validate_artifact_filename(name: &str) -> AppResult<()> {
    let refuse = |why: &str| Err(AppError::BadRequest(format!("artifact filename {why}")));
    if name.trim().is_empty() {
        return refuse("must not be empty");
    }
    if name.len() > MAX_ARTIFACT_FILENAME {
        return refuse(&format!("must not exceed {MAX_ARTIFACT_FILENAME} bytes"));
    }
    if !name.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return refuse("must be printable ASCII");
    }
    if name.contains(['/', '\\']) {
        return refuse("must not contain '/' or '\\'");
    }
    Ok(())
}

/// `attachment; filename="…"; filename*=UTF-8''…` (RFC 6266).
///
/// A header value may hold only visible ASCII, so a non-ASCII artifact name
/// used to fail `HeaderValue::from_str` and turn the download into a 500.
/// Upload no longer accepts such names, but a database migrated from hawkBit
/// may still hold them. The quoted form carries an ASCII fallback for old
/// clients, the starred form carries the real name.
fn content_disposition(filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = filename
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

// ---------------------------------------------------------------------------
// Metadata (SM metadata carries the extra `targetVisible` flag)
// ---------------------------------------------------------------------------

async fn list_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Query(params): Query<ListParams>,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sp_sm_metadata WHERE sm = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query(
        "SELECT meta_key, meta_value, target_visible FROM sp_sm_metadata
         WHERE sm = $1 ORDER BY meta_key LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(params.limit()?)
    .bind(params.offset()?)
    .fetch_all(&state.pool)
    .await?;
    let content: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "key": r.get::<String, _>("meta_key"),
                "value": r.get::<Option<String>, _>("meta_value"),
                "targetVisible": r.get::<Option<bool>, _>("target_visible").unwrap_or(false),
            })
        })
        .collect();
    Ok(paged(content, total))
}

async fn create_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    load_sm(&state, &identity.tenant, id).await?;
    let (items, _) = one_or_many(body)?;
    let mut created = Vec::new();
    for item in &items {
        let key = require_str(item, "key")?;
        let value = body_str(item, "value").unwrap_or_default();
        let target_visible = item
            .get("targetVisible")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        sqlx::query(
            "INSERT INTO sp_sm_metadata (sm, meta_key, meta_value, target_visible)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (meta_key, sm) DO UPDATE
                 SET meta_value = EXCLUDED.meta_value,
                     target_visible = EXCLUDED.target_visible",
        )
        .bind(id)
        .bind(&key)
        .bind(&value)
        .bind(target_visible)
        .execute(&state.pool)
        .await?;
        created.push(json!({ "key": key, "value": value, "targetVisible": target_visible }));
    }
    Ok((StatusCode::CREATED, Json(Value::Array(created))))
}

async fn get_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    let row = sqlx::query(
        "SELECT meta_key, meta_value, target_visible FROM sp_sm_metadata
         WHERE sm = $1 AND meta_key = $2",
    )
    .bind(id)
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("metadata '{key}' not found")))?;
    Ok(Json(json!({
        "key": row.get::<String, _>("meta_key"),
        "value": row.get::<Option<String>, _>("meta_value"),
        "targetVisible": row.get::<Option<bool>, _>("target_visible").unwrap_or(false),
    })))
}

async fn put_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    load_sm(&state, &identity.tenant, id).await?;
    let value = body_str(&body, "value").unwrap_or_default();
    let target_visible = body.get("targetVisible").and_then(|v| v.as_bool());
    // COALESCE keeps the stored flag when the caller omits it, so the answer
    // has to come back from the row — reporting the request's own `false`
    // told the caller the flag had been cleared when it had not.
    let updated: Option<bool> = sqlx::query_scalar(
        "UPDATE sp_sm_metadata
         SET meta_value = $1, target_visible = COALESCE($2, target_visible)
         WHERE sm = $3 AND meta_key = $4
         RETURNING target_visible",
    )
    .bind(&value)
    .bind(target_visible)
    .bind(id)
    .bind(&key)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("metadata '{key}' not found")))?;
    Ok(Json(json!({
        "key": key,
        "value": value,
        "targetVisible": updated.unwrap_or(false),
    })))
}

async fn delete_metadata(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path((id, key)): Path<(i64, String)>,
) -> AppResult<StatusCode> {
    load_sm(&state, &identity.tenant, id).await?;
    let result = sqlx::query("DELETE FROM sp_sm_metadata WHERE sm = $1 AND meta_key = $2")
        .bind(id)
        .bind(&key)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("metadata '{key}' not found")));
    }
    Ok(StatusCode::OK)
}
