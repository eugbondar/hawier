//! hawier authorisation extension: plain role-based access control.
//!
//! The host looks up which roles a user holds and what those roles carry —
//! it owns the database — and this decides. For plain RBAC that decision is
//! a set membership test, which is exactly the point: the interesting
//! policies are the ones that are *not* a lookup, and they belong in a
//! module like this rather than in the node.
//!
//! Two rules it implements, both deliberate:
//!
//! - a tenant that has defined no roles gets everything. Enforcement is
//!   something an operator switches on; upgrading hawier must not lock a
//!   working deployment out of itself.
//! - `ROLE_ALL` is a permission like any other, held by config users and by
//!   whatever role an operator chooses to give it.

use hawier_api::ext::{
    permissions, points, AuthzRequest, AuthzVerdict, ExecutionSite, ExtManifest, ExtRequest,
    ExtResponse,
};

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-authz".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::AUTHZ_DECIDE.to_string()],
        // Every mgmt request, so it has to be cheap and cacheable.
        sites: vec![ExecutionSite::Request],
        capabilities: Vec::new(),
        deadline_ms: 500,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::AUTHZ_DECIDE {
        return ExtResponse::Err {
            message: format!("hawier-ext-authz does not implement {}", request.point),
        };
    }
    match serde_json::from_value::<AuthzRequest>(request.payload) {
        Ok(ask) => match serde_json::to_value(decide(&ask)) {
            Ok(payload) => ExtResponse::Ok { payload },
            Err(error) => ExtResponse::Err {
                message: format!("cannot encode the verdict: {error}"),
            },
        },
        Err(error) => ExtResponse::Err {
            message: format!("unusable authz.decide payload: {error}"),
        },
    }
}

fn decide(ask: &AuthzRequest) -> AuthzVerdict {
    if ask.unconfigured {
        return AuthzVerdict {
            allow: true,
            reason: "no roles defined for this tenant".to_string(),
        };
    }
    if ask.granted.iter().any(|p| p == permissions::ALL) {
        return AuthzVerdict {
            allow: true,
            reason: String::new(),
        };
    }
    if ask.granted.iter().any(|p| p == &ask.permission) {
        return AuthzVerdict {
            allow: true,
            reason: String::new(),
        };
    }
    AuthzVerdict {
        allow: false,
        reason: if ask.roles.is_empty() {
            format!("{} holds no roles in {}", ask.username, ask.tenant)
        } else {
            format!(
                "{} needs {}; holds {}",
                ask.username,
                ask.permission,
                ask.roles.join(", ")
            )
        },
    }
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(granted: &[&str], permission: &str, unconfigured: bool) -> AuthzRequest {
        AuthzRequest {
            username: "bob".to_string(),
            tenant: "DEFAULT".to_string(),
            roles: vec!["operator".to_string()],
            granted: granted.iter().map(|p| p.to_string()).collect(),
            permission: permission.to_string(),
            unconfigured,
        }
    }

    #[test]
    fn a_tenant_with_no_roles_is_not_locked_out() {
        // Upgrading must not brick a working deployment; RBAC is switched on
        // by defining a role, not by installing a version.
        let verdict = decide(&ask(&[], permissions::DELETE_TARGET, true));
        assert!(verdict.allow);
    }

    #[test]
    fn a_held_permission_is_allowed() {
        let verdict = decide(&ask(
            &[permissions::DELETE_TARGET],
            permissions::DELETE_TARGET,
            false,
        ));
        assert!(verdict.allow);
    }

    #[test]
    fn a_missing_permission_is_refused_and_says_what_was_needed() {
        let verdict = decide(&ask(
            &[permissions::READ_TARGET],
            permissions::DELETE_TARGET,
            false,
        ));
        assert!(!verdict.allow);
        assert!(
            verdict.reason.contains("DELETE_TARGET"),
            "{}",
            verdict.reason
        );
        assert!(verdict.reason.contains("operator"), "{}", verdict.reason);
    }

    #[test]
    fn role_all_covers_everything() {
        for permission in permissions::KNOWN {
            assert!(decide(&ask(&[permissions::ALL], permission, false)).allow);
        }
    }

    #[test]
    fn reading_does_not_imply_deleting() {
        // The whole point of the model: a read-only role stays read-only.
        let reader = [permissions::READ_TARGET, permissions::READ_REPOSITORY];
        assert!(decide(&ask(&reader, permissions::READ_TARGET, false)).allow);
        assert!(!decide(&ask(&reader, permissions::UPDATE_TARGET, false)).allow);
        assert!(!decide(&ask(&reader, permissions::DELETE_REPOSITORY, false)).allow);
    }

    #[test]
    fn holding_no_roles_at_all_is_refused_clearly() {
        let mut request = ask(&[], permissions::READ_TARGET, false);
        request.roles.clear();
        let verdict = decide(&request);
        assert!(!verdict.allow);
        assert!(
            verdict.reason.contains("holds no roles"),
            "{}",
            verdict.reason
        );
    }
}
