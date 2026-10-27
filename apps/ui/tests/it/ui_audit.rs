//! Automated console audit — the guard that makes UI/UX defects fail the
//! build instead of waiting to be found by hand (see CLAUDE.md, UI/UX
//! rules). It walks every resource, every system view and every static
//! page, and checks both data correctness and structural invariants.
//!
//! Run everything: `make it`. Just this file: `make ui-audit`.

use crate::common::{browser, get_html, login, login_as, spawn_stack, spawn_stack_with, Stack};
use hawier_ui::{resources, view};
use serde_json::Value;

/// Distinctive enough that a substring search must find it and nothing else.
const NEEDLE: &str = "auditdev7391";

async fn seed(stack: &Stack) {
    let pool = &stack.server_state.pool;
    let os_sm_type: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_software_module_type WHERE UPPER(tenant) = 'DEFAULT' AND type_key = 'os'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let os_ds_type: i64 = sqlx::query_scalar(
        "SELECT id FROM sp_distribution_set_type WHERE UPPER(tenant) = 'DEFAULT' AND type_key = 'os'",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    // Targets in several update states so the dashboard cards are
    // non-trivial (codes from hawier_api::codes::TargetUpdateStatus).
    for (n, status) in [(1, 1), (2, 2), (3, 0)] {
        sqlx::query(
            "INSERT INTO sp_target
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, controller_id, sec_token, request_controller_attributes, update_status)
             VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, $1, $2, FALSE, $3)",
        )
        .bind(format!("{NEEDLE}-{n}"))
        .bind(format!("token-{NEEDLE}-{n}"))
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }

    let sm_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_software_module
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, version, sm_type, deleted, locked)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, '1.0.0', $2, FALSE, FALSE)
         RETURNING id",
    )
    .bind(format!("{NEEDLE}-module"))
    .bind(os_sm_type)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sp_artifact
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, md5_hash, file_size, provided_file_name, sha1_hash, software_module)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', 'md5', 4, 'audit.bin', 'sha1', $1)",
    )
    .bind(sm_id)
    .execute(pool)
    .await
    .unwrap();

    let ds_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_distribution_set
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, version, deleted, required_migration_step, ds_type, valid, locked)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, '1.0.0', FALSE, FALSE, $2, TRUE, FALSE)
         RETURNING id",
    )
    .bind(format!("{NEEDLE}-ds"))
    .bind(os_ds_type)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO sp_ds_sm (ds_id, sm_id) VALUES ($1, $2)")
        .bind(ds_id)
        .bind(sm_id)
        .execute(pool)
        .await
        .unwrap();

    // A tag (the by-tag view sorts on it) and a target filter.
    let tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_target_tag
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, description, colour)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, 'audit', '#00ff00')
         RETURNING id",
    )
    .bind(format!("{NEEDLE}-tag"))
    .fetch_one(pool)
    .await
    .unwrap();
    let target_id: i64 = sqlx::query_scalar("SELECT id FROM sp_target WHERE controller_id = $1")
        .bind(format!("{NEEDLE}-1"))
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sp_target_target_tag (target, tag) VALUES ($1, $2)")
        .bind(target_id)
        .bind(tag_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sp_target_filter_query
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, query, auto_assign_weight)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, 'name==*', 1000)",
    )
    .bind(format!("{NEEDLE}-filter"))
    .execute(pool)
    .await
    .unwrap();

    // An action with history. The action page is its own template, and it was
    // the one page the crawler never opened: nothing here was structurally
    // checked. The second status row carries a repeat count (optlock_revision
    // above 1), so the "xN last <time>" marker is crawled too.
    let action_id: i64 = sqlx::query_scalar(
        "INSERT INTO sp_action
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, active, action_type, status, distribution_set, target, weight,
          initiated_by)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', TRUE, 0, 6, $1, $2, 500, 'audit')
         RETURNING id",
    )
    .bind(ds_id)
    .bind(target_id)
    .fetch_one(pool)
    .await
    .unwrap();
    for (status, revision, message) in [(6, 1_i64, "action created"), (2, 12, "downloading")] {
        let status_id: i64 = sqlx::query_scalar(
            "INSERT INTO sp_action_status
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, target_occurred_at, status, action)
             VALUES (1, 'audit', 2, 'audit', $1, 'DEFAULT', 1, $2, $3)
             RETURNING id",
        )
        .bind(revision)
        .bind(status)
        .bind(action_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sp_action_status_messages (action_status, detail_message)
             VALUES ($1, $2)",
        )
        .bind(status_id)
        .bind(message)
        .execute(pool)
        .await
        .unwrap();
    }

    // A webhook and one delivery, so the column check below actually resolves
    // their columns instead of skipping both resources for want of a row.
    let webhook_id = stack.server_state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_webhook (id, tenant, name, url, secret, event_mask, active)
         VALUES ($1, 'DEFAULT', 'audit-hook', 'https://example.invalid/hook', 's3cret',
                 'rollout.started', TRUE)",
    )
    .bind(webhook_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO hawier_webhook_delivery
         (id, tenant, webhook, event, kind, status, error, duration_ms)
         VALUES ($1, 'DEFAULT', $2, $3, 'rollout.started', 200, '', 12)",
    )
    .bind(stack.server_state.incidents.next())
    .bind(webhook_id)
    .bind(stack.server_state.incidents.next())
    .execute(pool)
    .await
    .unwrap();

    // A role and a grant, so the column checks resolve rather than skip.
    // Deliberately after everything else: defining a role switches
    // enforcement on for the tenant, and `admin` must keep working.
    let role_id = stack.server_state.incidents.next();
    sqlx::query(
        "INSERT INTO hawier_role (id, tenant, name, description, permissions)
         VALUES ($1, 'DEFAULT', 'audit-admin', 'everything', 'ROLE_ALL')",
    )
    .bind(role_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO hawier_grant (id, tenant, username, role)
         VALUES ($1, 'DEFAULT', 'admin', $2)",
    )
    .bind(stack.server_state.incidents.next())
    .bind(role_id)
    .execute(pool)
    .await
    .unwrap();
    stack.server_state.authz_cache.invalidate();

    sqlx::query(
        "INSERT INTO hawier_retention_rule
         (id, tenant, name, scope, filter_rsql, min_age_ms, cron, dry_run, active, last_result)
         VALUES ($1, 'DEFAULT', 'audit-rule', 'artifact', 'module.version==*-dev',
                 604800000, '0 0 3 * * *', TRUE, TRUE, 'dry run: would delete 0')",
    )
    .bind(stack.server_state.incidents.next())
    .execute(pool)
    .await
    .unwrap();
}

/// A page is broken if it shows an error callout or leaks an incident number.
fn assert_no_error(page: &str, where_: &str) {
    assert!(
        !page.contains("callout danger"),
        "{where_} renders an error callout:\n{}",
        page.split("callout danger")
            .nth(1)
            .unwrap_or("")
            .chars()
            .take(300)
            .collect::<String>()
    );
    assert!(
        !page.contains("incident="),
        "{where_} leaked an incident number — the request failed"
    );
    // A template that fails to render used to answer 200 with this line, so
    // every page check below would have called it healthy.
    assert!(
        !page.contains("template render failed"),
        "{where_} could not render its template"
    );
}

#[tokio::test]
#[ignore]
async fn every_resource_view_renders_without_errors() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for resource in resources::all() {
        let list = get_html(&stack, &http, &cookie, &format!("/{}", resource.id)).await;
        assert_no_error(&list, &format!("/{}", resource.id));

        for view in resources::system_views(&resource.id) {
            let path = format!("/{}?view={}", resource.id, view.id);
            let page = get_html(&stack, &http, &cookie, &path).await;
            // Every view must be backed by fields the server actually
            // supports — a 400 here means the view is a broken feature.
            assert_no_error(&page, &path);
            assert!(page.contains("resource-toolbar"), "{path} lost its toolbar");
        }
    }
}

#[tokio::test]
#[ignore]
async fn static_pages_render_without_errors() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for path in [
        "/",
        "/settings",
        "/action-log",
        "/incidents",
        "/incidents?view=all",
        "/incidents?view=ddi",
        "/incidents?view=console",
        "/search?q=audit",
        "/search/live?q=",
        // Typeahead fragments behind every picker.
        "/pick/distribution-sets?q=",
        "/pick/target-tags?q=a",
        "/pick/software-modules?q=",
        // The manual, its search, and the What's-This fragment — including a
        // key nobody has written, which must answer plainly rather than fail.
        "/help",
        "/help?q=rollout",
        "/help/glossary",
        "/help/panel?key=glossary%23finished",
        "/help/panel?key=deployment%2Frollouts",
        "/help/panel?key=nothing%23written",
    ] {
        let page = get_html(&stack, &http, &cookie, path).await;
        assert_no_error(&page, path);
    }
}

