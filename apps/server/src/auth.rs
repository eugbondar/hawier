//! Authentication: Mgmt HTTP Basic (config users) and DDI token schemes.

use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap};
use base64::Engine as _;
use sqlx::PgPool;
use subtle::ConstantTimeEq;

use crate::db::model::TargetRow;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::tenant;

fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Mgmt API: HTTP Basic against statically configured users
// ---------------------------------------------------------------------------

/// Authenticated Mgmt API identity.
#[derive(Debug, Clone)]
pub struct MgmtIdentity {
    pub username: String,
    pub tenant: String,
}

pub fn verify_basic(state: &AppState, headers: &HeaderMap) -> AppResult<MgmtIdentity> {
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let encoded = header
        .strip_prefix("Basic ")
        .ok_or(AppError::Unauthorized)?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| AppError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| AppError::Unauthorized)?;
    let (username, password) = decoded.split_once(':').ok_or(AppError::Unauthorized)?;

    // Optional tenant override, as hawier-ui sends it today.
    let tenant_header = headers
        .get("X-Hawkbit-Tenant")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|t| !t.is_empty());

    let mut matched = None;
    for user in &state.config.mgmt_users {
        // Constant-time compare over both fields, no early exit on username.
        let ok = ct_eq(&user.username, username) & ct_eq(&user.password, password);
        if ok {
            matched = Some(user);
        }
    }
    let user = matched.ok_or(AppError::Unauthorized)?;
    let tenant = tenant_header.unwrap_or(&user.tenant).to_uppercase();
    Ok(MgmtIdentity {
        username: user.username.clone(),
        tenant,
    })
}

impl FromRequestParts<AppState> for MgmtIdentity {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let identity = verify_basic(state, &parts.headers)?;
        // Implicit tenant creation on first mgmt access (hawkBit monolith
        // behaviour with implicitTenantCreateAllowed=true).
        tenant::ensure_tenant(&state.pool, &identity.tenant).await?;
        Ok(identity)
    }
}

// ---------------------------------------------------------------------------
// Internal /ui/v1 API: shared bearer token
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct InternalIdentity;

impl FromRequestParts<AppState> for InternalIdentity {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;
        if ct_eq(token.trim(), &state.config.internal_token) {
            Ok(InternalIdentity)
        } else {
            Err(AppError::Unauthorized)
        }
    }
}

// ---------------------------------------------------------------------------
// DDI: TargetToken / GatewayToken (per-tenant toggles in tenant config)
// ---------------------------------------------------------------------------

/// Result of DDI authentication for `/{tenant}/controller/v1/{controllerId}`.
#[derive(Debug)]
pub enum DdiActor {
    /// Authenticated via its own security token; row loaded.
    Target(Box<TargetRow>),
    /// Authenticated via tenant gateway token (target may not exist yet —
    /// plug-and-play registration is allowed).
    Gateway(Option<Box<TargetRow>>),
    /// Anonymous access allowed by server config (dev only).
    Anonymous(Option<Box<TargetRow>>),
}

impl DdiActor {
    pub fn target(&self) -> Option<&TargetRow> {
        match self {
            DdiActor::Target(t) => Some(t),
            DdiActor::Gateway(t) | DdiActor::Anonymous(t) => t.as_deref(),
        }
    }

    /// Plug-and-play registration allowed for this actor?
    pub fn may_register(&self) -> bool {
        !matches!(self, DdiActor::Target(_))
    }
}

async fn load_target(
    pool: &PgPool,
    tenant: &str,
    controller_id: &str,
) -> AppResult<Option<TargetRow>> {
    Ok(sqlx::query_as::<_, TargetRow>(
        "SELECT id, tenant, controller_id, name, description, sec_token, address,
                last_target_query, install_date, request_controller_attributes,
                update_status, target_group, assigned_distribution_set,
                installed_distribution_set, target_type,
                created_at, created_by, last_modified_at, last_modified_by
         FROM sp_target WHERE controller_id = $1 AND UPPER(tenant) = UPPER($2)",
    )
    .bind(controller_id)
    .bind(tenant)
    .fetch_optional(pool)
    .await?)
}

/// Authenticate a DDI request. `tenant` and `controller_id` come from the
/// URL path; the scheme toggles are per-tenant config with server-env
/// defaults.
pub async fn authenticate_ddi(
    state: &AppState,
    tenant: &str,
    controller_id: &str,
    headers: &HeaderMap,
) -> AppResult<DdiActor> {
    let pool = &state.ddi_pool;
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(token) = authorization.strip_prefix("TargetToken ") {
        let enabled = tenant::cached_bool(
            state,
            tenant,
            tenant::KEY_TARGETTOKEN_ENABLED,
            state.config.ddi_targettoken_default,
        )
        .await?;
        if enabled {
            if let Some(target) = load_target(pool, tenant, controller_id).await? {
                if ct_eq(token.trim(), &target.sec_token) {
                    return Ok(DdiActor::Target(Box::new(target)));
                }
            }
        }
        return Err(AppError::Unauthorized);
    }

    if let Some(token) = authorization.strip_prefix("GatewayToken ") {
        let enabled = tenant::cached_bool(
            state,
            tenant,
            tenant::KEY_GATEWAYTOKEN_ENABLED,
            state.config.ddi_gatewaytoken_default,
        )
        .await?;
        let key = tenant::cached_value(state, tenant, tenant::KEY_GATEWAYTOKEN_KEY, None)
            .await?
            .unwrap_or_default();
        if enabled && !key.is_empty() && ct_eq(token.trim(), &key) {
            let target = load_target(pool, tenant, controller_id).await?;
            return Ok(DdiActor::Gateway(target.map(Box::new)));
        }
        return Err(AppError::Unauthorized);
    }

    if state.config.ddi_anonymous {
        let target = load_target(pool, tenant, controller_id).await?;
        return Ok(DdiActor::Anonymous(target.map(Box::new)));
    }

    Err(AppError::Unauthorized)
}
