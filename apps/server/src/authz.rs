//! Authorisation: what an identity may do.
//!
//! The host owns the database, so it looks up the roles a user holds and the
//! permissions those roles carry. The decision is an extension's
//! (`apps/ext-authz` implements plain RBAC), because the interesting policies
//! are the ones a lookup cannot express — a time window, a protected tenant,
//! an approval requirement — and those belong in a module rather than here.
//!
//! Two things stay firmly on this side. Enforcement: a refusal is a 403 from
//! the host, not something an extension can be talked out of. And the
//! opt-in: a tenant with no roles defined is unconfigured, and unconfigured
//! means everything is allowed, because switching hawier on must not lock a
//! working deployment out of itself.
//!
//! Answers are cached per (tenant, user, permission) for a few seconds. Every
//! mgmt request asks at least once, and a wasm call per request per check
//! would show up.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hawier_api::ext::{permissions, points, AuthzRequest, AuthzVerdict};
use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::ext;
use crate::state::AppState;

/// How long a verdict stays good. Short enough that revoking a role takes
/// effect while someone is still looking at the screen.
const CACHE_TTL: Duration = Duration::from_secs(5);

type CacheKey = (String, String, String);
type Entries = HashMap<CacheKey, (Instant, bool)>;

/// Verdicts already decided. It hangs off `AppState` and not off a static,
/// because the key cannot tell two nodes apart: a process-wide map lets one
/// node's refusal for (`DEFAULT`, `admin`, …) answer for another node
/// entirely. The integration suite is where that bites first — every test
/// there authenticates as `admin` in `DEFAULT` — and the DDI path is anyway
/// required not to depend on process singletons.
#[derive(Clone, Default)]
pub struct VerdictCache {
    entries: Arc<Mutex<Entries>>,
}

