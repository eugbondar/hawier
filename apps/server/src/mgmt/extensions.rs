//! `/rest/v1/system/extensions` — what this node delegates, and to what.
//!
//! Node-scoped, not tenant-scoped: which modules are loaded and what they are
//! configured with is a property of the process, and a control node and a sub
//! node may legitimately differ. So the answer describes *this* node, says so,
//! and claims nothing about any other.
//!
//! On mgmt rather than the internal `/ui/v1` API, deliberately.
//! `authz::permission_for` only classifies `/rest/v1/` paths, so anything on
//! `/ui/v1` is outside authorisation entirely — module paths, versions and
//! capability allowlists would be readable by every logged-in operator
//! regardless of role. Here `system` already maps to READ_SYSTEM/UPDATE_SYSTEM,
//! and `reject_writes_on_sub` already refuses writes on a sub node. Both come
//! for free; neither would exist on the other transport.
//!
//! It also keeps read and write on the same node. `HAWIER_API_URL` and
//! `HAWKBIT_MGMT_URL` are independent settings in the console, so a deployment
//! with a load balancer in front of mgmt could otherwise read one node's drift
//! and write another node's file.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::auth::MgmtIdentity;
use crate::error::{AppError, AppResult};
use crate::ext::params;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rest/v1/system/extensions", get(inventory))
        .route(
            "/rest/v1/system/extensions/parameters/{name}",
            axum::routing::put(set_parameter).delete(delete_parameter),
        )
}

/// What this node has, is configured with, and is running on.
async fn inventory(
    State(state): State<AppState>,
    _identity: MgmtIdentity,
) -> AppResult<Json<Value>> {
    // Read the file again rather than trusting the startup snapshot: the
    // whole point of the drift indicator is that the two can differ.
    let current = params::Resolved::load(file_path(&state).as_deref());
    let started = &state.ext.params;

    let points: Vec<Value> = crate::ext::SLOTS
        .iter()
        .map(|slot| {
            let module = state.ext.slot(slot.key).map(|extension| {
                let manifest = extension.manifest();
                json!({
                    "name": manifest.name,
                    "version": manifest.version,
                    "points": manifest.points,
                    "sites": manifest.sites,
                    "capabilities": manifest.capabilities,
                    // Declared by the module and never used by the host — the
                    // node enforces its own. Shown side by side so the
                    // difference is visible rather than surprising.
                    "declaredDeadlineMs": manifest.deadline_ms,
                    "path": extension.path(),
                })
            });
            json!({
                "key": slot.key,
                "label": slot.label,
                "envVar": slot.env_var,
                "points": slot.points,
                "configured": module.is_some(),
                "absence": slot.absence,
                "module": module,
            })
        })
        .collect();

    let drift = current.drift_from(started);
    let parameters: Vec<Value> = current
        .params()
        .filter(|param| !params::is_reserved(&param.name))
        .map(|param| {
            json!({
                "name": param.name,
                // A credential never leaves the node. Not masked, not in a
                // data attribute, not in an input's value: settable, never
                // readable, the same rule a webhook secret follows.
                "value": if param.secret() { Value::Null } else { json!(param.value) },
                "secret": param.secret(),
                "set": !param.value.is_empty(),
                "origin": param.origin.as_str(),
                "inFile": param.in_file,
                "editable": param.editable(),
                "changedSinceStart": drift.contains(&param.name),
            })
        })
        .collect();

    Ok(Json(json!({
        "node": {
            "id": state.config.node_id,
            "role": match state.config.role {
                crate::config::Role::Control => "control",
                crate::config::Role::Sub => "sub",
            },
            "build": crate::ui_state::BUILD,
            // Which cargo profile this binary was built with. A debug node
            // is perfectly correct and several times slower, so anything
            // measuring throughput has to be able to tell — `scripts/load.sh`
            // refuses to record a run against one.
            "profile": crate::build_profile(),
            // The deadline the host actually applies, for every module.
            "enforcedDeadlineMs": state.config.ext_deadline_ms,
            "paramsFile": state.config.ext_params_file,
            "paramsFileState": current.state.as_str(),
            "paramsFileDetail": current.state.detail(),
            "paramsFileWritable": writable(&current),
            // A sub node answers this truthfully about itself, and refuses
            // writes: parameters are set on the control node.
            "writable": matches!(state.config.role, crate::config::Role::Control),
        },
        "points": points,
        "parameters": parameters,
        "drift": {
            // Unreadable is not the same as unchanged. Saying "0 changes"
            // when the file cannot be read would be a lie the operator acts
            // on.
            // Unconfigured and absent both mean "nothing to read and nothing
            // has changed", which is a quiet, correct answer. Only a file
            // that exists and will not open is unreadable — and there the
            // difference matters, because "0 changes" would be a lie.
            "readable": !matches!(current.state, params::FileState::Unreadable(_) | params::FileState::Malformed(_)),
            "changed": drift.len(),
            "names": drift,
            "detail": current.state.detail(),
        },
    })))
}