/// Create forms are pages too: they render pickers and static selects, and
/// they broke silently once when a select source changed shape.
#[tokio::test]
#[ignore]
async fn create_forms_render() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for resource in resources::all().into_iter().filter(|r| r.create_verified) {
        let path = format!("/{}/new", resource.id);
        let page = get_html(&stack, &http, &cookie, &path).await;
        assert_no_error(&page, &path);
        assert!(page.contains("form-panel"), "{path} rendered no form");
    }
}

#[tokio::test]
#[ignore]
async fn dashboard_counters_match_the_lists_they_link_to() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let overview = get_html(&stack, &http, &cookie, "/").await;
    assert_no_error(&overview, "/");

    // Seeded: 1 in_sync (3), 1 pending (2), 1 unknown (0).
    for (status, expected) in [("in_sync", 1u64), ("pending", 1), ("unknown", 1)] {
        let total: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM sp_target WHERE update_status = {}",
            match status {
                "in_sync" => 1,
                "pending" => 2,
                _ => 0,
            }
        ))
        .fetch_one(&stack.server_state.pool)
        .await
        .unwrap();
        assert_eq!(total as u64, expected, "fixture drift for {status}");

        // The card shows the number and links to the query it counted.
        let card = overview
            .split("stat-card")
            .find(|chunk| chunk.contains(&format!(">{status}<")))
            .unwrap_or_else(|| panic!("no card for {status} in overview"));
        assert!(
            card.contains(&format!("<span class=\"stat-value\">{expected}</span>")),
            "card {status} does not show {expected}: {card}"
        );
        assert!(
            card.contains("updatestatus"),
            "card {status} must link to its own query"
        );
    }
    assert!(
        !overview.contains("<span class=\"stat-value\">—</span>"),
        "a counter failed to load"
    );
}

/// What an extension declared is really in the tree and really opens.
///
/// Every other check here crawls whatever the registry holds, so all of them
/// would pass just as happily if the declaration had been dropped on the
/// floor — this is the one that notices an empty merge. It used to look for
/// a declared *resource*, recognisable because a built-in never carries an
/// icon. Nothing declares a surface now, so it looks for declared views, and
/// `SystemView.declared` exists to make that possible: a view had no mark of
/// its own, which is how a declared view collided with a built-in id and was
/// discarded in silence for as long as it shipped.
#[tokio::test]
#[ignore]
async fn an_extension_declaration_reaches_the_tree() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // A surface, if one is ever declared again: recognisable because a
    // built-in resource never carries an icon.
    let surfaces: Vec<_> = resources::all()
        .into_iter()
        .filter(|resource| resource.icon.is_some())
        .collect();
    let declared_views: Vec<(String, resources::SystemView)> = resources::all()
        .into_iter()
        .flat_map(|resource| {
            resources::system_views(&resource.id)
                .into_iter()
                .filter(|view| view.declared)
                .map(move |view| (resource.id.clone(), view))
        })
        .collect();

    assert!(
        !surfaces.is_empty() || !declared_views.is_empty(),
        "nothing was adopted — the console is running on its built-in tree \
         alone, and every other check would still pass"
    );

    for resource in surfaces {
        let page = get_html(&stack, &http, &cookie, &format!("/{}", resource.id)).await;
        assert_no_error(&page, &format!("declared surface /{}", resource.id));
        assert!(
            page.contains(&resource.label),
            "the declared label is what the tree shows: {}",
            resource.label
        );
    }

    for (resource_id, view) in declared_views {
        let path = format!("/{resource_id}?view={}", view.id);
        let page = get_html(&stack, &http, &cookie, &path).await;
        // A declared view whose `q` names a field the resource's server-side
        // map does not know answers 400 and replaces the list with a red
        // callout — the module's mistake, but the operator's broken page.
        assert_no_error(&page, &format!("declared view {path}"));
        assert!(
            page.contains(&view.label),
            "{path}: the declared label is what the tree shows: {}",
            view.label
        );
    }
}

/// The two liveness views agree with the column they filter on.
///
/// This is UI/UX rule 6 at its sharpest: the same fact is a filter, a sort,
/// a badge in every row and a flag on the card, and the only way it stays one
/// fact is that one SQL expression produces all four. A row saying
/// "responding" inside the Not Responding list is the failure this exists to
/// catch, and nothing else here could see it — the crawl only checks that a
/// page renders, not that it renders the truth.
#[tokio::test]
#[ignore]
async fn liveness_views_agree_with_the_column_they_filter_on() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // One of each state. `seed` leaves its targets never-polled, so this only
    // has to add the two that have spoken.
    //
    // "now" comes from the database, not from a literal: the predicate is
    // evaluated by Postgres against its own clock, and a hardcoded epoch is a
    // fixture that silently rots into the past.
    let now: i64 = sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM now()) * 1000)::BIGINT")
        .fetch_one(&stack.server_state.pool)
        .await
        .unwrap();
    for (suffix, last) in [("live", now - 1_000), ("silent", now - 3_600_000)] {
        sqlx::query(
            "INSERT INTO sp_target
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, controller_id, sec_token, request_controller_attributes,
              update_status, last_target_query)
             VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, $1, $2, FALSE, 1, $3)",
        )
        .bind(format!("{NEEDLE}-{suffix}"))
        .bind(format!("token-{NEEDLE}-{suffix}"))
        .bind(last)
        .execute(&stack.server_state.pool)
        .await
        .unwrap();
    }
    let responding = get_html(&stack, &http, &cookie, "/targets?view=responding").await;
    assert_no_error(&responding, "/targets?view=responding");
    let not_responding = get_html(&stack, &http, &cookie, "/targets?view=not-responding").await;
    assert_no_error(&not_responding, "/targets?view=not-responding");

    assert!(
        !not_responding.contains(">responding<"),
        "a device the filter calls not-responding rendered as responding"
    );
    assert!(
        not_responding.contains(">overdue<") && not_responding.contains("never polled"),
        "both halves of not-responding must be reachable: overdue and never polled"
    );
    assert!(
        !responding.contains(">overdue<") && !responding.contains("never polled"),
        "the responding view must contain only devices that answered in time"
    );

    // Rule 6 again, as arithmetic: the two views are complements, so their
    // totals must add up to the unfiltered one. A page whose counter
    // disagrees with its own query is the defect the rule names.
    let total = |page: &str| -> i64 {
        page.split("of ")
            .nth(1)
            .and_then(|rest| {
                rest.split('<')
                    .next()
                    .and_then(|n| n.trim().parse::<i64>().ok())
            })
            .unwrap_or_else(|| panic!("no pager total on the page"))
    };
    let all = get_html(&stack, &http, &cookie, "/targets?view=all").await;
    assert_eq!(
        total(&responding) + total(&not_responding),
        total(&all),
        "the two liveness views must partition the fleet exactly"
    );

    // Rule 2: the frame survives whatever the data does.
    for page in [&responding, &not_responding] {
        assert!(
            page.contains("resource-toolbar"),
            "the toolbar left the page"
        );
    }
}

/// Every extension point has a row, and an absent one says what it costs.
///
/// A slot that simply vanished when unconfigured would leave an operator
/// unable to tell "this node cannot do that" from "I did not know it could".
#[tokio::test]
#[ignore]
async fn settings_lists_every_extension_point() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(&stack, &http, &cookie, "/settings").await;
    assert_no_error(&page, "/settings");

    for slot in hawier_server::ext::SLOTS {
        assert!(
            page.contains(slot.label),
            "no row for the {} slot",
            slot.key
        );
        assert!(
            page.contains(slot.env_var),
            "{}: the page must name the variable that points it at a module",
            slot.key
        );
    }
    // The harness configures every slot, so nothing should read as absent —
    // but the consequence text must be reachable, which is what makes an
    // unconfigured node legible.
    assert!(
        page.contains("node enforces"),
        "the deadline the node applies must sit beside the one modules declare"
    );
    // Rule 22's assertion counts exactly two of these; module versions use
    // their own class for that reason.
    assert!(
        page.matches("build-version").count() == 2,
        "module versions must not borrow the build stamp's class"
    );
}