impl VerdictCache {
    fn get(&self, key: &CacheKey) -> Option<bool> {
        let mut entries = self.entries.lock().ok()?;
        match entries.get(key) {
            Some((at, allowed)) if at.elapsed() < CACHE_TTL => Some(*allowed),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn put(&self, key: CacheKey, allowed: bool) {
        if let Ok(mut entries) = self.entries.lock() {
            // A cache that only ever grows is a leak; this is bounded by the
            // number of (tenant, user, permission) triples in play, but a
            // burst of unknown users should not pin it.
            if entries.len() > 10_000 {
                entries.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            }
            entries.insert(key, (Instant::now(), allowed));
        }
    }

    /// Forget everything, so a role change takes effect at once rather than
    /// after the TTL.
    pub fn invalidate(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

/// The roles a user holds in a tenant, and everything those roles carry.
async fn holdings(
    state: &AppState,
    tenant: &str,
    username: &str,
) -> AppResult<(Vec<String>, Vec<String>, bool)> {
    // Enforcement begins with the first *grant*, not the first role. A role
    // is a definition; a grant is the moment a person is actually subject to
    // one. Counting roles put the switch between defining a role and
    // assigning it — and since assigning is itself a mgmt write, the operator
    // who turned authorisation on was refused before they could grant
    // themselves anything, with no way back except editing the table by hand.
    let assigned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hawier_grant WHERE UPPER(tenant) = UPPER($1)")
            .bind(tenant)
            .fetch_one(&state.pool)
            .await?;
    if assigned == 0 {
        return Ok((Vec::new(), Vec::new(), true));
    }

    let rows = sqlx::query(
        "SELECT r.name, r.permissions FROM hawier_grant g
         JOIN hawier_role r ON r.id = g.role
         WHERE UPPER(g.tenant) = UPPER($1) AND g.username = $2",
    )
    .bind(tenant)
    .bind(username)
    .fetch_all(&state.pool)
    .await?;

    let mut roles = Vec::new();
    let mut granted = Vec::new();
    for row in &rows {
        roles.push(row.get::<String, _>("name"));
        for permission in row.get::<String, _>("permissions").split(',') {
            let permission = permission.trim();
            if !permission.is_empty() && !granted.iter().any(|p| p == permission) {
                granted.push(permission.to_string());
            }
        }
    }
    Ok((roles, granted, false))
}

/// May this identity do this? Refusal is a 403 carrying the extension's own
/// explanation, because "forbidden" alone sends an operator hunting.
pub async fn require(
    state: &AppState,
    identity: &crate::auth::MgmtIdentity,
    permission: &str,
) -> AppResult<()> {
    let key = (
        identity.tenant.to_uppercase(),
        identity.username.clone(),
        permission.to_string(),
    );
    if let Some(allowed) = state.authz_cache.get(&key) {
        return if allowed {
            Ok(())
        } else {
            Err(AppError::Forbidden(format!(
                "{} is not permitted to {permission}",
                identity.username
            )))
        };
    }

    let (roles, granted, unconfigured) =
        holdings(state, &identity.tenant, &identity.username).await?;

    let verdict = match state.ext.authz.as_deref() {
        Some(extension) => {
            ext::call::<_, AuthzVerdict>(
                extension,
                points::AUTHZ_DECIDE,
                &AuthzRequest {
                    username: identity.username.clone(),
                    tenant: identity.tenant.clone(),
                    roles,
                    granted,
                    permission: permission.to_string(),
                    unconfigured,
                },
            )
            .await?
        }
        // No policy module. Not a lockout: a node that was never told how to
        // decide has not been told to refuse either, and the alternative is
        // an installation nobody can use.
        None => AuthzVerdict {
            allow: true,
            reason: "no authorisation extension configured".to_string(),
        },
    };

    state.authz_cache.put(key, verdict.allow);
    if verdict.allow {
        Ok(())
    } else {
        Err(AppError::Forbidden(if verdict.reason.is_empty() {
            format!("{} is not permitted to {permission}", identity.username)
        } else {
            verdict.reason
        }))
    }
}

/// Everything this identity may do, for `GET /rest/v1/userinfo`. The console
/// hides what an operator cannot use rather than offering a button that 403s.
pub async fn permissions_of(
    state: &AppState,
    identity: &crate::auth::MgmtIdentity,
) -> AppResult<Vec<String>> {
    let (_, granted, unconfigured) = holdings(state, &identity.tenant, &identity.username).await?;
    if unconfigured || state.ext.authz.is_none() {
        return Ok(vec![permissions::ALL.to_string()]);
    }
    Ok(granted)
}

// ---------------------------------------------------------------------------
// Enforcement
// ---------------------------------------------------------------------------

/// Which permission a request needs, from its method and path.
///
/// A table rather than a guard at every handler: forty hand-placed checks
/// drift, and the one that gets forgotten is a hole nobody sees. Anything
/// unrecognised falls back to the repository permissions, so a new endpoint
/// is guarded by default instead of open by default.
///
/// `None` means the request is not subject to authorisation at all — only
/// `userinfo`, which is how the console discovers what it may show.
pub fn permission_for(method: &axum::http::Method, path: &str) -> Option<&'static str> {
    use axum::http::Method;
    use hawier_api::ext::permissions as perm;

    let rest = path.strip_prefix("/rest/v1/")?;
    let mut segments = rest.split('/');
    let resource = segments.next().unwrap_or_default();
    if resource == "userinfo" {
        return None;
    }

    let reading = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    Some(match resource {
        // hawier's own machinery: subscriptions, retention, roles, tenant
        // configuration. Separate from the repository so a release engineer
        // is not automatically an administrator.
        "webhooks" | "webhookdeliveries" | "retentionrules" | "roles" | "grants" | "system"
        | "actionlog" | "incidents" => {
            if reading {
                perm::READ_SYSTEM
            } else {
                perm::UPDATE_SYSTEM
            }
        }
        "rollouts" => {
            // `POST /rollouts/{id}/start` is running a rollout, not creating
            // one — a different job, and often a different person.
            let acting = !reading && segments.clone().count() >= 2;
            match (*method).clone() {
                _ if reading => perm::READ_ROLLOUT,
                _ if acting => perm::HANDLE_ROLLOUT,
                Method::POST => perm::CREATE_ROLLOUT,
                Method::DELETE => perm::DELETE_ROLLOUT,
                _ => perm::UPDATE_ROLLOUT,
            }
        }
        "targets" | "targetfilters" | "targetgroups" | "targetgroupdetails" => {
            match (*method).clone() {
                _ if reading => perm::READ_TARGET,
                Method::POST => perm::CREATE_TARGET,
                Method::DELETE => perm::DELETE_TARGET,
                _ => perm::UPDATE_TARGET,
            }
        }
        // Distribution sets, software modules, artifacts, tags, types — and
        // anything added later that nobody remembered to classify.
        _ => match (*method).clone() {
            _ if reading => perm::READ_REPOSITORY,
            Method::POST => perm::CREATE_REPOSITORY,
            Method::DELETE => perm::DELETE_REPOSITORY,
            _ => perm::UPDATE_REPOSITORY,
        },
    })
}

/// Mgmt-wide authorisation layer.
///
/// Runs before the handler, so a refusal never reaches the code that would
/// have done the work. Authentication is re-checked here rather than shared
/// with the extractor because middleware cannot borrow one — it is a
/// constant-time comparison against the configured users, not a round-trip.
pub async fn enforce(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let Some(permission) = permission_for(request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    // An unauthenticated request is the extractor's business; here it simply
    // has no identity to authorise, so let it through to be refused there.
    let Ok(identity) = crate::auth::verify_basic(&state, request.headers()) else {
        return next.run(request).await;
    };
    if let Err(error) = require(&state, &identity, permission).await {
        return error.into_response();
    }
    next.run(request).await
}