/// Set one parameter in the file. Applies when the node restarts.
async fn set_parameter(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let value = extract_value(&body)?;
    let current = guard(&state, &name)?;

    // An empty value means "leave it alone", not "clear it". An HTML text
    // input always submits a value, so clear-on-empty would wipe a credential
    // on any accidental save; clearing is the separate DELETE.
    if value.is_empty() {
        if let Some(existing) = current.get(&name) {
            return Ok(Json(
                json!({ "name": name, "set": !existing.value.is_empty() }),
            ));
        }
        return Err(AppError::BadRequest(format!(
            "'{name}' has no stored value to keep — send one"
        )));
    }

    current.write(&name, Some(&value)).map_err(|error| {
        // Deliberately not Unavailable: that carries Retry-After and means
        // "busy, come back", while an unwritable config file is permanent and
        // the operator needs the incident number.
        AppError::Internal(
            anyhow::Error::from(error).context(format!("writing {}", state.config.ext_params_file)),
        )
    })?;
    // The value itself is never logged: the action log is a readable mgmt
    // resource, and repeating a secret key into it would put it in the
    // database.
    tracing::info!(
        parameter = %name,
        by = %identity.username,
        "extension parameter set; applies when this node restarts"
    );
    Ok(Json(json!({ "name": name, "set": true })))
}

async fn delete_parameter(
    State(state): State<AppState>,
    identity: MgmtIdentity,
    Path(name): Path<String>,
) -> AppResult<axum::http::StatusCode> {
    let current = guard(&state, &name)?;
    if current.get(&name).is_none() {
        return Err(AppError::NotFound(format!("'{name}' is not set")));
    }
    current.write(&name, None).map_err(|error| {
        AppError::Internal(
            anyhow::Error::from(error).context(format!("writing {}", state.config.ext_params_file)),
        )
    })?;
    tracing::info!(
        parameter = %name,
        by = %identity.username,
        "extension parameter removed; applies when this node restarts"
    );
    Ok(axum::http::StatusCode::OK)
}

/// Everything a write has to be true of before it touches the file.
fn guard(state: &AppState, name: &str) -> AppResult<params::Resolved> {
    let path = file_path(state).ok_or_else(|| {
        AppError::BadRequest(
            "this node has no extension parameters file; set HAWIER_EXT_PARAMS_FILE".to_string(),
        )
    })?;
    if !name.starts_with(params::PREFIX)
        || name.len() == params::PREFIX.len()
        || !name[params::PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(AppError::BadRequest(format!(
            "'{name}' is not an extension parameter: a name outside {}[A-Z0-9_]+ \
             would never reach a module",
            params::PREFIX
        )));
    }
    if params::is_reserved(name) {
        return Err(AppError::BadRequest(format!(
            "'{name}' selects which code this node runs and belongs to the \
             deployment, not to this file"
        )));
    }

    let current = params::Resolved::load(Some(&path));
    if let Some(existing) = current.get(name) {
        if existing.origin == params::Origin::Environment {
            return Err(AppError::Conflict(format!(
                "'{name}' is pinned by this node's environment; the file value \
                 would be ignored"
            )));
        }
    }
    Ok(current)
}

fn file_path(state: &AppState) -> Option<std::path::PathBuf> {
    Some(state.config.ext_params_file.trim())
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
}

/// Can this node write the file it was pointed at?
///
/// Answered by trying, not by reading permission bits. `readonly()` reports
/// the mode, which says nothing about whether *this* process may write: a
/// directory owned by root with mode 755 is not "readonly" and is still
/// closed to an unprivileged node — which is exactly the shipped container,
/// where the volume mounts as root and the process runs as `hawier`. The
/// mode-bit version answered "writable" and the console would have offered
/// Save buttons that 500.
///
/// Ownership, mode, a read-only mount and a full filesystem all decide this,
/// and only an attempt sees all four. Two syscalls on a settings page is the
/// right price for not lying.
fn writable(resolved: &params::Resolved) -> bool {
    let Some(path) = resolved.path.as_ref() else {
        return false;
    };
    if path.exists() {
        return std::fs::OpenOptions::new().append(true).open(path).is_ok();
    }
    // The file may not exist yet — a save is allowed to create it, so the
    // question is really about the directory.
    let directory = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let probe = directory.join(format!(
        ".{}.probe",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("params")
    ));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `{"value": "..."}`, or a bare scalar — the same shape
/// `system/configs` accepts, so an operator learns one convention.
fn extract_value(body: &Value) -> AppResult<String> {
    let value = match body {
        Value::Object(map) => map.get("value").unwrap_or(&Value::Null),
        other => other,
    };
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Null => Ok(String::new()),
        _ => Err(AppError::BadRequest(
            "'value' must be a string, number or boolean".to_string(),
        )),
    }
}