/// A credential set through the API never comes back to the console.
///
/// Deliberately stricter than `secrets_are_not_printed_in_the_clear`: that
/// one exempts `data-token`, because a device's security token has to be
/// readable to be given to a device. An S3 key never needs reading back — it
/// needs replacing — so here there is no exemption at all, and the absence of
/// a reveal control is asserted so nobody later "fixes" this by reaching for
/// one.
#[tokio::test]
#[ignore]
async fn extension_parameter_secrets_never_reach_the_console() {
    let secret = format!("{NEEDLE}-s3-secret-value");
    let path = std::env::temp_dir().join(format!("hawier-audit-{}.env", std::process::id()));
    std::fs::write(&path, "").unwrap();
    let stack = spawn_stack_with(|config| {
        config.ext_params_file = path.display().to_string();
    })
    .await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let response = http
        .put(format!(
            "{}/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_SECRET_KEY",
            stack.server_url
        ))
        .basic_auth("admin", Some("admin"))
        .json(&serde_json::json!({ "value": secret }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);

    // It really was stored — otherwise everything below passes vacuously.
    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(
        stored.contains(&secret),
        "the write did not happen: {stored}"
    );

    let page = get_html(&stack, &http, &cookie, "/settings").await;
    assert_no_error(&page, "/settings");
    assert!(
        !page.contains(&secret),
        "/settings printed an infrastructure credential"
    );
    assert!(
        page.contains("HAWIER_EXT_S3_SECRET_KEY"),
        "but the operator must see that it is configured"
    );

    // The reveal convention must not be borrowed here.
    let panel = page
        .split("Extension parameters")
        .nth(1)
        .expect("the parameters panel");
    assert!(
        !panel.contains("data-token") && !panel.contains("data-copy"),
        "a credential must not be carried in an attribute for a reveal button"
    );

    let inventory = http
        .get(format!("{}/rest/v1/system/extensions", stack.server_url))
        .basic_auth("admin", Some("admin"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!inventory.contains(&secret), "the API handed it back");

    let log = http
        .get(format!("{}/rest/v1/actionlog?limit=100", stack.server_url))
        .basic_auth("admin", Some("admin"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !log.contains(&secret),
        "the action log is a readable resource; a credential must not land in it"
    );
    std::fs::remove_file(&path).ok();
}

/// The page survives a node that will not answer for its extensions.
#[tokio::test]
#[ignore]
async fn extension_panels_degrade_when_the_node_refuses() {
    let stack = spawn_stack().await;
    let http = browser();
    // Logged in as `limited`, not admin: the authorisation cache is
    // process-wide and keyed by (tenant, user, permission), so restricting
    // admin here would decide what every other audit running beside this one
    // sees.
    let cookie = login_as(&stack, &http, "limited").await;

    let admin_role: i64 = stack.server_state.incidents.next();
    let limited_role: i64 = stack.server_state.incidents.next();
    for (id, name, permissions) in [
        (admin_role, "administrator", "ROLE_ALL"),
        (
            limited_role,
            "no-system",
            "READ_TARGET,READ_REPOSITORY,READ_ROLLOUT",
        ),
    ] {
        sqlx::query(
            "INSERT INTO hawier_role (id, tenant, name, permissions) VALUES ($1, 'DEFAULT', $2, $3)",
        )
        .bind(id)
        .bind(name)
        .bind(permissions)
        .execute(&stack.server_state.pool)
        .await
        .unwrap();
    }
    for (username, role) in [("admin", admin_role), ("limited", limited_role)] {
        sqlx::query(
            "INSERT INTO hawier_grant (id, tenant, username, role) VALUES ($1, 'DEFAULT', $2, $3)",
        )
        .bind(stack.server_state.incidents.next())
        .bind(username)
        .bind(role)
        .execute(&stack.server_state.pool)
        .await
        .unwrap();
    }
    stack.server_state.authz_cache.invalidate();

    let page = get_html(&stack, &http, &cookie, "/settings").await;
    // Deliberately not `assert_no_error`: a refusal *should* render a danger
    // callout carrying the reason, and the console appends the incident
    // number to it on purpose so it can be found on /incidents. What must
    // hold is rule 2 — the message lives in the data area and the frame
    // around it is untouched.
    assert!(
        !page.contains("template render failed"),
        "/settings could not render"
    );
    assert!(page.contains("System Settings"), "the page frame is gone");
    assert!(page.contains("build-version"), "the Build panel is gone");
    assert!(
        page.contains("Tenant configuration"),
        "one panel failing took another with it"
    );
    assert!(
        page.contains("READ_SYSTEM"),
        "the panel must say why it is empty, not just be empty"
    );
    // And the panels are still there, with their titles, so the page does
    // not change shape depending on what the node answered.
    assert!(page.contains("Extension parameters"), "a panel disappeared");
}

/// Rule 25: every column the console renders has a clickable header, so
/// every column must be sortable. A field missing from the resource's
/// server-side map answers 400 and replaces the list with a red callout.
///
/// Driven by the registry rather than a hand-kept list, because the point is
/// that no resource can be added without this holding.
#[tokio::test]
#[ignore]
async fn every_declared_column_is_sortable() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();

    for resource in resources::all() {
        for column in resource.columns {
            let status = http
                .get(format!(
                    "{}/rest/v1/{}?limit=1&sort={column}:ASC",
                    stack.server_url, resource.endpoint
                ))
                .basic_auth("admin", Some("admin"))
                .send()
                .await
                .unwrap()
                .status();
            assert_eq!(
                status.as_u16(),
                200,
                "{}: sorting by '{column}' answered {status} — the console \
                 renders that column with a clickable header, so this is a \
                 red callout instead of a list",
                resource.id
            );
        }
    }
}

/// A column whose key the API never sends renders as "empty" in every row
/// — that is a broken column, not sparse data. Checked against the payload
/// rather than the rendered word, so it stays true however a null is drawn.
#[tokio::test]
#[ignore]
async fn declared_columns_exist_in_the_payload() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for resource in resources::all() {
        let listing: Value = serde_json::from_str(
            &http
                .get(format!(
                    "{}/rest/v1/{}?limit=1",
                    stack.server_url, resource.endpoint
                ))
                .basic_auth("admin", Some("admin"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
        )
        .unwrap_or_default();
        let Some(row) = listing["content"].as_array().and_then(|rows| rows.first()) else {
            continue; // nothing seeded for this resource
        };
        for column in resource.columns {
            let resolved = column
                .split('.')
                .try_fold(row, |value, part| value.get(part))
                .is_some();
            assert!(
                resolved,
                "{}: column '{column}' is not a key of the payload — it would render empty in \
                 every row (row keys: {:?})",
                resource.id,
                row.as_object().map(|o| o.keys().collect::<Vec<_>>())
            );
        }
    }
    let _ = cookie;
}

#[tokio::test]
#[ignore]
async fn global_search_finds_seeded_rows_on_the_server() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    // Bury the needle behind enough rows that first-page filtering fails.
    for n in 0..120 {
        sqlx::query(
            "INSERT INTO sp_target
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, controller_id, sec_token, request_controller_attributes, update_status)
             VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, $1, $2, FALSE, 0)",
        )
        .bind(format!("filler-{n:04}"))
        .bind(format!("filler-token-{n:04}"))
        .execute(&stack.server_state.pool)
        .await
        .unwrap();
    }
    let http = browser();
    let cookie = login(&stack, &http).await;

    let page = get_html(&stack, &http, &cookie, &format!("/search?q={NEEDLE}")).await;
    assert_no_error(&page, "/search");
    for expected in [
        format!("{NEEDLE}-1"),
        format!("{NEEDLE}-ds"),
        format!("{NEEDLE}-module"),
    ] {
        assert!(page.contains(&expected), "search did not find {expected}");
    }

    // A partial, mid-string needle must work too (substring, not prefix).
    let page = get_html(&stack, &http, &cookie, "/search?q=dev7391").await;
    assert!(
        page.contains(&format!("{NEEDLE}-1")),
        "substring search failed"
    );

    // Spaces and the RSQL keywords `and` / `or` in a needle are text.
    sqlx::query(
        "INSERT INTO sp_target
         (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
          tenant, name, controller_id, sec_token, request_controller_attributes, update_status)
         VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', 'north and south', 'north and south',
                 'keyword-token', FALSE, 0)",
    )
    .execute(&stack.server_state.pool)
    .await
    .unwrap();
    let page = get_html(&stack, &http, &cookie, "/search?q=h+and+s").await;
    assert_no_error(&page, "/search");
    assert!(
        page.contains("north and south"),
        "a needle with a keyword was not searched as text"
    );

    // Searching is remembered and offered back.
    let recents = get_html(&stack, &http, &cookie, "/search/live?q=").await;
    assert!(
        recents.contains(NEEDLE),
        "the search was not remembered: {recents}"
    );
}

