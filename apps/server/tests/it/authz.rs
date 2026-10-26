//! Authorisation: who may do what. Run via `make it`.
//!
//! The interesting cases are the refusals and the two defaults, because a
//! permission system that is wrong in the permissive direction is not a
//! permission system.

use crate::common;

use hawier_api::ext::permissions;
use serde_json::{json, Value};

async fn role(server: &common::TestServer, name: &str, permissions: &str) -> i64 {
    let id = server.state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_role (id, tenant, name, permissions)
         VALUES ($1, 'DEFAULT', $2, $3)",
    )
    .bind(id)
    .bind(name)
    .bind(permissions)
    .execute(&server.state.pool)
    .await
    .unwrap();
    server.state.authz_cache.invalidate();
    id
}

async fn grant(server: &common::TestServer, username: &str, role_id: i64) {
    sqlx::query(
        "INSERT INTO hawier_grant (id, tenant, username, role) VALUES ($1, 'DEFAULT', $2, $3)",
    )
    .bind(server.state.incidents.next())
    .bind(username)
    .bind(role_id)
    .execute(&server.state.pool)
    .await
    .unwrap();
    server.state.authz_cache.invalidate();
}

async fn call(server: &common::TestServer, method: reqwest::Method, path: &str) -> u16 {
    common::http()
        .request(method, format!("{}{path}", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!([]))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore]
async fn a_tenant_with_no_roles_keeps_working() {
    // Upgrading hawier must not lock an existing deployment out of itself:
    // enforcement starts when an operator defines a role, not on install.
    let server = common::spawn().await;
    server.state.authz_cache.invalidate();

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        200
    );
    assert_eq!(
        call(&server, reqwest::Method::POST, "/rest/v1/targets").await,
        201,
        "creating is allowed while nothing is configured"
    );
}

#[tokio::test]
#[ignore]
async fn defining_a_role_is_not_yet_enforcement() {
    // A role is a definition. Nobody is subject to it until it is assigned,
    // and until then the tenant is still unconfigured — which is what makes
    // it possible to define roles through the API at all.
    let server = common::spawn().await;
    role(&server, "reader", permissions::READ_TARGET).await;

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        200
    );
}

#[tokio::test]
#[ignore]
async fn granting_a_role_starts_enforcing() {
    let server = common::spawn().await;
    let reader = role(&server, "reader", permissions::READ_TARGET).await;
    // Assigned to somebody else: admin is now subject to a configured tenant
    // and holds nothing in it.
    grant(&server, "release-bot", reader).await;

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        403,
        "the first grant switches enforcement on for the whole tenant"
    );
}

