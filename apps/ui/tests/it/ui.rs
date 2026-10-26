//! UI console smoke matrix: settings editor, target card, DS card and
//! DS-type modules driven like a browser. Run via `make it`.

use crate::common::{browser, get_html, login, post_form, spawn_stack};

#[tokio::test]
#[ignore]
async fn ui_settings_page_edits_tenant_config() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let page = get_html(&stack, &http, &cookie, "/settings").await;
    assert!(page.contains("Tenant configuration"), "config editor shown");

    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/settings/config",
        &[("key", "pollingTime"), ("value", "00:10:00")],
    )
    .await;
    assert_eq!(response.status(), 303);
    let value: Option<String> = sqlx::query_scalar(
        "SELECT conf_value FROM sp_tenant_configuration
         WHERE tenant = 'DEFAULT' AND conf_key = 'pollingTime'",
    )
    .fetch_optional(&stack.server_state.pool)
    .await
    .unwrap();
    assert_eq!(value.as_deref(), Some("00:10:00"));

    // Reset returns the key to the server default.
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/settings/config/delete",
        &[("key", "pollingTime")],
    )
    .await;
    assert_eq!(response.status(), 303);
    let value: Option<String> = sqlx::query_scalar(
        "SELECT conf_value FROM sp_tenant_configuration
         WHERE tenant = 'DEFAULT' AND conf_key = 'pollingTime'",
    )
    .fetch_optional(&stack.server_state.pool)
    .await
    .unwrap();
    assert_eq!(value, None);
}

#[tokio::test]
#[ignore]
async fn ui_target_card_tags_type_and_auto_confirm() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // Create tag, type and target through the UI forms.
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/target-tags/new",
        &[("name", "ui-smoke-tag"), ("colour", "#00ff00")],
    )
    .await;
    assert_eq!(response.status(), 303, "tag create redirects");
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/target-types/new",
        &[("name", "ui-smoke-type"), ("key", "ui-smoke")],
    )
    .await;
    assert_eq!(response.status(), 303, "type create redirects");
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/targets/new",
        &[("controllerId", "ui-dev-1"), ("name", "ui-dev-1")],
    )
    .await;
    assert_eq!(response.status(), 303, "target create redirects");

    let tag_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_target_tag WHERE name = 'ui-smoke-tag'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    let type_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_target_type WHERE name = 'ui-smoke-type'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();

    // Tag chip appears after assignment.
    post_form(
        &stack,
        &http,
        &cookie,
        "/targets/ui-dev-1/tags",
        &[("tag_id", &tag_id.to_string())],
    )
    .await;
    let card = get_html(&stack, &http, &cookie, "/targets/ui-dev-1").await;
    assert!(card.contains("ui-smoke-tag"), "tag chip rendered");
    assert!(card.contains("tag-chip"), "chip markup present");

    // Target type select works.
    post_form(
        &stack,
        &http,
        &cookie,
        "/targets/ui-dev-1/type",
        &[("type_id", &type_id.to_string())],
    )
    .await;
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT target_type FROM sp_target WHERE controller_id = 'ui-dev-1'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    assert_eq!(assigned, Some(type_id));

    // Auto-confirm toggle round-trips.
    post_form(
        &stack,
        &http,
        &cookie,
        "/targets/ui-dev-1/auto-confirm/activate",
        &[],
    )
    .await;
    let card = get_html(&stack, &http, &cookie, "/targets/ui-dev-1").await;
    assert!(
        card.contains("auto-confirm/deactivate"),
        "active state shown"
    );
}