#[tokio::test]
#[ignore]
async fn list_frame_survives_a_failing_query() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // A deliberately invalid filter: the data area shows the error, the
    // frame around it must stay usable.
    let page = get_html(&stack, &http, &cookie, "/targets?q=nosuchfield%3D%3D1").await;
    assert!(page.contains("callout danger"), "the error must be shown");
    for expected in ["resource-toolbar", "filter-panel", "pager", "New"] {
        assert!(
            page.contains(expected),
            "a failing list lost its {expected}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn structural_and_branding_invariants() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let list = get_html(&stack, &http, &cookie, "/targets").await;
    // Counter and paging live above the table, not below it.
    let pager = list.find("class=\"pager\"").expect("list has a pager");
    let table = list.find("<table").expect("list has a table");
    assert!(
        pager < table,
        "the pager must come before the table (it is at {pager}, table at {table})"
    );

    // Brand, and no stale product names anywhere (the login page is only
    // reachable logged out).
    // The manual is interface too, so it is held to the same names.
    for path in ["/", "/targets", "/settings", "/help", "/help/glossary"] {
        let page = get_html(&stack, &http, &cookie, path).await;
        assert!(
            !page.contains("hawkBit") && !page.contains("hawier-ui"),
            "{path} still shows a stale product name"
        );
    }
    let login = http
        .get(format!("{}/login", stack.ui_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !login.contains("hawkBit") && !login.contains("hawier-ui"),
        "the login page still shows a stale product name"
    );
    assert!(login.contains("class=\"brand"), "login lost the brand");
    assert!(list.contains("class=\"brand"), "the brand is missing");

    // The sidebar owns the session; the top bar owns only the search.
    let topbar = list
        .split("<header class=\"topbar\">")
        .nth(1)
        .and_then(|rest| rest.split("</header>").next())
        .expect("a topbar");
    assert!(
        !topbar.contains("logout") && !topbar.contains("Logout"),
        "session controls belong in the sidebar"
    );
    assert!(
        topbar.contains("global-search") && topbar.contains("brand"),
        "the top bar must hold the brand and the search"
    );
    assert!(
        list.contains("sidebar-footer"),
        "the sidebar must carry the session footer"
    );
}

/// A missing semicolon before a line starting with `(` once turned five
/// working features into one `TypeError` — ASI hazards are a class, not an
/// accident, so they fail the build now.
#[tokio::test]
#[ignore]
async fn client_script_cannot_break_silently() {
    let js = std::fs::read_to_string("../../static/assets/main.js")
        .expect("the console script next to the workspace root");

    let mut previous: Option<&str> = None;
    for (index, line) in js.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(first) = line.chars().next() {
            if matches!(first, '(' | '[' | '`') {
                let prev = previous.unwrap_or("");
                let ends_safely = prev
                    .chars()
                    .last()
                    .is_some_and(|c| matches!(c, ';' | '{' | '}' | ',' | '(' | '['));
                assert!(
                    ends_safely,
                    "line {}: automatic semicolon insertion hazard — '{}' continues into '{}'",
                    index + 1,
                    prev,
                    line
                );
            }
        }
        previous = Some(line);
    }

    let depth: i32 = js.chars().fold(0, |depth, c| match c {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    });
    assert_eq!(depth, 0, "unbalanced braces in main.js");

    // Every initialiser referenced by the boot list must exist, and the boot
    // list must isolate failures.
    assert!(
        js.contains("try {") && js.contains("catch (error)"),
        "initialisers must be wrapped so one failure cannot kill the page"
    );
    for name in js
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("init") && line.ends_with(','))
    {
        let name = name.trim_end_matches(',');
        assert!(
            js.contains(&format!("function {name}(")),
            "boot list references {name}, which is not declared"
        );
    }
}

/// Timestamps are never printed as raw numbers, and always carry the exact
/// UTC stamp for hovering.
#[tokio::test]
#[ignore]
async fn timestamps_are_human_everywhere() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // Give the console something to log, so the action log is non-empty.
    get_html(&stack, &http, &cookie, "/targets").await;

    let target_id: String =
        sqlx::query_scalar("SELECT controller_id FROM sp_target WHERE controller_id LIKE $1")
            .bind(format!("{NEEDLE}%"))
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    for path in [
        "/".to_string(),
        "/targets".to_string(),
        "/action-log".to_string(),
        "/incidents?view=all".to_string(),
        "/distribution-sets".to_string(),
        format!("/targets/{target_id}"),
    ] {
        let path = path.as_str();
        let page = get_html(&stack, &http, &cookie, path).await;
        assert_no_error(&page, path);

        // Text nodes must not contain an epoch: those read as gibberish.
        for chunk in page.split('>').skip(1) {
            let Some(text) = chunk.split('<').next() else {
                continue;
            };
            for word in text.split_whitespace() {
                let digits = word.trim_matches(|c: char| !c.is_ascii_digit());
                if digits.len() >= 10 && digits.len() <= 13 && digits.starts_with('1') {
                    if let Ok(value) = digits.parse::<i64>() {
                        // 2001-09-09 .. 2286-11-20 in seconds, or millis.
                        let looks_like_time =
                            (1_000_000_000..=9_999_999_999).contains(&value) || digits.len() == 13;
                        assert!(
                            !looks_like_time,
                            "{path} shows a raw epoch ({digits}) instead of a formatted time"
                        );
                    }
                }
            }
        }

        assert!(
            !page.contains("<td>object</td>"),
            "{path} renders a nested value as the literal word 'object'"
        );

        for tag in page.split("<time").skip(1) {
            let head = tag.split('>').next().unwrap_or("");
            assert!(
                head.contains("data-epoch=") && head.contains("title="),
                "{path}: <time{head}> must carry data-epoch and the UTC title"
            );
        }
    }
}

/// Trailing controls live in their own fixed column — they must not drift
/// with the length of the text beside them.
#[tokio::test]
#[ignore]
async fn trailing_controls_are_aligned() {
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    assert!(
        css.contains(".list-row {") && css.contains(".list-row-actions {"),
        "the shared row layout must exist"
    );

    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    // Remember a search so the dropdown has a row with a delete control.
    get_html(&stack, &http, &cookie, &format!("/search?q={NEEDLE}")).await;

    let ds_id: i64 = sqlx::query_scalar("SELECT id FROM sp_distribution_set WHERE name = $1")
        .bind(format!("{NEEDLE}-ds"))
        .fetch_one(&stack.server_state.pool)
        .await
        .unwrap();
    for path in [
        "/search/live?q=".to_string(),
        format!("/distribution-sets/{ds_id}"),
    ] {
        let page = get_html(&stack, &http, &cookie, &path).await;
        for (index, _) in page.match_indices("chip-remove") {
            let context = &page[index.saturating_sub(400)..index];
            assert!(
                context.contains("list-row-actions"),
                "{path}: a remove control outside the fixed action column"
            );
        }
    }
}

/// A table row is one line tall, whatever a cell holds.
///
/// A single long value — a role's permission list — wrapped, grew the row to
/// three lines, and pushed the last two columns off the right edge of a
/// five-row list. Nothing errored, so nothing caught it: the page was 200
/// and every column was in the markup, just not on the screen.
#[tokio::test]
#[ignore]
async fn a_long_value_does_not_push_columns_off_the_table() {
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    // Any rule reaching `td` will do — `th, td { ... }` also ends in "td {",
    // so the first match is the shared one and finding it would prove
    // nothing.
    let clips = css
        .split("td {")
        .skip(1)
        .filter_map(|block| block.split_once('}').map(|(block, _)| block))
        .any(|block| {
            ["max-width", "overflow: hidden", "text-overflow: ellipsis"]
                .iter()
                .all(|property| block.contains(property))
        });
    assert!(
        clips,
        "no rule makes a cell clip rather than wrap: a long value grows the \
         row and pushes the last columns off the table"
    );
    // Clipping without this is data you cannot reach at all.
    let js = std::fs::read_to_string("../../static/assets/main.js").expect("script");
    assert!(
        js.contains("initCellTitles") && js.contains("scrollWidth > cell.clientWidth"),
        "a clipped cell must keep its value in a tooltip"
    );

    // And the values that provoked it really are long, so this is not a test
    // of a case the console never produces.
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(&stack, &http, &cookie, "/roles").await;
    assert_no_error(&page, "/roles");
}