/// The bootstrap, over HTTP, exactly as an operator does it.
///
/// This is the sequence that used to brick a tenant: enforcement flipped on
/// with the first *role*, so the grant that would have given the operator
/// their own access was already refused — and every route back was refused
/// too. Nothing caught it because every other test here writes the rows
/// straight to SQL.
#[tokio::test]
#[ignore]
async fn an_operator_can_turn_authorisation_on_over_the_api() {
    let server = common::spawn().await;
    server.state.authz_cache.invalidate();

    let created: Value = common::http()
        .post(format!("{}/rest/v1/roles", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({"name": "administrator", "permissions": permissions::ALL}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let role_id = created["id"].as_str().expect("role id").to_string();

    let granted = common::http()
        .post(format!("{}/rest/v1/grants", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({"username": "admin", "role": role_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        granted.status().as_u16(),
        201,
        "the operator must be able to grant themselves the role they just \
         defined — otherwise defining one is a one-way door"
    );

    server.state.authz_cache.invalidate();
    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        200,
        "and they still hold the tenant afterwards"
    );
}

/// The other direction: authorisation may be switched on, but not switched
/// on around everybody.
#[tokio::test]
#[ignore]
async fn a_tenant_cannot_be_left_without_an_administrator() {
    let server = common::spawn().await;
    server.state.authz_cache.invalidate();

    let reader: Value = common::http()
        .post(format!("{}/rest/v1/roles", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({"name": "reader", "permissions": permissions::READ_TARGET}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let refused = common::http()
        .post(format!("{}/rest/v1/grants", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&json!({"username": "release-bot", "role": reader["id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        refused.status().as_u16(),
        400,
        "this grant would switch enforcement on with nobody able to administer"
    );

    server.state.authz_cache.invalidate();
    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        200,
        "and the refusal left the tenant as it was, not half-configured"
    );
}

#[tokio::test]
#[ignore]
async fn a_reader_reads_but_does_not_write() {
    let server = common::spawn().await;
    let reader = role(&server, "reader", "READ_TARGET,READ_REPOSITORY").await;
    grant(&server, "admin", reader).await;

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/targets").await,
        200
    );
    assert_eq!(
        call(&server, reqwest::Method::POST, "/rest/v1/targets").await,
        403,
        "reading a resource is not permission to change it"
    );
    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/distributionsets").await,
        200
    );
}

#[tokio::test]
#[ignore]
async fn running_a_rollout_is_not_creating_one() {
    let server = common::spawn().await;
    // Someone who may start and stop rollouts, but not author them.
    let operator = role(&server, "operator", "READ_ROLLOUT,HANDLE_ROLLOUT").await;
    grant(&server, "admin", operator).await;

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/rollouts").await,
        200
    );
    assert_eq!(
        call(&server, reqwest::Method::POST, "/rest/v1/rollouts").await,
        403,
        "creating a rollout is a different job from running one"
    );
    // The action path is HANDLE, and it gets past authorisation — what it
    // answers afterwards is the rollout's own business, not a 403.
    assert_ne!(
        call(&server, reqwest::Method::POST, "/rest/v1/rollouts/1/start").await,
        403
    );
}

#[tokio::test]
#[ignore]
async fn system_resources_need_their_own_permission() {
    let server = common::spawn().await;
    // A full repository role does not make someone an administrator.
    let releaser = role(
        &server,
        "releaser",
        "READ_REPOSITORY,CREATE_REPOSITORY,UPDATE_REPOSITORY,DELETE_REPOSITORY",
    )
    .await;
    grant(&server, "admin", releaser).await;

    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/softwaremodules").await,
        200
    );
    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/webhooks").await,
        403,
        "hawier's own machinery is not part of the repository"
    );
    assert_eq!(
        call(&server, reqwest::Method::GET, "/rest/v1/retentionrules").await,
        403
    );
}

#[tokio::test]
#[ignore]
async fn role_all_still_means_everything() {
    let server = common::spawn().await;
    let admin = role(&server, "admin", permissions::ALL).await;
    grant(&server, "admin", admin).await;

    for path in [
        "/rest/v1/targets",
        "/rest/v1/rollouts",
        "/rest/v1/webhooks",
        "/rest/v1/retentionrules",
        "/rest/v1/distributionsets",
    ] {
        assert_eq!(
            call(&server, reqwest::Method::GET, path).await,
            200,
            "{path}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn userinfo_is_always_reachable_and_tells_the_truth() {
    let server = common::spawn().await;
    let reader = role(&server, "reader", "READ_TARGET").await;
    grant(&server, "admin", reader).await;

    // The console asks this to decide what to show; refusing it would leave
    // an operator staring at buttons that all answer 403.
    let response = common::http()
        .get(format!("{}/rest/v1/userinfo", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["permissions"], json!(["READ_TARGET"]));
}

#[tokio::test]
#[ignore]
async fn an_unclassified_endpoint_is_guarded_not_open() {
    // A path nobody thought to classify falls back to the repository
    // permissions, so adding an endpoint cannot accidentally add a hole.
    use axum::http::Method;
    assert_eq!(
        hawier_server::authz::permission_for(&Method::DELETE, "/rest/v1/somethingnew/1"),
        Some(permissions::DELETE_REPOSITORY)
    );
    assert_eq!(
        hawier_server::authz::permission_for(&Method::GET, "/rest/v1/somethingnew"),
        Some(permissions::READ_REPOSITORY)
    );
    // Only userinfo is exempt.
    assert_eq!(
        hawier_server::authz::permission_for(&Method::GET, "/rest/v1/userinfo"),
        None
    );
}

#[tokio::test]
#[ignore]
async fn the_ddi_path_is_untouched_by_mgmt_authorisation() {
    // Devices authenticate with their own tokens and must keep working while
    // an operator is locked out of the console.
    let server = common::spawn().await;
    role(&server, "reader", permissions::READ_TARGET).await;
    let target = common::seed_target(&server, "DEFAULT", "authz-dev").await;

    let status = common::http()
        .get(format!(
            "{}/DEFAULT/controller/v1/{}",
            server.base_url, target.controller_id
        ))
        .header("Authorization", format!("TargetToken {}", target.token))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(
        status, 200,
        "a locked-down tenant still updates its devices"
    );
}