#[tokio::test]
#[ignore]
async fn ui_ds_card_shows_tags_and_action_page_cancels() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // DS tag chips (uses the additive GET /distributionsets/{id}/tags).
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/distribution-set-tags/new",
        &[("name", "ds-smoke-tag")],
    )
    .await;
    assert_eq!(response.status(), 303);
    let response = post_form(
        &stack,
        &http,
        &cookie,
        "/distribution-sets/new",
        &[("name", "ui-smoke-ds"), ("version", "1.0"), ("type", "os")],
    )
    .await;
    assert_eq!(response.status(), 303, "DS create redirects");
    let ds_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_distribution_set WHERE name = 'ui-smoke-ds'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    let tag_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_distribution_set_tag WHERE name = 'ds-smoke-tag'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    post_form(
        &stack,
        &http,
        &cookie,
        &format!("/distribution-sets/{ds_id}/tags"),
        &[("tag_id", &tag_id.to_string())],
    )
    .await;
    let card = get_html(
        &stack,
        &http,
        &cookie,
        &format!("/distribution-sets/{ds_id}"),
    )
    .await;
    assert!(card.contains("ds-smoke-tag"), "DS tag chip rendered");

    // Assign the DS to a target via mgmt, then cancel the action from the UI.
    post_form(
        &stack,
        &http,
        &cookie,
        "/targets/new",
        &[("controllerId", "ui-dev-2"), ("name", "ui-dev-2")],
    )
    .await;
    let target_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_target WHERE controller_id = 'ui-dev-2'")
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    let outcome = hawier_server::deploy_logic::assign(
        &stack.server_state.pool,
        &stack.server_state.incidents,
        "DEFAULT",
        &hawier_server::deploy_logic::AssignRequest {
            target_id,
            ds_id,
            action_type: hawier_api::codes::ActionType::Forced,
            forced_time: None,
            weight: hawier_server::deploy_logic::DEFAULT_WEIGHT,
            initiated_by: "it-ui".into(),
            confirmation_required: None,
            maintenance_window: None,
        },
    )
    .await
    .unwrap();

    // Action page renders status history.
    let page = get_html(
        &stack,
        &http,
        &cookie,
        &format!("/targets/ui-dev-2/actions/{}", outcome.action_id),
    )
    .await;
    assert!(page.contains("Status history"), "action page rendered");
    assert!(page.contains("running"), "initial RUNNING status listed");

    // Cancel from the UI: action flips to CANCELING (soft handshake).
    let response = post_form(
        &stack,
        &http,
        &cookie,
        &format!("/targets/ui-dev-2/actions/{}/cancel", outcome.action_id),
        &[],
    )
    .await;
    assert_eq!(response.status(), 303);
    let status: i32 = sqlx::query_scalar("SELECT status FROM sp_action WHERE id = $1")
        .bind(outcome.action_id)
        .fetch_one(&stack.server_state.pool)
        .await
        .unwrap();
    assert_eq!(status, hawier_api::codes::ActionStatus::Canceling.code());

    // Force-quit closes it entirely.
    let response = post_form(
        &stack,
        &http,
        &cookie,
        &format!("/targets/ui-dev-2/actions/{}/cancel", outcome.action_id),
        &[("force", "true")],
    )
    .await;
    assert_eq!(response.status(), 303);
    let (active, status): (Option<bool>, i32) =
        sqlx::query_as("SELECT active, status FROM sp_action WHERE id = $1")
            .bind(outcome.action_id)
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    assert_eq!(active, Some(false));
    assert_eq!(status, hawier_api::codes::ActionStatus::Canceled.code());
}

#[tokio::test]
#[ignore]
async fn ui_ds_type_module_management() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // Standard tenant bootstrap ships DS type "os" and SM type "os"/"app".
    let ds_type_id: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set_type
         WHERE UPPER(tenant) = 'DEFAULT' AND type_key = 'os'",
    )
    .fetch_one(&stack.server_state.pool)
    .await
    .unwrap();
    let app_type_id: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_software_module_type
         WHERE UPPER(tenant) = 'DEFAULT' AND type_key = 'application'",
    )
    .fetch_one(&stack.server_state.pool)
    .await
    .unwrap();

    // Add "application" as optional module type through the UI.
    let response = post_form(
        &stack,
        &http,
        &cookie,
        &format!("/distribution-set-types/{ds_type_id}/modules"),
        &[
            ("sm_type_id", app_type_id.to_string().as_str()),
            ("kind", "optional"),
        ],
    )
    .await;
    assert_eq!(response.status(), 303);
    let linked: Option<bool> = sqlx::query_scalar(
        "SELECT mandatory FROM sp_ds_type_element
         WHERE distribution_set_type = $1 AND software_module_type = $2",
    )
    .bind(ds_type_id)
    .bind(app_type_id)
    .fetch_optional(&stack.server_state.pool)
    .await
    .unwrap();
    assert_eq!(linked, Some(false), "linked as optional");

    let page = get_html(
        &stack,
        &http,
        &cookie,
        &format!("/distribution-set-types/{ds_type_id}"),
    )
    .await;
    assert!(page.contains("Optional module types"));

    // And remove it again.
    let response = post_form(
        &stack,
        &http,
        &cookie,
        &format!("/distribution-set-types/{ds_type_id}/modules/optional/{app_type_id}/remove"),
        &[],
    )
    .await;
    assert_eq!(response.status(), 303);
    let linked: Option<bool> = sqlx::query_scalar(
        "SELECT mandatory FROM sp_ds_type_element
         WHERE distribution_set_type = $1 AND software_module_type = $2",
    )
    .bind(ds_type_id)
    .bind(app_type_id)
    .fetch_optional(&stack.server_state.pool)
    .await
    .unwrap();
    assert_eq!(linked, None, "unlinked");
}