/// Colour lives in one place: tokens on `:root`. Literals scattered through
/// the file are how the palette drifted apart before.
#[tokio::test]
#[ignore]
async fn colours_come_from_tokens() {
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    let (root, rest) = css.split_once("\n}").expect(":root block");
    assert!(
        root.contains(":root"),
        "the file must open with :root tokens"
    );
    for (index, line) in rest.lines().enumerate() {
        let line = line.trim();
        if line.starts_with("/*") || line.starts_with("*") {
            continue;
        }
        assert!(
            !line.contains('#') || !line.chars().any(|c| c.is_ascii_hexdigit()) || !has_hex(line),
            "line {}: raw colour outside :root — use a token: {line}",
            index + 2
        );
    }
}

/// `#rrggbb` / `#rgb` outside a comment.
fn has_hex(line: &str) -> bool {
    let bytes: Vec<char> = line.chars().collect();
    bytes.iter().enumerate().any(|(i, c)| {
        *c == '#'
            && bytes
                .get(i + 1..i + 4)
                .is_some_and(|next| next.iter().all(|c| c.is_ascii_hexdigit()))
    })
}

/// In the collapsed rail the icon is the only thing left — two resources
/// sharing a mark are indistinguishable.
#[tokio::test]
#[ignore]
async fn every_resource_has_its_own_icon() {
    let markup = std::fs::read_to_string("src/view.rs").expect("view.rs");
    let mapping = markup
        .split("fn resource_icon(")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("resource_icon body");
    for resource in resources::all() {
        // A built-in is known by id; a declared surface brings its own name,
        // and the name still has to resolve to a real shape.
        let named = if resource.icon.is_some() {
            view::resource_icon(&resource) != "resource"
        } else {
            mapping.contains(&format!("\"{}\"", resource.id))
        };
        assert!(named, "resource '{}' has no icon of its own", resource.id);
    }
    // The fallback square must remain unused by real resources.
    assert!(
        mapping.matches("=> \"resource\"").count() == 1,
        "the generic square should only be the catch-all arm"
    );
}

/// Controls are icons or words, never lone glyphs, and the collapsed rail
/// has no empty clickable strips.
#[tokio::test]
#[ignore]
async fn no_glyph_controls_and_no_empty_rail_strips() {
    for (name, markup) in source_files() {
        for glyph in ['✎', '✂', '➜'] {
            assert!(
                !markup.contains(glyph),
                "{name}: control rendered as the bare glyph {glyph} — use an icon"
            );
        }
    }
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    let collapsed = css
        .split("body.sidebar-collapsed")
        .nth(1)
        .expect("collapsed rules");
    assert!(
        css.contains("body.sidebar-collapsed .nav-section > summary"),
        "group headers must be hidden in the rail, not left as empty strips"
    );
    let _ = collapsed;
}

/// Any list capped by a limit must say so with a pager — silently showing
/// the first N of many is how the deploy-group page lied about 300 devices.
#[tokio::test]
#[ignore]
async fn sub_lists_are_paged() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let rollout_id: Option<i64> = sqlx::query_scalar("SELECT id FROM sp_rollout LIMIT 1")
        .fetch_optional(&stack.server_state.pool)
        .await
        .unwrap();
    let Some(rollout_id) = rollout_id else {
        // No rollout in the fixture: the markup check below still applies.
        let markup =
            std::fs::read_to_string("templates/rollout_group.html").expect("group template");
        assert!(
            markup.contains("m::pager"),
            "sub-lists must use the shared pager"
        );
        return;
    };
    let group_id: i64 =
        sqlx::query_scalar("SELECT id FROM sp_rollout_group WHERE rollout = $1 LIMIT 1")
            .bind(rollout_id)
            .fetch_one(&stack.server_state.pool)
            .await
            .unwrap();
    let page = get_html(
        &stack,
        &http,
        &cookie,
        &format!("/rollouts/{rollout_id}/groups/{group_id}"),
    )
    .await;
    assert_no_error(&page, "rollout group");
    assert!(
        page.contains("class=\"pager\""),
        "the group list needs a pager"
    );
}

/// Selection must exist wherever a bulk action is offered.
#[tokio::test]
#[ignore]
async fn lists_support_bulk_selection() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for resource in resources::all() {
        let page = get_html(&stack, &http, &cookie, &format!("/{}", resource.id)).await;
        assert!(
            page.contains("data-select-all"),
            "{}: no select-all checkbox",
            resource.id
        );
        if resources::bulk_actions(&resource.id).is_empty() {
            continue;
        }
        assert!(
            page.contains("data-bulk-form") && page.contains(&format!("/{}/bulk", resource.id)),
            "{}: the bulk form has no action",
            resource.id
        );
        assert!(
            page.contains("data-context-menu") && page.contains("data-bulk-menu"),
            "{}: the selection has no operations menu",
            resource.id
        );
    }
}

/// A named reference must be clickable, not a dead end.
#[tokio::test]
#[ignore]
async fn named_references_are_links() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let pool = &stack.server_state.pool;
    let ds_id: i64 = sqlx::query_scalar("SELECT id FROM sp_distribution_set WHERE name = $1")
        .bind(format!("{NEEDLE}-ds"))
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sp_target SET assigned_distribution_set = $1 WHERE controller_id = $2")
        .bind(ds_id)
        .bind(format!("{NEEDLE}-1"))
        .execute(pool)
        .await
        .unwrap();

    let http = browser();
    let cookie = login(&stack, &http).await;
    let list = get_html(&stack, &http, &cookie, "/targets").await;
    assert!(
        list.contains(&format!("/distribution-sets/{ds_id}")),
        "the assigned set must link to its page"
    );
}

/// The action page is where an operator looks when a device misbehaves. The
/// server keeps one row per state and counts verbatim repeats on it, so a
/// device stuck in a retry loop is visible only through that counter — if it
/// stops being rendered, the page turns a loop into "reported once, went
/// quiet". The crawl covers the page; this covers what it has to say.
#[tokio::test]
#[ignore]
async fn action_page_shows_repeat_counts() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let pool = &stack.server_state.pool;
    let controller_id = format!("{NEEDLE}-1");
    let action_id: i64 = sqlx::query_scalar(
        "SELECT a.id FROM sp_action a JOIN sp_target t ON t.id = a.target
         WHERE t.controller_id = $1",
    )
    .bind(&controller_id)
    .fetch_one(pool)
    .await
    .unwrap();

    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(
        &stack,
        &http,
        &cookie,
        &format!("/targets/{controller_id}/actions/{action_id}"),
    )
    .await;
    assert_no_error(&page, "action page");
    // The fixture's second status row was reported 12 times (11 repeats).
    assert!(
        page.contains("repeat-count") && page.contains("×12"),
        "the repeat counter must be on the page:\n{page}"
    );
    assert!(
        !page.contains("×1<") && !page.contains("×1 "),
        "a status reported once must not be decorated with a count"
    );
}

#[tokio::test]
#[ignore]
async fn stylesheet_stays_consistent() {
    // The console is strictly rectangular; radii crept in once already.
    let css = std::fs::read_to_string("../../static/assets/main.css")
        .expect("stylesheet next to the workspace root");
    for (line_no, line) in css.lines().enumerate() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("border-radius:") {
            assert!(
                value.trim().trim_end_matches(';') == "0",
                "line {}: rounded corners are not the console's style: {line}",
                line_no + 1
            );
        }
        // Every custom property used must be defined in this file.
        if let Some(index) = line.find("var(--") {
            let name: String = line[index + 4..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-')
                .collect();
            assert!(
                css.contains(&format!("{name}:")),
                "line {}: undefined custom property {name}",
                line_no + 1
            );
        }
    }
}

#[tokio::test]
#[ignore]
async fn incidents_are_classified_and_filterable() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    // A console-caused failure (the UI asking mgmt for a bad filter)…
    get_html(&stack, &http, &cookie, "/targets?q=nosuchfield%3D%3D1").await;
    // …and a device-caused one.
    http.get(format!(
        "{}/DEFAULT/controller/v1/audit-unknown",
        stack.server_url
    ))
    .send()
    .await
    .ok();

    // Journaling is async. Waiting a fixed 300 ms passed alone and failed
    // under the full suite, where the machine is busy — poll for the row
    // instead of guessing how slow the box is today.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let kinds = loop {
        let kinds: Vec<String> = sqlx::query_scalar("SELECT DISTINCT kind FROM hawier_incident")
            .fetch_all(&stack.server_state.pool)
            .await
            .unwrap();
        if kinds.iter().any(|kind| kind == "console") || std::time::Instant::now() >= deadline {
            break kinds;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert!(
        kinds.iter().any(|kind| kind == "console"),
        "console-caused incidents must be labelled as such, got {kinds:?}"
    );

    // The dashboard hides console noise but accounts for it.
    let overview = get_html(&stack, &http, &cookie, "/").await;
    assert!(
        overview.contains("console-side incident"),
        "the dashboard must mention hidden console noise"
    );
    // Incidents are an ordinary mgmt resource now: the "important" view is
    // just an RSQL filter like any other.
    let listed: Value = serde_json::from_str(
        &http
            .get(format!("{}/rest/v1/incidents", stack.server_url))
            .query(&[("q", "kind=in=(ddi,mgmt),status=ge=500")])
            .basic_auth("admin", Some("admin"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    for incident in listed["content"].as_array().expect("paged envelope") {
        assert_ne!(
            incident["kind"], "console",
            "console incidents are not important"
        );
    }
}

// ---------------------------------------------------------------------------
// Fourth-wave invariants
// ---------------------------------------------------------------------------

/// Every `.rs` file of the console, for the source-level checks below.
fn source_files() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir)
            .expect("readable source tree")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push((
                    path.display().to_string(),
                    std::fs::read_to_string(&path).expect("readable source file"),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(std::path::Path::new("src"), &mut out);
    out
}

/// Pages worth crawling for the structural checks: every list, plus one
/// object page of each shape that has its own panels.
async fn every_page(stack: &Stack, http: &reqwest::Client, cookie: &str) -> Vec<(String, String)> {
    let pool = &stack.server_state.pool;
    let mut paths: Vec<String> = resources::all()
        .iter()
        .map(|resource| format!("/{}", resource.id))
        .collect();
    paths.push("/".to_string());
    paths.push("/settings".to_string());
    paths.push(format!("/search?q={NEEDLE}"));

    let target: Option<String> =
        sqlx::query_scalar("SELECT controller_id FROM sp_target WHERE controller_id LIKE $1")
            .bind(format!("{NEEDLE}%"))
            .fetch_optional(pool)
            .await
            .unwrap();
    if let Some(target) = target {
        paths.push(format!("/targets/{target}"));
        let action: Option<i64> = sqlx::query_scalar(
            "SELECT a.id FROM sp_action a JOIN sp_target t ON t.id = a.target
             WHERE t.controller_id = $1 LIMIT 1",
        )
        .bind(&target)
        .fetch_optional(pool)
        .await
        .unwrap();
        if let Some(action) = action {
            paths.push(format!("/targets/{target}/actions/{action}"));
        }
    }
    for (table, prefix) in [
        ("sp_distribution_set", "/distribution-sets"),
        ("sp_software_module", "/software-modules"),
        ("sp_target_tag", "/target-tags"),
        // Object pages carry most of the console's controls, so every card
        // shape has to be crawled — the panels on these two used to be
        // written with no structural check watching them at all.
        ("sp_target_filter_query", "/target-filters"),
    ] {
        let id: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT id FROM {table} WHERE name LIKE $1 LIMIT 1"
        ))
        .bind(format!("{NEEDLE}%"))
        .fetch_optional(pool)
        .await
        .unwrap();
        if let Some(id) = id {
            paths.push(format!("{prefix}/{id}"));
        }
    }
    for (table, prefix) in [
        ("sp_distribution_set_type", "/distribution-set-types"),
        ("sp_target_type", "/target-types"),
        ("sp_software_module_type", "/software-module-types"),
    ] {
        let id: Option<i64> = sqlx::query_scalar(&format!("SELECT id FROM {table} LIMIT 1"))
            .fetch_optional(pool)
            .await
            .unwrap();
        if let Some(id) = id {
            paths.push(format!("{prefix}/{id}"));
        }
    }

    let mut pages = Vec::new();
    for path in paths {
        let page = get_html(stack, http, cookie, &path).await;
        pages.push((path, page));
    }
    pages
}

/// Markup belongs in `apps/ui/templates`. Rust may still build small value
/// fragments (an icon), but never page structure — that is how html.rs grew
/// to two thousand lines nobody wanted to touch.
#[tokio::test]
#[ignore]
async fn markup_lives_only_in_templates() {
    for (name, source) in source_files() {
        for tag in [
            "<div", "<section", "<table", "<tr", "<form", "<button", "<input",
        ] {
            let hits = source.matches(tag).count();
            assert_eq!(
                hits, 0,
                "{name} builds page structure ('{tag}') in Rust — move it to a template"
            );
        }
    }
    assert!(
        std::path::Path::new("templates/base.html").exists(),
        "the page shell must be a template"
    );
}

/// UUIDs were dropped everywhere the operator can see: they cannot be
/// double-clicked, cannot be dictated, and mean nothing.
#[tokio::test]
#[ignore]
async fn no_uuids_reach_the_interface() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    // Produce a saved view and a log entry — the two that used to be UUIDs.
    http.post(format!("{}/views", stack.ui_url))
        .header("Cookie", &cookie)
        .form(&[
            ("resource", "targets"),
            ("name", "audit view"),
            ("q", ""),
            ("sort", ""),
            ("columns", ""),
            ("limit", "25"),
        ])
        .send()
        .await
        .unwrap();

    for (path, page) in every_page(&stack, &http, &cookie).await {
        if let Some(found) = find_uuid(&page) {
            panic!("{path} shows a UUID ({found}) — hawier ids are numbers");
        }
    }
}

/// `8-4-4-4-12` hex, the shape we banned.
fn find_uuid(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let groups = [8usize, 4, 4, 4, 12];
    'outer: for start in 0..chars.len() {
        let mut at = start;
        for (index, len) in groups.iter().enumerate() {
            if index > 0 {
                if chars.get(at) != Some(&'-') {
                    continue 'outer;
                }
                at += 1;
            }
            for offset in 0..*len {
                match chars.get(at + offset) {
                    Some(c) if c.is_ascii_hexdigit() => {}
                    _ => continue 'outer,
                }
            }
            at += len;
        }
        return Some(chars[start..at].iter().collect());
    }
    None
}

/// A dropdown filled from a server list breaks the moment the list grows.
/// Only short, static enumerations may stay a `<select>`; everything else
/// is a searchable picker.
#[tokio::test]
#[ignore]
async fn dropdowns_are_static_enumerations_only() {
    /// `<select name="...">` allowed to exist: fixed hawkBit enums.
    const STATIC_SELECTS: [&str; 2] = ["type", "kind"];

    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for (path, page) in every_page(&stack, &http, &cookie).await {
        for chunk in page.split("<select").skip(1) {
            let head = chunk.split('>').next().unwrap_or("");
            let name = head
                .split("name=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or("");
            assert!(
                STATIC_SELECTS.contains(&name),
                "{path}: <select name=\"{name}\"> is filled from the server — use a picker"
            );
            let options = chunk
                .split("</select>")
                .next()
                .unwrap_or("")
                .matches("<option")
                .count();
            assert!(
                options <= 8,
                "{path}: <select name=\"{name}\"> has {options} options — that is a list, not an enum"
            );
        }
    }
}

/// Which build is actually running has to be readable, per component.
#[tokio::test]
#[ignore]
async fn settings_shows_the_build_of_each_component() {
    let stack = spawn_stack().await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(&stack, &http, &cookie, "/settings").await;
    assert_no_error(&page, "/settings");

    let versions: Vec<&str> = page
        .split("class=\"build-version\">")
        .skip(1)
        .filter_map(|chunk| chunk.split('<').next())
        .collect();
    assert_eq!(
        versions.len(),
        2,
        "console and server builds must both show"
    );
    for version in versions {
        let rest = version
            .strip_prefix('v')
            .unwrap_or_else(|| panic!("build string '{version}' must look like vYY.MM.DD-<sha>"));
        let (date, sha) = rest
            .split_once('-')
            .unwrap_or_else(|| panic!("build string '{version}' has no commit part"));
        assert_eq!(
            date.split('.').count(),
            3,
            "build string '{version}' has no YY.MM.DD date"
        );
        assert!(!sha.is_empty(), "build string '{version}' has no commit");
    }
}

/// Deleting a hundred rows must ask first. The confirmation used to be
/// attached to the button while the handler read it off the form, so bulk
/// delete went through in silence.
#[tokio::test]
#[ignore]
async fn destructive_bulk_actions_confirm() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let js = std::fs::read_to_string("../../static/assets/main.js").expect("console script");
    assert!(
        js.contains("data-bulk-danger") || js.contains("bulkDanger"),
        "the script must read the confirmation off the clicked button"
    );
    assert!(
        js.contains("event.submitter"),
        "the confirmation must come from the submitter, not the form"
    );

    for resource in resources::all() {
        let dangerous: Vec<&str> = resources::bulk_actions(&resource.id)
            .iter()
            .filter(|action| action.danger)
            .map(|action| action.id)
            .collect();
        if dangerous.is_empty() {
            continue;
        }
        let page = get_html(&stack, &http, &cookie, &format!("/{}", resource.id)).await;
        for action in dangerous {
            let button = page
                .split("<button")
                .filter_map(|chunk| chunk.split('>').next())
                .find(|head| head.contains(&format!("value=\"{action}\"")))
                .unwrap_or_else(|| panic!("{}: no button for '{action}'", resource.id));
            assert!(
                button.contains("data-bulk-danger"),
                "{}: '{action}' can destroy rows without asking",
                resource.id
            );
        }
    }
}

/// A selection you cannot see is a selection you act on by accident, and a
/// click one pixel beside the box must not navigate away and lose it.
#[tokio::test]
#[ignore]
async fn selection_is_visible_and_forgiving() {
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    assert!(
        css.contains("tr.selected"),
        "a selected row needs a style of its own, not just a ticked box"
    );
    assert!(
        css.contains(".select-box"),
        "the selection cell must be a full-size label"
    );
    let js = std::fs::read_to_string("../../static/assets/main.js").expect("console script");
    assert!(
        js.contains(".select-cell"),
        "clicking beside the box must select, not navigate"
    );

    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(&stack, &http, &cookie, "/targets").await;
    for chunk in page.split("class=\"select-cell\"").skip(1) {
        let cell = chunk.split("</td>").next().unwrap_or(chunk);
        if !cell.contains("<input") {
            continue;
        }
        assert!(
            cell.contains("<label"),
            "a bare checkbox outside a label: {cell}"
        );
    }
}

/// Navigation lives in the sidebar and the page title; "Back" buttons only
/// duplicate the browser. The action log is one resource, not a panel
/// stapled to the bottom of every page.
#[tokio::test]
#[ignore]
async fn no_back_buttons_and_no_stray_log_panels() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for (path, page) in every_page(&stack, &http, &cookie).await {
        for phrase in [">Back<", ">Back to "] {
            assert!(
                !page.contains(phrase),
                "{path} still offers a Back button — navigation belongs to the sidebar"
            );
        }
        if path != "/action-log" {
            assert!(
                !page.contains("Recent actions") && !page.contains("logs-panel"),
                "{path} carries an activity panel — the journal is the Action Log resource"
            );
        }
    }
}

/// Any rule that sets `display` outbids the browser's own `[hidden]` rule,
/// so a "hidden" element stays laid out — an invisible menu over the table
/// swallowed clicks on the rows beneath it, and the selection counter read
/// "0 selected" with nothing selected. One global opt-out is the fix; this
/// asserts it is there and that nothing weakens it.
#[tokio::test]
#[ignore]
async fn hidden_elements_are_really_hidden() {
    let css = std::fs::read_to_string("../../static/assets/main.css").expect("stylesheet");
    let block = css
        .split("[hidden] {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("a global [hidden] rule — the attribute alone loses to any display: rule");
    assert!(
        block.contains("display: none !important"),
        "the global [hidden] rule must win against class rules that set display"
    );

    // And the layers must actually start hidden.
    let list = std::fs::read_to_string("templates/list.html").expect("list template");
    for layer in ["data-context-menu", "data-bulk-menu"] {
        let head = list
            .split(layer)
            .nth(1)
            .and_then(|rest| rest.split('>').next())
            .unwrap_or("");
        assert!(head.contains("hidden"), "{layer} must start hidden");
    }
}

/// A secret with a "Show" button beside it must not also be sitting in
/// plain text three rows above — the target page printed the security
/// token in its generic property table.
#[tokio::test]
#[ignore]
async fn secrets_are_not_printed_in_the_clear() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT controller_id, sec_token FROM sp_target WHERE controller_id LIKE $1",
    )
    .bind(format!("{NEEDLE}%"))
    .fetch_all(&stack.server_state.pool)
    .await
    .unwrap();
    assert!(
        !rows.is_empty(),
        "the fixture must seed a target with a token"
    );

    for (controller_id, token) in rows {
        let path = format!("/targets/{controller_id}");
        let page = get_html(&stack, &http, &cookie, &path).await;
        // The reveal control carries it in a data attribute on purpose; the
        // page text must not.
        let visible = page.replace(&format!("data-token=\"{token}\""), "");
        let visible = visible.replace(&format!("data-copy=\"{token}\""), "");
        assert!(
            !visible.contains(&token),
            "{path} prints the security token in the clear"
        );
    }
}

/// The six things the server could do that the console could not reach.
/// Each is a panel on a card; a missing one is a dead end, not a cosmetic gap.
#[tokio::test]
#[ignore]
async fn cards_reach_what_the_server_offers() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let pool = &stack.server_state.pool;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let one = |table: &'static str| async move {
        sqlx::query_scalar::<_, i64>(&format!("SELECT id FROM {table} LIMIT 1"))
            .fetch_one(pool)
            .await
            .unwrap()
    };

    // Metadata is editable wherever the server keeps it.
    let target: String =
        sqlx::query_scalar("SELECT controller_id FROM sp_target WHERE controller_id LIKE $1")
            .bind(format!("{NEEDLE}%"))
            .fetch_one(pool)
            .await
            .unwrap();
    let ds = one("sp_distribution_set").await;
    let sm = one("sp_software_module").await;
    for (path, action) in [
        (
            format!("/targets/{target}"),
            format!("/targets/{target}/metadata"),
        ),
        (
            format!("/distribution-sets/{ds}"),
            format!("/distribution-sets/{ds}/metadata"),
        ),
        (
            format!("/software-modules/{sm}"),
            format!("/software-modules/{sm}/metadata"),
        ),
    ] {
        let page = get_html(&stack, &http, &cookie, &path).await;
        assert_no_error(&page, &path);
        assert!(
            page.contains("Metadata") && page.contains(&action),
            "{path} cannot edit metadata, which the server has stored for it"
        );
    }

    // A device's group, a filter's auto-assignment, a type's compatible sets.
    let filter = one("sp_target_filter_query").await;
    let target_type: Option<i64> = sqlx::query_scalar("SELECT id FROM sp_target_type LIMIT 1")
        .fetch_optional(pool)
        .await
        .unwrap();
    let mut expected = vec![
        (
            format!("/targets/{target}"),
            format!("/targets/{target}/group"),
        ),
        (
            format!("/target-filters/{filter}"),
            format!("/target-filters/{filter}/auto-assign"),
        ),
    ];
    if let Some(type_id) = target_type {
        expected.push((
            format!("/target-types/{type_id}"),
            format!("/target-types/{type_id}/ds-types"),
        ));
    }
    for (path, action) in expected {
        let page = get_html(&stack, &http, &cookie, &path).await;
        assert_no_error(&page, &path);
        assert!(page.contains(&action), "{path} is missing {action}");
    }

    // An artifact is downloadable, and through the console — a link straight
    // at the mgmt API would ask the browser for credentials it has not got.
    let artifact: Option<i64> = sqlx::query_scalar("SELECT id FROM sp_artifact LIMIT 1")
        .fetch_optional(pool)
        .await
        .unwrap();
    if let Some(artifact) = artifact {
        let path = format!("/software-modules/{sm}");
        let page = get_html(&stack, &http, &cookie, &path).await;
        assert!(
            page.contains(&format!(
                "/software-modules/{sm}/artifacts/{artifact}/download"
            )),
            "the artifact has no download link"
        );
        assert!(
            !page.contains("/api/hawkbit/"),
            "downloads must not point at the raw mgmt proxy"
        );
    }

    // A distribution set says who carries it, counted by the query it links to.
    let page = get_html(&stack, &http, &cookie, &format!("/distribution-sets/{ds}")).await;
    for label in ["assigned", "installed"] {
        assert!(
            page.contains(&format!("/targets?q={label}distributionset%3D%3D{ds}"))
                || page.contains(&format!("{label}distributionset")),
            "the DS card does not link its '{label}' counter to the matching list"
        );
    }
}

/// Groups are a resource like any other, and a row leads to the devices it
/// counted — with the name quoted, so a group holding `,` or a trailing space
/// still links to a list whose total matches the number in the row.
#[tokio::test]
#[ignore]
async fn target_groups_are_an_ordinary_resource() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let pool = &stack.server_state.pool;
    // A name RSQL would mangle unquoted.
    for (n, group) in [(1, "eu,west"), (2, "eu,west"), (3, "us")] {
        sqlx::query(
            "INSERT INTO sp_target
             (created_at, created_by, last_modified_at, last_modified_by, optlock_revision,
              tenant, name, controller_id, sec_token, request_controller_attributes,
              update_status, target_group)
             VALUES (1, 'audit', 1, 'audit', 1, 'DEFAULT', $1, $1, $2, FALSE, 0, $3)",
        )
        .bind(format!("{NEEDLE}-grp-{n}"))
        .bind(format!("token-{NEEDLE}-grp-{n}"))
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }

    let http = browser();
    let cookie = login(&stack, &http).await;
    let page = get_html(&stack, &http, &cookie, "/target-groups").await;
    assert_no_error(&page, "/target-groups");
    assert!(page.contains("eu,west"), "the group list is empty");
    assert!(
        page.contains("class=\"pager\""),
        "a resource list always pages"
    );

    // Follow the row link the way a click would, and check the count agrees.
    let href = page
        .split("<tr data-href=\"")
        .find(|chunk| chunk.starts_with("/targets?"))
        .and_then(|chunk| chunk.split('"').next())
        .expect("a group row links to its devices")
        .replace("&amp;", "&");
    let listed = get_html(&stack, &http, &cookie, &href).await;
    assert_no_error(&listed, &href);
    assert!(
        listed.contains("1–2 of 2"),
        "the row said 2 devices, the list it links to must show 2: {href}"
    );
}

/// A link to a filtered list must be built in Rust, where the value gets
/// quoted and percent-encoded. Glued together in a template it looks right
/// until a name carries a comma, a slash or an ampersand — the group link on
/// the target card shipped exactly that way.
#[tokio::test]
#[ignore]
async fn filter_links_are_not_glued_in_templates() {
    for entry in std::fs::read_dir("templates")
        .expect("templates")
        .chain(std::fs::read_dir("templates/partials").expect("partials"))
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext != "html") {
            continue;
        }
        let markup = std::fs::read_to_string(&path).unwrap_or_default();
        for (index, line) in markup.lines().enumerate() {
            let Some(after) = line.split("?q=").nth(1) else {
                continue;
            };
            // `?q={{ something }}` is a ready-made value; `?q=name=={{ x }}`
            // is RSQL assembled here, with no quoting in sight.
            assert!(
                after.trim_start().starts_with("{{"),
                "{}:{}: an RSQL filter is being assembled in the template — \
                 build it with html::filtered_list_href instead: {line}",
                path.display(),
                index + 1
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The manual. Coverage is enforced mechanically: a field, column, view, bulk
// operation or status word that nobody documented fails the build.
// ---------------------------------------------------------------------------

/// Every `data-help="…"` on a page, in document order.
fn help_keys(page: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for piece in page.split("data-help=\"").skip(1) {
        if let Some(key) = piece.split('"').next() {
            keys.push(key.to_string());
        }
    }
    keys
}

/// Every chapter renders, including the ones nothing links to.
#[tokio::test]
#[ignore]
async fn help_topics_render() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let index = get_html(&stack, &http, &cookie, "/help").await;
    assert_no_error(&index, "/help");
    for topic in hawier_ui::help::manual().topics() {
        let path = format!("/help/{}", topic.id);
        let page = get_html(&stack, &http, &cookie, &path).await;
        assert_no_error(&page, &path);
        // One chapter is one document. Wrapping each section in its own card
        // turned a page into a stack of little documents once already.
        assert_eq!(
            page.matches("class=\"panel\"").count(),
            1,
            "{path} is split into cards instead of being one page"
        );
        assert!(
            index.contains(&format!("href=\"{path}\"")),
            "{path} is not listed in the table of contents"
        );
        for section in topic.sections.iter() {
            assert!(
                page.contains(&format!("id=\"{}\"", section.anchor)),
                "{path} does not anchor its section '{}'",
                section.anchor
            );
        }
    }
}

/// The load-bearing test: whatever an operator points at, on any page, the
/// manual has an answer for it. A renamed heading or a new control without a
/// key fails the build here.
#[tokio::test]
#[ignore]
async fn every_help_key_on_every_page_resolves() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let mut pages = every_page(&stack, &http, &cookie).await;
    for resource in resources::all().into_iter().filter(|r| r.create_verified) {
        let path = format!("/{}/new", resource.id);
        let page = get_html(&stack, &http, &cookie, &path).await;
        pages.push((path, page));
    }
    pages.push((
        "/help".to_string(),
        get_html(&stack, &http, &cookie, "/help").await,
    ));

    let mut broken = Vec::new();
    let mut seen = 0usize;
    for (path, page) in &pages {
        for key in help_keys(page) {
            seen += 1;
            if !hawier_ui::help::has(&key) && hawier_ui::help::topic(&key).is_none() {
                broken.push(format!("{path}: {key}"));
            }
        }
    }
    assert!(seen > 100, "only {seen} help keys on the whole console");
    broken.sort();
    broken.dedup();
    assert!(
        broken.is_empty(),
        "{} controls point at a section nobody wrote:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// A help-mode click can land anywhere, including on empty space, so every
/// page carries a key of its own as the last resort.
#[tokio::test]
#[ignore]
async fn every_page_answers_a_click_anywhere() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    for (path, page) in every_page(&stack, &http, &cookie).await {
        let body = page
            .split("<body")
            .nth(1)
            .and_then(|rest| rest.split('>').next())
            .unwrap_or_default()
            .to_string();
        let key = body
            .split("data-help=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_default();
        assert!(
            !key.is_empty(),
            "{path} has no page-level help key: a click on empty space answers nothing"
        );
        assert!(
            hawier_ui::help::has(key) || hawier_ui::help::topic(key).is_some(),
            "{path} answers with '{key}', which nobody wrote"
        );
        assert!(
            page.contains("data-help-panel"),
            "{path} has no panel for What's This? to open"
        );
        assert!(
            page.contains("data-help-toggle"),
            "{path} lost the help control in the top bar"
        );
    }
}

/// Colour means the same thing on every surface: a pill and a badge both carry
/// one of the three semantic classes, never the word itself.
#[tokio::test]
#[ignore]
async fn a_status_word_looks_the_same_everywhere() {
    let stack = spawn_stack().await;
    seed(&stack).await;
    let http = browser();
    let cookie = login(&stack, &http).await;

    let allowed = ["ok", "warn", "error", "neutral", "attention"];
    let mut wrong = Vec::new();
    let mut seen = 0usize;
    for (path, page) in every_page(&stack, &http, &cookie).await {
        for marker in ["status-pill", "status-badge"] {
            for piece in page.split(&format!("class=\"{marker}")).skip(1) {
                let classes = piece.split('"').next().unwrap_or_default().trim();
                seen += 1;
                for class in classes.split_whitespace() {
                    if !allowed.contains(&class) {
                        wrong.push(format!("{path}: {marker} {class}"));
                    }
                }
            }
        }
    }
    // Only a floor, so the test cannot pass by finding nothing: the seeded
    // fixture is small, and how many marks it produces is not the point.
    assert!(seen >= 10, "only {seen} status marks on the whole console");
    wrong.sort();
    wrong.dedup();
    assert!(
        wrong.is_empty(),
        "{} status marks colour by word instead of by meaning:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
